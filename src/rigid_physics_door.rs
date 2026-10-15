use std::collections::HashMap;

use glam::{Quat, Vec3};
use physx::prelude::*;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::door::{body_push_force, DoorDef, DoorGeom, BODY_PUSH_RADIUS, BODY_PUSH_SPAN};
use crate::rig::{JointId, PlayerRig};
use crate::rigid_physics::{from_px_transform, to_px_transform, to_raw_transform, DynamicActor, PhysicsWorld};
use crate::scene::GameObject;

pub const HINGE_DRY_FRICTION: f32 = 1.0;

pub const CLOSER_PRELOAD: f32 = 2.0;

const DOOR_TICK: f32 = 1.0 / 60.0;

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

impl PhysicsWorld {

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

        }
        self.doors.insert(obj.id.clone(), geom);
        self.door_hinges.insert(obj.id.clone(), (def.closer.max(0.0), def.friction.max(0.0)));
    }

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
                unsafe { physx_sys::PxRigidBody_addTorque_mut(body, &t, physx_sys::PxForceMode::Force, true) };
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

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::runtime_test_support::PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
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
        let spread = tail.iter().cloned().fold(f32::MIN, f32::max) - tail.iter().cloned().fold(f32::MAX, f32::min);
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

    fn shipped() -> (PhysicsWorld, Scene) {
        let game = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
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

    #[test]
    fn walking_down_the_hallway_pushes_both_pairs_of_doors_open() {
        use crate::locomotion::{Locomotion, LocomotionMode};
        let _guard = lock();
        let (mut physics, mut scene) = shipped();
        let mut loco = Locomotion::new(LocomotionMode::Smooth);

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

        assert!(seconds < 12.0, "took {seconds:.1} s");
        assert!(opened[0] > 30f32.to_radians() && opened[1] > 30f32.to_radians(), "{opened:?}");

        for _ in 0..300 {
            physics.step(DT, &mut scene, &HashMap::new());
        }
        let back = physics.door_angle("hall_door_south").unwrap();
        assert!(crate::door::DoorGeom::shut(back), "the closer left the hall door at {} deg", back.to_degrees());
    }
}
