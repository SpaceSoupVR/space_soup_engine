use std::collections::HashMap;

use glam::{Quat, Vec3};
use physx::prelude::*;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::door::{body_push_force, DoorDef, DoorGeom, DoorLatch, BODY_PUSH_RADIUS, BODY_PUSH_SPAN, LATCH_PLAY_DEG};
use crate::rig::{JointId, PlayerRig};
use crate::rigid_physics::{from_px_transform, to_px_transform, to_raw_transform, DynamicActor, PhysicsWorld};
use crate::scene::GameObject;

pub const HINGE_DRY_FRICTION: f32 = 1.0;

pub const CLOSER_PRELOAD: f32 = 2.0;

const DOOR_TICK: f32 = 1.0 / 60.0;

/// A latching door's handle and bolt in the physics world: whether the bolt
/// is home, and the hinge whose limits it narrows while it is.
pub(crate) struct LatchState {
    pub(crate) latch: DoorLatch,
    pub(crate) bolted: bool,
    pub(crate) joint: *mut physx_sys::PxD6Joint,
}

fn mass_of(body: *mut physx_sys::PxRigidBody) -> f32 {
    unsafe { physx_sys::PxRigidBody_getMass(body) }
}

fn hinge_frame() -> Quat {
    Quat::from_rotation_arc(Vec3::X, Vec3::Y)
}

fn raw(pos: Vec3, rot: Quat) -> physx_sys::PxTransform {
    to_raw_transform(to_px_transform(pos, rot))
}

fn px(v: Vec3) -> physx_sys::PxVec3 {
    physx_sys::PxVec3 { x: v.x, y: v.y, z: v.z }
}

/// Every hand of every player, world: the grips, which is where a hand
/// reaching for a handle is.
fn hands_of(rigs: &HashMap<PlayerId, PlayerRig>) -> impl Iterator<Item = Vec3> + '_ {
    rigs.values().flat_map(|rig| {
        [crate::events::Hand::Left, crate::events::Hand::Right].into_iter().filter_map(|h| rig.get(JointId::HandGrip(h)).map(|t| t.position))
    })
}

impl PhysicsWorld {

    pub(crate) fn spawn_door(&mut self, entity: &GameObject, def: &DoorDef) {
        let geom = DoorGeom::new(entity.cuboid.position, entity.cuboid.rotation, entity.cuboid.half_size, def);
        let start = def.start_deg.to_radians().clamp(geom.lo, geom.hi);
        let (pos, rot) = geom.pose_at(start);
        if geom.fixed() {
            if self.statics.contains_key(&entity.id) || self.dynamic.contains_key(&entity.id) || self.kinematic.contains_key(&entity.id) {
                log::warn!("rigid_physics: door '{}' already has a collider", entity.id);
                return;
            }
            let h = geom.half;
            let (Some(mut material), geo) =
                (self.foundation.create_material(def.surface_friction, def.surface_friction, 0.0, ()), PxBoxGeometry::new(h.x, h.y, h.z))
            else {
                return;
            };
            if let Some(mut actor) =
                self.foundation.create_rigid_static(to_px_transform(pos, rot), &geo, material.as_mut(), PxTransform::default(), ())
            {
                let ptr: *mut crate::rigid_physics::PxRigidStatic = &raw mut *actor;
                self.scene.add_static_actor(actor);
                self.statics.insert(entity.id.clone(), ptr);
                self.materials.push(material);
                self.note_floor(pos.y - crate::water::world_half_extents(h, rot).y);
                self.doors.insert(entity.id.clone(), geom);
            }
            return;
        }
        if !self.dynamic.contains_key(&entity.id) {
            if self.statics.contains_key(&entity.id) || self.kinematic.contains_key(&entity.id) {
                log::warn!("rigid_physics: door '{}' already has a non-dynamic collider", entity.id);
                return;
            }
            let Some(mut material) =
                self.foundation.create_material(def.surface_friction, def.surface_friction, 0.0, ())
            else {
                log::warn!("rigid_physics: failed to create a material for door '{}'", entity.id);
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
                log::warn!("rigid_physics: failed to create door '{}'", entity.id);
                return;
            };
            actor.set_mass(def.mass.max(0.5));
            unsafe {
                physx_sys::PxRigidBodyExt_setMassAndUpdateInertia_1(
                    (&raw mut *actor).cast::<physx_sys::PxRigidBody>(),
                    def.mass.max(0.5),
                    std::ptr::null(),
                    false,
                );
            }
            actor.set_rigid_body_flag(RigidBodyFlag::EnableCcd, true);
            actor.set_solver_iteration_counts(16, 4);
            let ptr: *mut crate::rigid_physics::PxRigidDynamic = &raw mut *actor;
            unsafe {
                let body = ptr.cast::<physx_sys::PxRigidBody>();
                physx_sys::PxRigidBody_setMaxDepenetrationVelocity_mut(body, 1.0);
                physx_sys::PxRigidBody_setMaxAngularVelocity_mut(body, 20.0);
                physx_sys::PxRigidBody_setAngularDamping_mut(body, 0.05);

                physx_sys::PxRigidDynamic_setSleepThreshold_mut(ptr.cast::<physx_sys::PxRigidDynamic>(), 1.0e-6);
            }
            self.scene.add_dynamic_actor(actor);
            self.materials.push(material);
            self.dynamic.insert(entity.id.clone(), DynamicActor::new(ptr, pos, rot, h, 8.0 * h.element_product()));
            self.note_floor(pos.y - crate::water::world_half_extents(h, rot).y);
        }
        let Some(child) = self.dynamic.get(&entity.id) else { return };

        let frame = hinge_frame();
        let world_frame = raw(geom.hinge(), geom.closed_rot * frame);
        let leaf_frame = raw(geom.hinge_local, frame);
        let joint = unsafe {
            physx_sys::phys_PxD6JointCreate(
                self.foundation.as_mut_ptr(),
                std::ptr::null_mut(),
                &raw const world_frame,
                child.ptr.cast::<physx_sys::PxRigidActor>(),
                &raw const leaf_frame,
            )
        };
        if joint.is_null() {
            log::warn!("rigid_physics: hinge creation failed for door '{}'", entity.id);
            return;
        }
        unsafe {
            use physx_sys::{PxD6Axis as A, PxD6Motion as M};
            for axis in [A::X, A::Y, A::Z, A::Swing1, A::Swing2] {
                physx_sys::PxD6Joint_setMotion_mut(joint, axis, M::Locked);
            }
            physx_sys::PxD6Joint_setMotion_mut(joint, A::Twist, M::Limited);
            let limit = physx_sys::PxJointAngularLimitPair_new(geom.lo, geom.hi.max(geom.lo + 1e-3), 0.0);
            physx_sys::PxD6Joint_setTwistLimit_mut(joint, &raw const limit);

        }
        self.doors.insert(entity.id.clone(), geom);
        self.door_hinges.insert(entity.id.clone(), (def.closer.max(0.0), def.friction.max(0.0)));
        if let Some(latch) = DoorLatch::of(def) {
            self.door_latches.insert(entity.id.clone(), LatchState { latch, bolted: false, joint });
        }
    }

    /// Whether a latching door's bolt is home. None for a door with no latch.
    #[must_use]
    pub fn door_bolted(&self, id: &str) -> Option<bool> {
        self.door_latches.get(id).filter(|l| l.latch.bolts).map(|l| l.bolted)
    }

    /// Whether a hand is at a door's handle, or holding it: the lever is
    /// down. None for a door with no handle.
    #[must_use]
    pub fn door_handle_worked(&self, id: &str, rigs: &HashMap<PlayerId, PlayerRig>) -> Option<bool> {
        let latch = self.door_latches.get(id)?;
        let (geom, state) = (self.doors.get(id)?, self.dynamic.get(id)?);
        let pose = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
        Some(self.is_held(id) || latch.latch.worked(pose, geom.half.z, hands_of(rigs)))
    }

    pub(crate) fn push_doors(&mut self, rigs: &HashMap<PlayerId, PlayerRig>) {
        for (id, geom) in &self.doors {
            let Some(state) = self.dynamic.get(id) else { continue };
            let body = state.ptr.cast::<physx_sys::PxRigidBody>();
            let (pos, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
            let angle = geom.angle_of(rot);
            let mut bolted = false;
            if let Some(latch) = self.door_latches.get_mut(id) {
                let held = self.grabs.keys().any(|(_, held, _)| held == id);
                let worked = held || latch.latch.worked((pos, rot), geom.half.z, hands_of(rigs));
                let w = unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) };
                let spin = Vec3::new(w.x, w.y, w.z).dot(geom.axis());
                let now = latch.latch.bolt(latch.bolted, angle, spin, worked);
                if now != latch.bolted {
                    let play = LATCH_PLAY_DEG.to_radians();
                    let (lo, hi) = if now { (-play, play) } else { (geom.lo, geom.hi.max(geom.lo + 1e-3)) };
                    unsafe {
                        let limit = physx_sys::PxJointAngularLimitPair_new(lo, hi, 0.0);
                        physx_sys::PxD6Joint_setTwistLimit_mut(latch.joint, &raw const limit);
                        physx_sys::PxRigidDynamic_wakeUp_mut(state.ptr.cast::<physx_sys::PxRigidDynamic>());
                    }
                    latch.bolted = now;
                }
                bolted = now;
            }
            if bolted {
                continue;
            }
            {
                let w = unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) };
                let w = Vec3::new(w.x, w.y, w.z);
                let spin = w.dot(geom.axis());
                let allowed = crate::door::soft_stop(angle, spin, geom.lo, geom.hi);
                if allowed != spin {
                    let turning = w + geom.axis() * (allowed - spin);
                    let moving = turning.cross(pos - geom.hinge());
                    let dynamic = state.ptr.cast::<physx_sys::PxRigidDynamic>();
                    unsafe {
                        physx_sys::PxRigidDynamic_setAngularVelocity_mut(dynamic, &px(turning), false);
                        physx_sys::PxRigidDynamic_setLinearVelocity_mut(dynamic, &px(moving), false);
                    }
                }
            }
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
            for ((player, held, hand), grab) in &self.grabs {
                let (true, Some(local)) = (held == id, grab.local) else { continue };
                let (Some(rig), Some(&anchor)) = (rigs.get(player), self.hand_anchors.get(&(*player, *hand))) else { continue };
                if anchor.is_null() {
                    continue;
                }
                let to = rig.hand_grip(*hand).position;
                let (from, _) = from_px_transform(&unsafe { (*anchor).get_global_pose() });
                let point = pos + rot * local;
                let arm = point - hinge;
                let w = unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) };
                let moving = Vec3::new(w.x, w.y, w.z).cross(arm) - (to - from) / DOOR_TICK;
                torque += arm.cross(crate::door::door_pull_force(to - point, moving)).dot(axis);
            }
            let w = unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) };
            let spin = Vec3::new(w.x, w.y, w.z).dot(axis);
            let inertia = geom.hinge_inertia(mass_of(body));
            let (closer, friction) = self.door_hinges.get(id).copied().unwrap_or((0.0, 0.0));

            let viscous = (friction * spin).clamp(-inertia * spin.abs() / DOOR_TICK, inertia * spin.abs() / DOOR_TICK);
            torque -= viscous;
            let left = inertia * spin / DOOR_TICK - viscous;
            torque -= left.clamp(-HINGE_DRY_FRICTION, HINGE_DRY_FRICTION);

            if closer > 0.0 {
                torque -= closer * angle + CLOSER_PRELOAD * (angle / 2f32.to_radians()).clamp(-1.0, 1.0);
            }

            const AT_STOP: f32 = 0.5 * std::f32::consts::PI / 180.0;
            if (angle <= geom.lo + AT_STOP && torque < 0.0) || (angle >= geom.hi - AT_STOP && torque > 0.0) {
                torque = 0.0;
            }
            if torque != 0.0 {
                let t = px(axis * torque);
                unsafe { physx_sys::PxRigidBody_addTorque_mut(body, &raw const t, physx_sys::PxForceMode::Force, true) };
            }
        }
    }

    pub fn set_door_pose(&mut self, id: &str, position: Vec3, rotation: Quat) {
        if !self.doors.contains_key(id) {
            return;
        }
        if let Some(state) = self.dynamic.get(id) {
            let pose = to_px_transform(position, rotation);
            unsafe { (*state.ptr).set_global_pose(&pose, false) };
        }
    }

    pub fn doors(&self) -> impl Iterator<Item = (&str, &DoorGeom)> {
        self.doors.iter().map(|(k, v)| (k.as_str(), v))
    }

    #[must_use]
    pub fn door_angle(&self, id: &str) -> Option<f32> {
        let geom = self.doors.get(id)?;
        if geom.fixed() {
            return Some(geom.lo);
        }
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

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::runtime_test_support::PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

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

        let o = scene.find_object("door").unwrap();
        assert!((o.cuboid.position - Vec3::new(0.0, 1.1, 0.0)).length() < 1e-3);
    }

    #[test]
    fn walking_into_a_door_swings_it_open_and_it_settles_within_its_limits() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef::default());

        let rigs = rig_at(Vec3::new(0.1, 1.7, 0.27), None);
        let pushed = run(&mut physics, &mut scene, &rigs, 1.0);
        let a = *pushed.last().unwrap();
        assert!(a > 10f32.to_radians(), "a second of pushing opened it only {} deg", a.to_degrees());

        let coasting = run(&mut physics, &mut scene, &HashMap::new(), 6.0);
        let hi = 100f32.to_radians();
        assert!(coasting.iter().all(|a| *a >= -0.01 && *a <= hi + 0.02), "past its limits");
        let tail = &coasting[coasting.len() - 30..];
        let spread = tail.iter().copied().fold(f32::MIN, f32::max) - tail.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread < 1e-3, "still moving after 6 s: {spread}");
    }

    #[test]
    fn a_body_pressing_a_door_against_its_stop_holds_it_there_without_jitter() {
        let _guard = lock();

        let (mut physics, mut scene) = door(DoorDef::default());
        let rigs = rig_at(Vec3::new(0.1, 1.7, -0.27), None);
        let angles = run(&mut physics, &mut scene, &rigs, 3.0);
        let worst = angles.iter().map(|a| a.abs()).fold(0.0, f32::max);
        assert!(worst < 1.5f32.to_radians(), "pressed against its stop it moved {} deg", worst.to_degrees());

        let tail = &angles[angles.len() - 60..];
        let step = tail.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        eprintln!("pressed against its stop: worst {:.3} deg, largest tick {:.4} deg", worst.to_degrees(), step.to_degrees());
        assert!(step < 0.02f32.to_radians(), "it jitters against its stop: {} deg a tick", step.to_degrees());
    }

    #[test]
    fn a_hand_knocking_the_door_hard_swings_it_and_the_stop_holds() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef { friction: 1.0, ..Default::default() });

        let mut rigs = rig_at(Vec3::new(2.0, 1.7, 2.0), Some(Vec3::new(0.2, 1.2, 0.15)));
        let player = *rigs.keys().next().unwrap();

        physics.ensure_player(player);
        let mut angles = Vec::new();
        for k in 0..240 {

            let t = (k as f32).min(8.0 - k as f32).max(-30.0);
            let hand = Vec3::new(0.2, 1.2, 0.15 - 3.0 * DT * t);
            for h in [crate::events::Hand::Left, crate::events::Hand::Right] {
                rigs.get_mut(&player).unwrap().joints.insert(JointId::HandGrip(h), Transform { position: hand, ..Default::default() });
            }
            physics.step(DT, &mut scene, &rigs);
            angles.push(physics.door_angle("door").unwrap());
        }
        let most = angles.iter().copied().fold(f32::MIN, f32::max);
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

        let rigs = rig_at(Vec3::new(0.1, 1.7, -0.27), None);
        let pushed = run(&mut physics, &mut scene, &rigs, 1.0);
        assert!(*pushed.last().unwrap() < -5f32.to_radians(), "{}", pushed.last().unwrap().to_degrees());
    }

    #[test]
    fn the_client_world_puts_a_door_where_the_server_says_and_rays_meet_it_there() {
        let _guard = lock();
        let (mut physics, _) = door(DoorDef::default());

        assert!(physics.raycast(Vec3::new(0.0, 1.0, 1.0), Vec3::NEG_Z, 2.0).is_some());
        let geom = *physics.doors().next().unwrap().1;
        let (p, r) = geom.pose_at(95f32.to_radians());
        physics.set_door_pose("door", p, r);
        assert!(physics.raycast(Vec3::new(0.0, 1.0, 1.0), Vec3::NEG_Z, 2.0).is_none(), "the open doorway is clear");
        assert!((physics.door_angle("door").unwrap() - 95f32.to_radians()).abs() < 1e-3);
    }

    /// A latching door: the test leaf with a lever 31 cm out from its middle
    /// (9 cm in from the free edge), a metre off the floor.
    fn latching() -> DoorDef {
        DoorDef { latch: true, handle: Some([0.31, -0.1, 0.0]), ..Default::default() }
    }

    /// The handle's hold point on the leaf's +z face at `angle`, world.
    fn handle_at(angle: f32) -> Vec3 {
        let geom = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, Vec3::new(0.4, 1.05, 0.02), &latching());
        let (c, r) = geom.pose_at(angle);
        c + r * Vec3::new(0.31, -0.1, 0.02 + crate::door::HANDLE_STANDOFF)
    }

    /// A LATCH HOLDS THE DOOR SHUT against a body leaning on it, with nothing
    /// moving; a hand at the handle draws the bolt and the same lean opens
    /// the door; shut again with no hand there, the bolt goes home.
    #[test]
    fn a_latched_door_holds_against_a_body_and_opens_with_a_hand_at_the_handle() {
        let _guard = lock();
        let (mut physics, mut scene) = door(latching());
        let lean = Vec3::new(0.1, 1.7, 0.27);
        let rigs = rig_at(lean, Some(Vec3::new(0.3, 0.8, 0.6)));
        let pushed = run(&mut physics, &mut scene, &rigs, 3.0);
        assert_eq!(physics.door_bolted("door"), Some(true));
        let worst = pushed.iter().map(|a| a.abs()).fold(0.0, f32::max);
        assert!(worst <= (LATCH_PLAY_DEG + 0.1).to_radians(), "a latched door gave {} deg to a lean", worst.to_degrees());
        let tail = &pushed[pushed.len() - 60..];
        let step = tail.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(step < 0.02f32.to_radians(), "it rattles in its latch: {} deg a tick", step.to_degrees());
        let rigs = rig_at(lean, Some(handle_at(0.0) + Vec3::new(0.0, 0.0, 0.03)));
        assert_eq!(physics.door_handle_worked("door", &rigs), Some(true));
        let opened = run(&mut physics, &mut scene, &rigs, 1.0);
        assert_eq!(physics.door_bolted("door"), Some(false));
        assert!(*opened.last().unwrap() > 8f32.to_radians(), "bolt drawn, a second's lean opened it {} deg", opened.last().unwrap().to_degrees());
    }

    /// BY ITS HANDLE: a hand that takes the lever and walks its arc swings
    /// the door after it, with the leaf's weight -- behind a quick hand, up
    /// with it when it stops. A hand that goes on past the stop does not
    /// rattle the door against it. Brought back and let go, the bolt goes
    /// home, and the door is shut and held.
    #[test]
    fn a_hand_on_the_handle_swings_the_door_and_the_latch_catches_it_shut() {
        let _guard = lock();
        let (mut physics, mut scene) = door(latching());
        let mut rigs = rig_at(Vec3::new(2.0, 1.7, 2.0), Some(handle_at(0.0)));
        let player = *rigs.keys().next().unwrap();
        let hold = |rigs: &mut HashMap<PlayerId, PlayerRig>, p: Vec3| {
            rigs.get_mut(&player).unwrap().joints.insert(JointId::HandGrip(crate::events::Hand::Right), Transform { position: p, ..Default::default() });
        };
        rigs.get_mut(&player).unwrap().joints.insert(JointId::HandGrip(crate::events::Hand::Left), Transform { position: Vec3::new(2.0, 1.0, 2.0), ..Default::default() });
        run(&mut physics, &mut scene, &rigs, 0.1);
        assert!(physics.grab_door(player, "door", crate::events::Hand::Right, handle_at(0.0)), "the hand is at the handle");
        assert!(!physics.grab_door(player, "door", crate::events::Hand::Left, Vec3::new(2.0, 1.0, 2.0)), "a hand across the room took the handle");
        let mut lag = 0.0f32;
        for k in 0..48 {
            let a = 70f32.to_radians() * (k + 1) as f32 / 48.0;
            hold(&mut rigs, handle_at(a));
            physics.step(DT, &mut scene, &rigs);
            lag = lag.max(a - physics.door_angle("door").unwrap());
        }
        let held = run(&mut physics, &mut scene, &rigs, 1.0);
        let at = *held.last().unwrap();
        eprintln!("swung to 70 deg by the handle: the leaf was up to {:.1} deg behind the hand, and rests at {:.1}", lag.to_degrees(), at.to_degrees());
        assert!(lag > 1f32.to_radians() && lag < 30f32.to_radians(), "the leaf was {} deg behind the hand", lag.to_degrees());
        assert!((at - 70f32.to_radians()).abs() < 2f32.to_radians(), "held at 70, the leaf rests at {}", at.to_degrees());
        for k in 0..40 {
            hold(&mut rigs, handle_at((70.0 + 55.0 * (k + 1) as f32 / 40.0).to_radians()));
            physics.step(DT, &mut scene, &rigs);
        }
        let stopped = run(&mut physics, &mut scene, &rigs, 1.5);
        let most = stopped.iter().copied().fold(f32::MIN, f32::max);
        assert!(most < 100.6f32.to_radians(), "pulled past its stop to {} deg", most.to_degrees());
        let tail = &stopped[stopped.len() - 60..];
        let step = tail.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(step < 0.02f32.to_radians(), "held past its stop it jitters {} deg a tick", step.to_degrees());
        assert!(physics.is_held("door"), "25 degrees past the stop is 30 cm: not out of reach");
        for k in 0..90 {
            hold(&mut rigs, handle_at((125.0 * (1.0 - (k + 1) as f32 / 90.0)).to_radians()));
            physics.step(DT, &mut scene, &rigs);
        }
        run(&mut physics, &mut scene, &rigs, 0.5);
        assert_eq!(physics.door_bolted("door"), Some(false), "a held handle keeps the bolt drawn");
        physics.release(player, "door", crate::events::Hand::Right);
        hold(&mut rigs, Vec3::new(2.0, 1.0, 2.0));
        let shut = run(&mut physics, &mut scene, &rigs, 1.0);
        assert_eq!(physics.door_bolted("door"), Some(true), "left at {} deg", shut.last().unwrap().to_degrees());
        assert!(shut.last().unwrap().abs() <= (LATCH_PLAY_DEG + 0.1).to_radians());
    }

    /// THE SOFT STOP: a leaf flung at its stop as fast as a leaf turns
    /// arrives without passing it and without bouncing back off it.
    #[test]
    fn a_door_flung_at_its_stop_arrives_softly_and_stays() {
        let _guard = lock();
        let (mut physics, mut scene) = door(DoorDef { friction: 0.5, ..Default::default() });
        let state = physics.dynamic.get("door").unwrap();
        let w = px(Vec3::Y * 18.0);
        let (pos, _) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
        let v = px((Vec3::Y * 18.0).cross(pos - Vec3::new(-0.4, 1.1, 0.0)));
        unsafe {
            physx_sys::PxRigidDynamic_setAngularVelocity_mut(state.ptr.cast::<physx_sys::PxRigidDynamic>(), &raw const w, true);
            physx_sys::PxRigidDynamic_setLinearVelocity_mut(state.ptr.cast::<physx_sys::PxRigidDynamic>(), &raw const v, true);
        }
        let angles = run(&mut physics, &mut scene, &HashMap::new(), 2.0);
        let most = angles.iter().copied().fold(f32::MIN, f32::max);
        let end = *angles.last().unwrap();
        eprintln!("flung at 18 rad/s: reached {:.2} deg, rests at {:.2}", most.to_degrees(), end.to_degrees());
        assert!(most <= 100.3f32.to_radians(), "it passed its stop: {} deg", most.to_degrees());
        assert!(most > 99f32.to_radians(), "it never reached its stop: {} deg", most.to_degrees());
        assert!(end > 97f32.to_radians(), "it bounced back off the stop to {} deg", end.to_degrees());
    }

    /// IN STEP: the headset's own hinge, given the same hand on the same
    /// handle, swings its leaf as the server's physics does -- so the door a
    /// player draws under their own hand is the door the server will send.
    #[test]
    fn the_headsets_hinge_follows_a_held_handle_as_the_physics_does() {
        use crate::door::{HingeLoad, HingeState};
        let _guard = lock();
        let def = latching();
        let (mut physics, mut scene) = door(def.clone());
        let geom = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, Vec3::new(0.4, 1.05, 0.02), &def);
        let load = HingeLoad::of(&geom, &def);
        let local = Vec3::new(0.31, -0.1, 0.02 + crate::door::HANDLE_STANDOFF);
        let mut rigs = rig_at(Vec3::new(2.0, 1.7, 2.0), Some(handle_at(0.0)));
        let player = *rigs.keys().next().unwrap();
        rigs.get_mut(&player).unwrap().joints.insert(JointId::HandGrip(crate::events::Hand::Left), Transform { position: Vec3::new(2.0, 1.0, 2.0), ..Default::default() });
        run(&mut physics, &mut scene, &rigs, 0.1);
        assert!(physics.grab_door(player, "door", crate::events::Hand::Right, handle_at(0.0)));
        let mut own = HingeState::default();
        let mut was = handle_at(0.0);
        let mut worst = 0.0f32;
        for k in 0..180 {
            let t = k as f32 / 60.0;
            let a = if t < 1.0 { 80.0 * t } else if t < 1.5 { 80.0 } else { 80.0 - 50.0 * ((t - 1.5) / 1.5).min(1.0) };
            let hand = handle_at(a.to_radians());
            rigs.get_mut(&player).unwrap().joints.insert(JointId::HandGrip(crate::events::Hand::Right), Transform { position: hand, ..Default::default() });
            physics.step(DT, &mut scene, &rigs);
            own = geom.step_hinge_held(own, &load, &[], &[], &[(was, hand, local)], DT);
            was = hand;
            worst = worst.max((own.angle - physics.door_angle("door").unwrap()).abs());
        }
        let server = physics.door_angle("door").unwrap();
        eprintln!("held handle: the headset's hinge and the physics were never more than {:.1} deg apart; they end at {:.1} and {:.1}", worst.to_degrees(), own.angle.to_degrees(), server.to_degrees());
        assert!(worst < 6f32.to_radians(), "{} deg apart", worst.to_degrees());
        assert!((own.angle - server).abs() < 1.5f32.to_radians());
    }

    fn shipped() -> (PhysicsWorld, Scene) {
        let game = &crate::test_game_dir();
        let scene = Scene::load(&game.join("scenes/test_room.json")).expect("test_room.json loads");
        let mut physics = PhysicsWorld::new();
        physics.rebuild(&scene, game);
        (physics, scene)
    }

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
    /// stops them (`Locomotion::apply_collision`). The hall's swing doors open
    /// under their body, and the closers bring them home behind them. THE
    /// BRICK HALL'S DOOR IS LATCHED: their body alone leans on it and it
    /// stays shut; with a hand at its latch the bolt is drawn, and the same
    /// walk opens it and comes out the far side past the leaf that is bolted.
    #[test]
    fn walking_down_the_hallway_opens_the_swing_doors_by_body_and_the_latched_door_by_its_handle() {
        use crate::locomotion::{Locomotion, LocomotionMode};
        let _guard = lock();
        let (mut physics, mut scene) = shipped();
        let mut loco = Locomotion::new(LocomotionMode::Smooth);
        loco.player_offset = Vec3::new(1.5, 0.0, -2.7);
        let mut rigs = rig_at(Vec3::new(1.5, 1.7, -2.7), None);
        let player = *rigs.keys().next().unwrap();
        physics.ensure_player(player);
        let speed = 1.4;
        let mut opened = [0.0f32; 2];
        let mut walk = |physics: &mut PhysicsWorld, scene: &mut Scene, loco: &mut Locomotion, hand: bool, opened: &mut [f32; 2]| {
            let start = loco.collision_start(Some(loco.player_offset + Vec3::Y * 1.7));
            loco.player_offset.x += speed * DT;
            loco.apply_collision(physics, start);
            let head = loco.player_offset + Vec3::Y * 1.7;
            let rig = rigs.get_mut(&player).unwrap();
            rig.joints.insert(JointId::Head, Transform { position: head, ..Default::default() });
            let grip = if hand { Vec3::new(head.x + 0.4, 1.0, -2.928) } else { Vec3::new(0.0, -20.0, 0.0) };
            rig.joints.insert(JointId::HandGrip(crate::events::Hand::Right), Transform { position: grip, ..Default::default() });
            physics.step(DT, scene, &rigs);
            opened[0] = opened[0].max(physics.door_angle("hall_door_north").unwrap().abs());
            opened[1] = opened[1].max(physics.door_angle("brick_door_north").unwrap().abs());
        };
        let mut seconds = 0.0;
        while seconds < 10.0 {
            walk(&mut physics, &mut scene, &mut loco, false, &mut opened);
            seconds += DT;
        }
        eprintln!("hands down, 10 s: at x = {:.2}; hall door opened {:.0} deg, brick door {:.2} deg", loco.player_offset.x, opened[0].to_degrees(), opened[1].to_degrees());
        assert!(opened[0] > 30f32.to_radians(), "the swing door opened {} deg", opened[0].to_degrees());
        assert!(loco.player_offset.x > 9.0 && loco.player_offset.x < 10.3, "not held at the latched door: {:?}", loco.player_offset);
        assert!(opened[1] < 0.5f32.to_radians(), "a body alone opened the latched door {} deg", opened[1].to_degrees());
        assert_eq!(physics.door_bolted("brick_door_north"), Some(true));
        let back = physics.door_angle("hall_door_north").unwrap();
        assert!(crate::door::DoorGeom::shut(back), "the closer left the hall door at {} deg", back.to_degrees());
        loco.player_offset.x -= 0.6;
        let mut seconds = 0.0;
        while seconds < 8.0 && loco.player_offset.x < 12.0 {
            walk(&mut physics, &mut scene, &mut loco, true, &mut opened);
            seconds += DT;
        }
        eprintln!("hand at the handle: through in {seconds:.1} s; the latched door opened {:.0} deg", opened[1].to_degrees());
        assert!(loco.player_offset.x >= 12.0, "stuck at {:?} after {seconds:.1} s", loco.player_offset);
        assert!(opened[1] > 30f32.to_radians(), "{opened:?}");
        assert_eq!(physics.door_angle("brick_door_south"), Some(0.0));
    }
}
