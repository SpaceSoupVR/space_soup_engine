//! Doors in the physics world: a leaf on a hinge joint, and the soft push of a
//! player's body against it. See [`crate::door`].

use std::collections::HashMap;

use glam::{Quat, Vec3};
use physx::prelude::*;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::door::{body_push_force, DoorDef, DoorGeom, BODY_PUSH_RADIUS, BODY_PUSH_SPAN};
use crate::rig::{JointId, PlayerRig};
use crate::rigid_physics::{from_px_transform, to_px_transform, to_raw_transform, DynamicActor, PhysicsWorld};
use crate::scene::GameObject;

/// The hinge's dry friction, N m: a well-hung door turns under about this.
pub const HINGE_DRY_FRICTION: f32 = 1.0;
/// A door closer's torque at closed, N m, ramped in over the last 2 degrees.
pub const CLOSER_PRELOAD: f32 = 2.0;
/// The tick the dry friction stops a leaf within. The server runs at 60 Hz.
const DOOR_TICK: f32 = 1.0 / 60.0;

fn mass_of(body: *mut physx_sys::PxRigidBody) -> f32 {
    unsafe { physx_sys::PxRigidBody_getMass(body) }
}

/// The joint frame's x is the twist axis; a door turns about its leaf's +y.
fn hinge_frame() -> Quat {
    Quat::from_rotation_arc(Vec3::X, Vec3::Y)
}

fn raw(pos: Vec3, rot: Quat) -> physx_sys::PxTransform {
    to_raw_transform(to_px_transform(pos, rot))
}

fn px(v: Vec3) -> physx_sys::PxVec3 {
    physx_sys::PxVec3 { x: v.x, y: v.y, z: v.z }
}

impl PhysicsWorld {
    /// A DOOR: the leaf as a dynamic box hung on the world by a D6 joint with
    /// every motion locked but the twist about its hinged edge -- a revolute
    /// joint -- limited to the door's range. The twist DRIVE is the hinge's
    /// friction (its damping, against any turning) and the door closer (its
    /// stiffness, toward closed): one constraint the solver handles with the
    /// limit, so neither can fight it.
    ///
    /// Sixteen position iterations and a capped depenetration speed: a hand
    /// is a kinematic sphere, and a hand pushed through a leaf pressed against
    /// its stop is a contact no solver can satisfy -- capped, the leaf is
    /// nudged at a walking pace instead of flung.
    pub(crate) fn spawn_door(&mut self, obj: &GameObject, def: &DoorDef) {
        let geom = DoorGeom::new(obj.cuboid.position, obj.cuboid.rotation, obj.cuboid.half_size, def);
        let start = def.start_deg.to_radians().clamp(geom.lo, geom.hi);
        let (pos, rot) = geom.pose_at(start);
        if !self.dynamic.contains_key(&obj.id) {
            if self.statics.contains_key(&obj.id) || self.kinematic.contains_key(&obj.id) {
                log::warn!("rigid_physics: door '{}' already has a non-dynamic collider", obj.id);
                return;
            }
            let Some(mut material) =
                self.foundation.create_material(def.surface_friction, def.surface_friction, 0.0, ())
            else {
                log::warn!("rigid_physics: failed to create a material for door '{}'", obj.id);
                return;
            };
            let h = geom.half;
            let geo = PxBoxGeometry::new(h.x, h.y, h.z);
            let Some(mut actor) = self.foundation.create_rigid_dynamic(
                to_px_transform(pos, rot),
                &geo,
                material.as_mut(),
                1.0,
                PxTransform::default(),
                (),
            ) else {
                log::warn!("rigid_physics: failed to create door '{}'", obj.id);
                return;
            };
            actor.set_mass(def.mass.max(0.5));
            actor.set_rigid_body_flag(RigidBodyFlag::EnableCcd, true);
            actor.set_solver_iteration_counts(16, 4);
            let ptr: *mut crate::rigid_physics::PxRigidDynamic = &mut *actor as *mut _;
            unsafe {
                let body = ptr as *mut physx_sys::PxRigidBody;
                physx_sys::PxRigidBody_setMaxDepenetrationVelocity_mut(body, 1.0);
                physx_sys::PxRigidBody_setMaxAngularVelocity_mut(body, 20.0);
                physx_sys::PxRigidBody_setAngularDamping_mut(body, 0.05);
                // Asleep only when truly still: a closer drawing the leaf
                // home slows to nothing at the top of each swing, and at the
                // default threshold the leaf fell asleep there, held open at
                // 12 degrees with the closer still pulling.
                physx_sys::PxRigidDynamic_setSleepThreshold_mut(ptr as *mut physx_sys::PxRigidDynamic, 1.0e-6);
            }
            self.scene.add_dynamic_actor(actor);
            self.materials.push(material);
            self.dynamic.insert(
                obj.id.clone(),
                DynamicActor { ptr, spawn_pos: pos, spawn_rot: rot, respawn_interval: None, elapsed: 0.0 },
            );
        }
        let Some(child) = self.dynamic.get(&obj.id) else { return };

        let frame = hinge_frame();
        let world_frame = raw(geom.hinge(), geom.closed_rot * frame);
        let leaf_frame = raw(geom.hinge_local, frame);
        let joint = unsafe {
            physx_sys::phys_PxD6JointCreate(
                self.foundation.as_mut_ptr(),
                std::ptr::null_mut(),
                &world_frame,
                child.ptr as *mut physx_sys::PxRigidActor,
                &leaf_frame,
            )
        };
        if joint.is_null() {
            log::warn!("rigid_physics: hinge creation failed for door '{}'", obj.id);
            return;
        }
        unsafe {
            use physx_sys::{PxD6Axis as A, PxD6Motion as M};
            for axis in [A::X, A::Y, A::Z, A::Swing1, A::Swing2] {
                physx_sys::PxD6Joint_setMotion_mut(joint, axis, M::Locked);
            }
            physx_sys::PxD6Joint_setMotion_mut(joint, A::Twist, M::Limited);
            let limit = physx_sys::PxJointAngularLimitPair_new(geom.lo, geom.hi.max(geom.lo + 1e-3), 0.0);
            physx_sys::PxD6Joint_setTwistLimit_mut(joint, &limit);
            // NO JOINT DRIVE: the closer and the hinge's friction are torques
            // `push_doors` applies. As the twist drive, a 15 N m/rad closer
            // with 10 N m s/rad of friction held a leaf a walking body leant
            // on with 80 N m at 15 degrees -- far stiffer than either -- so a
            // player could not walk through the hall's doors.

        }
        self.doors.insert(obj.id.clone(), geom);
        self.door_hinges.insert(obj.id.clone(), (def.closer.max(0.0), def.friction.max(0.0)));
    }

    /// THE PLAYERS' BODIES PUSH THE DOORS: each body a vertical capsule under
    /// its head (`door::BODY_PUSH_SPAN`, `BODY_PUSH_RADIUS`), pushing a leaf
    /// it reaches into by a spring on the depth, damped by the leaf's speed
    /// toward it (`door::body_push_force`), as a torque about the hinge.
    ///
    /// Soft rather than a kinematic capsule in the scene: the player is not a
    /// physics body -- they stop at a leaf by the walk's ray -- and a hard
    /// capsule pressed into a leaf at its stop is an unsatisfiable contact
    /// that jitters. A spring against a stop is a steady torque the limit
    /// simply holds.
    pub(crate) fn push_doors(&mut self, rigs: &HashMap<PlayerId, PlayerRig>) {
        for (id, geom) in &self.doors {
            let Some(state) = self.dynamic.get(id) else { continue };
            let body = state.ptr as *mut physx_sys::PxRigidBody;
            let (_, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
            let angle = geom.angle_of(rot);
            let axis = geom.axis();
            let hinge = geom.hinge();
            let mut torque = 0.0f32;
            for rig in rigs.values() {
                let Some(head) = rig.get(JointId::Head) else { continue };
                let head = head.position;
                let span = (head.y - BODY_PUSH_SPAN.0, head.y - BODY_PUSH_SPAN.1);
                let Some((point, n, depth)) = geom.body_contact(angle, head, span, BODY_PUSH_RADIUS) else {
                    continue;
                };
                let w = unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) };
                let w = Vec3::new(w.x, w.y, w.z);
                let arm = point - hinge;
                let closing = w.cross(arm).dot(n);
                let force = -n * body_push_force(depth, closing);
                torque += arm.cross(force).dot(axis);
            }
            // THE HINGE'S DRY FRICTION: a constant torque against any
            // turning, which stops a coasting leaf in finite time where the
            // drive's viscous friction alone only slows it -- a door that has
            // not quite stopped after six seconds reads as a door on ice. Up
            // to what stops the leaf this tick, so it never turns it back.
            let w = unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) };
            let spin = Vec3::new(w.x, w.y, w.z).dot(axis);
            let inertia = geom.hinge_inertia(mass_of(body));
            let (closer, friction) = self.door_hinges.get(id).copied().unwrap_or((0.0, 0.0));
            // Viscous friction, then the dry friction up to what is left of
            // the leaf's turning this tick.
            let viscous = (friction * spin).clamp(-inertia * spin.abs() / DOOR_TICK, inertia * spin.abs() / DOOR_TICK);
            torque -= viscous;
            let left = inertia * spin / DOOR_TICK - viscous;
            torque -= left.clamp(-HINGE_DRY_FRICTION, HINGE_DRY_FRICTION);
            // The closer: a spring toward closed, with the preload real ones
            // hold at closed, so the last degrees are drawn home through the
            // dry friction.
            if closer > 0.0 {
                torque -= closer * angle + CLOSER_PRELOAD * (angle / 2f32.to_radians()).clamp(-1.0, 1.0);
            }
            // INTO A STOP, NOTHING: the limit would only take the torque back
            // the next iteration, and the leaf rattled against it by a tenth
            // of a degree a tick while a body leant on it.
            const AT_STOP: f32 = 0.5 * std::f32::consts::PI / 180.0;
            if (angle <= geom.lo + AT_STOP && torque < 0.0) || (angle >= geom.hi - AT_STOP && torque > 0.0) {
                torque = 0.0;
            }
            if torque != 0.0 {
                let t = px(axis * torque);
                unsafe { physx_sys::PxRigidBody_addTorque_mut(body, &t, physx_sys::PxForceMode::Force, true) };
            }
        }
    }

    /// Put a door's leaf where the server says it is: the client's own world,
    /// which is never stepped, collides the walk and casts the torch's rays
    /// against it. Ignores anything that is not a door.
    pub fn set_door_pose(&mut self, id: &str, position: Vec3, rotation: Quat) {
        if !self.doors.contains_key(id) {
            return;
        }
        if let Some(state) = self.dynamic.get(id) {
            let pose = to_px_transform(position, rotation);
            unsafe { (*state.ptr).set_global_pose(&pose, false) };
        }
    }

    /// The doors this world hangs, by object id.
    pub fn doors(&self) -> impl Iterator<Item = (&str, &DoorGeom)> {
        self.doors.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// How far a door stands open, radians from closed.
    pub fn door_angle(&self, id: &str) -> Option<f32> {
        let geom = self.doors.get(id)?;
        let state = self.dynamic.get(id)?;
        let (_, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
        Some(geom.angle_of(rot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rig::Transform;
    use crate::scene::{CuboidDef, Scene};

    const DT: f32 = 1.0 / 60.0;

    /// One PhysX foundation per process: see `PHYSX_TEST_LOCK`.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::runtime_test_support::PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A one-way door 80 cm wide in the xy plane, hinged on its -x edge at
    /// x = -0.4, opening toward -z (positive angles) to 100 degrees.
    fn door(def: DoorDef) -> (PhysicsWorld, Scene) {
        let mut scene = Scene::default();
        scene.objects.push(GameObject {
            id: "door".into(),
            cuboid: CuboidDef {
                position: Vec3::new(0.0, 1.1, 0.0),
                half_size: Vec3::new(0.4, 1.05, 0.02),
                ..Default::default()
            },
            door: Some(def),
            ..Default::default()
        });
        let mut physics = PhysicsWorld::new();
        physics.rebuild(&scene, std::path::Path::new("."));
        (physics, scene)
    }

    fn rig_at(head: Vec3, hand: Option<Vec3>) -> HashMap<PlayerId, PlayerRig> {
        let mut rig = PlayerRig::new();
        rig.joints.insert(JointId::Head, Transform { position: head, ..Default::default() });
        for h in [crate::events::Hand::Left, crate::events::Hand::Right] {
            // Hands out of the way unless asked for: straight below the
            // origin, where the anchors start, so they sweep through nothing.
            // (A kinematic teleported across a leaf hits it at the speed of
            // the teleport -- 3 km/s from the origin to a far corner.)
            let p = hand.unwrap_or(Vec3::new(0.0, -20.0, 0.0));
            rig.joints.insert(JointId::HandGrip(h), Transform { position: p, ..Default::default() });
        }
        let player = PlayerId::new();
        HashMap::from([(player, rig)])
    }

    fn run(physics: &mut PhysicsWorld, scene: &mut Scene, rigs: &HashMap<PlayerId, PlayerRig>, seconds: f32) -> Vec<f32> {
        for p in rigs.keys() {
            physics.ensure_player(*p);
        }
        let mut angles = Vec::new();
        for _ in 0..(seconds / DT) as usize {
            physics.step(DT, scene, rigs);
            angles.push(physics.door_angle("door").unwrap());
        }
        angles
    }

    #[test]
    fn a_door_hangs_still_and_closed() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef::default());
        let angles = run(&mut physics, &mut scene, &HashMap::new(), 2.0);
        assert!(angles.iter().all(|a| a.abs() < 1e-3), "a closed door with nothing on it moves: {:?}", &angles[..5]);
        // And its pose reaches the scene.
        let o = scene.find_object("door").unwrap();
        assert!((o.cuboid.position - Vec3::new(0.0, 1.1, 0.0)).length() < 1e-3);
    }

    #[test]
    fn walking_into_a_door_swings_it_open_and_it_settles_within_its_limits() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef::default());
        // A head 25 cm in front of the leaf's +z face, 20 cm out from its
        // middle -- where the walk's ray stops a player -- pushing it toward -z.
        let rigs = rig_at(Vec3::new(0.1, 1.7, 0.27), None);
        let pushed = run(&mut physics, &mut scene, &rigs, 1.0);
        let a = *pushed.last().unwrap();
        assert!(a > 10f32.to_radians(), "a second of pushing opened it only {} deg", a.to_degrees());
        // Then the body leaves; the door coasts, the hinge's friction stops it.
        let coasting = run(&mut physics, &mut scene, &HashMap::new(), 6.0);
        let hi = 100f32.to_radians();
        assert!(coasting.iter().all(|a| *a >= -0.01 && *a <= hi + 0.02), "past its limits");
        let tail = &coasting[coasting.len() - 30..];
        let spread = tail.iter().cloned().fold(f32::MIN, f32::max) - tail.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread < 1e-3, "still moving after 6 s: {spread}");
    }

    #[test]
    fn a_body_pressing_a_door_against_its_stop_holds_it_there_without_jitter() {
        let _guard = lock();
        // A one-way door pushed from its stop side: the push turns it toward
        // negative angles, which the limit forbids.
        let (mut physics, mut scene) = door(DoorDef::default());
        let rigs = rig_at(Vec3::new(0.1, 1.7, -0.27), None);
        let angles = run(&mut physics, &mut scene, &rigs, 3.0);
        let worst = angles.iter().map(|a| a.abs()).fold(0.0, f32::max);
        assert!(worst < 1.5f32.to_radians(), "pressed against its stop it moved {} deg", worst.to_degrees());
        // Held: over the last second no tick moves it a fiftieth of a degree
        // (a third of a millimetre at the latch edge).
        let tail = &angles[angles.len() - 60..];
        let step = tail.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        eprintln!("pressed against its stop: worst {:.3} deg, largest tick {:.4} deg", worst.to_degrees(), step.to_degrees());
        assert!(step < 0.02f32.to_radians(), "it jitters against its stop: {} deg a tick", step.to_degrees());
    }

    #[test]
    fn a_hand_knocking_the_door_hard_swings_it_and_the_stop_holds() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef { friction: 1.0, ..Default::default() });
        // The hand sweeps through the leaf toward -z at 3 m/s.
        let mut rigs = rig_at(Vec3::new(2.0, 1.7, 2.0), Some(Vec3::new(0.2, 1.2, 0.15)));
        let player = *rigs.keys().next().unwrap();
        // The anchors start at the origin, under the leaf's corner: their
        // first tick is a jump to the hand, which must not sweep the door.
        physics.ensure_player(player);
        let mut angles = Vec::new();
        for k in 0..240 {
            // In through the leaf's face at 3 m/s for four frames, then back
            // out the way it came at the same speed.
            let t = (k as f32).min(8.0 - k as f32).max(-30.0);
            let hand = Vec3::new(0.2, 1.2, 0.15 - 3.0 * DT * t);
            for h in [crate::events::Hand::Left, crate::events::Hand::Right] {
                rigs.get_mut(&player).unwrap().joints.insert(JointId::HandGrip(h), Transform { position: hand, ..Default::default() });
            }
            physics.step(DT, &mut scene, &rigs);
            angles.push(physics.door_angle("door").unwrap());
        }
        let most = angles.iter().cloned().fold(f32::MIN, f32::max);
        assert!(most > 40f32.to_radians(), "a hard knock swung it only {} deg", most.to_degrees());
        assert!(most < 100f32.to_radians() + 0.05, "past its stop: {} deg", most.to_degrees());
        assert!(angles.iter().all(|a| a.is_finite()), "it exploded");
    }

    #[test]
    fn a_door_closer_brings_it_back_and_a_two_way_door_swings_both_ways() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef { open_min_deg: -100.0, closer: 20.0, friction: 6.0, start_deg: 60.0, ..Default::default() });
        assert!((physics.door_angle("door").unwrap() - 60f32.to_radians()).abs() < 1e-3, "it starts where authored");
        let angles = run(&mut physics, &mut scene, &HashMap::new(), 6.0);
        let end = *angles.last().unwrap();
        assert!(end.abs() < 2f32.to_radians(), "the closer left it at {} deg", end.to_degrees());
        // Pushed from the -z side, a two-way door opens the other way.
        let rigs = rig_at(Vec3::new(0.1, 1.7, -0.27), None);
        let pushed = run(&mut physics, &mut scene, &rigs, 1.0);
        assert!(*pushed.last().unwrap() < -5f32.to_radians(), "{}", pushed.last().unwrap().to_degrees());
    }

    #[test]
    fn the_client_world_puts_a_door_where_the_server_says_and_rays_meet_it_there() {
        let _guard = lock();
        let (mut physics, _) = door(DoorDef::default());
        // Closed: a ray along -z through the doorway meets the leaf.
        assert!(physics.raycast(Vec3::new(0.0, 1.0, 1.0), Vec3::NEG_Z, 2.0).is_some());
        let geom = *physics.doors().next().unwrap().1;
        let (p, r) = geom.pose_at(95f32.to_radians());
        physics.set_door_pose("door", p, r);
        assert!(physics.raycast(Vec3::new(0.0, 1.0, 1.0), Vec3::NEG_Z, 2.0).is_none(), "the open doorway is clear");
        assert!((physics.door_angle("door").unwrap() - 95f32.to_radians()).abs() < 1e-3);
    }

    fn shipped() -> (PhysicsWorld, Scene) {
        let game = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
        let scene = Scene::load(&game.join("scenes/test_room.json")).expect("test_room.json loads");
        let mut physics = PhysicsWorld::new();
        physics.rebuild(&scene, game);
        (physics, scene)
    }

    /// The shipped doors hang in their doorways touching nothing: two
    /// seconds of simulation leaves every leaf closed. A leaf overlapping a
    /// jamb, the floor or its partner would be pushed out at once.
    #[test]
    fn the_hallways_doors_hang_closed_in_their_doorways() {
        let _guard = lock();
        let (mut physics, mut scene) = shipped();
        let ids: Vec<String> = physics.doors().map(|(id, _)| id.to_string()).collect();
        assert_eq!(ids.len(), 4, "test_room hangs two pairs of doors: {ids:?}");
        for _ in 0..120 {
            physics.step(DT, &mut scene, &HashMap::new());
        }
        for id in &ids {
            let a = physics.door_angle(id).unwrap();
            assert!(a.abs() < 0.2f32.to_radians(), "{id} drifted to {} deg", a.to_degrees());
        }
    }

    /// END TO END: a player walking the hallway from the marble hall to the
    /// brick hall at walking pace, stopped by the leaves as the client's walk
    /// stops them (`Locomotion::apply_collision`), pushes both pairs open
    /// with their body and comes out the far side -- and the closers bring the
    /// first pair home behind them.
    #[test]
    fn walking_down_the_hallway_pushes_both_pairs_of_doors_open() {
        use crate::locomotion::{Locomotion, LocomotionMode};
        let _guard = lock();
        let (mut physics, mut scene) = shipped();
        let mut loco = Locomotion::new(LocomotionMode::Smooth);
        // Off the leaves' meeting gap, into the south leaves.
        loco.player_offset = Vec3::new(1.5, 0.0, -3.25);
        let mut rigs = rig_at(Vec3::new(1.5, 1.7, -3.25), None);
        let player = *rigs.keys().next().unwrap();
        physics.ensure_player(player);
        let speed = 1.4;
        let mut opened = [0.0f32; 2];
        let mut seconds = 0.0;
        while seconds < 20.0 && loco.player_offset.x < 12.0 {
            let start = loco.collision_start(Some(loco.player_offset + Vec3::Y * 1.7));
            loco.player_offset.x += speed * DT;
            loco.apply_collision(&physics, start);
            let head = loco.player_offset + Vec3::Y * 1.7;
            rigs.get_mut(&player).unwrap().joints.insert(JointId::Head, Transform { position: head, ..Default::default() });
            physics.step(DT, &mut scene, &rigs);
            opened[0] = opened[0].max(physics.door_angle("hall_door_south").unwrap().abs());
            opened[1] = opened[1].max(physics.door_angle("brick_door_south").unwrap().abs());
            seconds += DT;
        }
        eprintln!(
            "through in {seconds:.1} s; hall door opened {:.0} deg, brick door {:.0} deg",
            opened[0].to_degrees(),
            opened[1].to_degrees()
        );
        assert!(loco.player_offset.x >= 12.0, "stuck at {:?} after {seconds:.1} s", loco.player_offset);
        // A clear walk is 7.5 s; pushing two doors may slow it, not stop it.
        assert!(seconds < 12.0, "took {seconds:.1} s");
        assert!(opened[0] > 30f32.to_radians() && opened[1] > 30f32.to_radians(), "{opened:?}");
        // Five seconds on, the hall's closers have drawn their pair shut.
        for _ in 0..300 {
            physics.step(DT, &mut scene, &HashMap::new());
        }
        let back = physics.door_angle("hall_door_south").unwrap();
        assert!(crate::door::DoorGeom::shut(back), "the closer left the hall door at {} deg", back.to_degrees());
    }
}
