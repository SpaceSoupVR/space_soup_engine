use std::collections::HashMap;

use glam::Vec3;
use physx::prelude::*;

use physx::scene::Scene as _;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::rig::PlayerRig;
use crate::rigid_physics::{from_px_transform, to_px_transform, PhysicsWorld};
use crate::scene::Scene;

impl PhysicsWorld {
    pub fn raycast_down(&self, origin: Vec3, max_distance: f32) -> Option<(Vec3, Vec3)> {
        self.raycast(origin, Vec3::NEG_Y, max_distance)
    }

    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<(Vec3, Vec3)> {
        let origin_px = physx_sys::PxVec3 {
            x: origin.x,
            y: origin.y,
            z: origin.z,
        };
        let dir_px = physx_sys::PxVec3 {
            x: dir.x,
            y: dir.y,
            z: dir.z,
        };
        let mut hit: physx_sys::PxRaycastHit = unsafe { std::mem::zeroed() };
        let hit_flags = physx_sys::PxHitFlags::Position | physx_sys::PxHitFlags::Normal;
        let filter_data = unsafe { physx_sys::PxQueryFilterData_new() };
        let found = unsafe {
            physx_sys::PxSceneQueryExt_raycastSingle(
                self.scene.as_ptr(),
                &origin_px,
                &dir_px,
                max_distance,
                hit_flags,
                &mut hit,
                &filter_data,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if !found {
            return None;
        }
        let p = hit.position;
        let n = hit.normal;
        Some((Vec3::new(p.x, p.y, p.z), Vec3::new(n.x, n.y, n.z)))
    }

    // Sets linear velocity directly rather than adding a PhysX force impulse -- spawned
    // projectiles/ejected casings always start at rest, so "apply an impulse" and "set the exit
    // velocity" are equivalent for this use case and avoid pulling in mass-scaling semantics.
    pub fn apply_impulse(&mut self, id: &str, velocity: Vec3) {
        let Some(state) = self.dynamic.get(id) else {
            log::warn!("apply_impulse: unknown or non-dynamic object '{id}'");
            return;
        };
        let v = PxVec3::new(velocity.x, velocity.y, velocity.z);
        unsafe { (*state.ptr).set_linear_velocity(&v, true) };
    }

    // Pulls a dynamic actor out of simulation without touching the scene object -- used when a
    // rigid-body object gets kinematically socketed (magazine into a mag well) so PhysX's own
    // gravity/step doesn't fight the socket's per-frame transform override.
    pub fn despawn_actor(&mut self, id: &str) {
        let Some(state) = self.dynamic.remove(id) else {
            return;
        };
        if state.ptr.is_null() {
            return;
        }
        unsafe {
            self.scene.remove_actor(&mut *state.ptr, false);
            physx_sys::PxActor_release_mut(state.ptr as *mut physx_sys::PxActor);
        }
    }

    pub fn step(&mut self, dt: f32, scene: &mut Scene, rigs: &HashMap<PlayerId, PlayerRig>) {
        for (id, &ptr) in &self.kinematic {
            let Some(obj) = scene.find_object(id) else {
                continue;
            };
            let target = to_px_transform(obj.cuboid.position, obj.cuboid.rotation);

            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        for (&(player, hand), &ptr) in &self.hand_anchors {
            if ptr.is_null() {
                continue;
            }
            let Some(rig) = rigs.get(&player) else {
                continue;
            };
            let grip = rig.hand_grip(hand);
            let target = to_px_transform(grip.position, grip.rotation);

            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        let zero = PxVec3::new(0.0, 0.0, 0.0);
        for state in self.dynamic.values_mut() {
            const KILL_Y: f32 = -15.0;

            let fell_out = unsafe { (*state.ptr).get_global_pose() }.translation().y() < KILL_Y;

            let due_for_timed_respawn = state
                .respawn_interval
                .map(|interval| {
                    state.elapsed += dt;
                    state.elapsed >= interval
                })
                .unwrap_or(false);

            if !fell_out && !due_for_timed_respawn {
                continue;
            }
            state.elapsed = 0.0;
            let spawn = to_px_transform(state.spawn_pos, state.spawn_rot);

            unsafe {
                (*state.ptr).set_global_pose(&spawn, true);
                (*state.ptr).set_linear_velocity(&zero, true);
                (*state.ptr).set_angular_velocity(&zero, true);
            }
        }

        if let Err(e) = self.scene.step(
            dt,
            None::<&mut physx_sys::PxBaseTask>,
            Some(&mut self.scratch),
            true,
        ) {
            log::warn!("rigid_physics: simulation step failed: {e:?}");
            return;
        }

        for (id, state) in &self.dynamic {
            let pose = unsafe { (*state.ptr).get_global_pose() };
            let (pos, rot) = from_px_transform(&pose);
            if let Some(obj) = scene.find_object_mut(id) {
                obj.cuboid.position = pos;
                obj.cuboid.rotation = rot;
            }
        }
    }
}
