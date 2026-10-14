use std::collections::HashMap;

use glam::Vec3;
use physx::prelude::*;

use physx::scene::Scene as _;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::rig::PlayerRig;
use crate::rigid_physics::{from_px_transform, to_px_transform, PhysicsWorld};
use crate::scene::Scene;

/// How far a hand anchor may move in one tick before the move counts as a
/// teleport: a hand swung at 15 m/s moves 25 cm a tick at 60 Hz.
const HAND_TELEPORT: f32 = 0.5;

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
    /// Remove an object's STATIC collider, if it has one.
    ///
    /// Separate from `despawn_actor`, which consults only the `dynamic` map --
    /// calling that for a wall returns immediately and does nothing, which is
    /// how a breached structure ended up visibly broken and still solid.
    ///
    /// Returns whether anything was removed, so a caller can tell "no collider
    /// to remove" from "collider removed" rather than assuming.
    pub fn despawn_static(&mut self, id: &str) -> bool {
        let Some(ptr) = self.statics.remove(id) else {
            return false;
        };
        if ptr.is_null() {
            return false;
        }
        unsafe {
            self.scene.remove_actor(&mut *ptr, false);
            physx_sys::PxActor_release_mut(ptr as *mut physx_sys::PxActor);
        }
        true
    }

    /// Whether an object has a collider of EITHER kind in the scene.
    ///
    /// The two maps are separate and each despawn consults only its own, which
    /// has already caused a breached wall to stay solid. Anything asking "is
    /// this thing collidable right now" wants both.
    pub fn has_collider(&self, id: &str) -> bool {
        self.statics.contains_key(id) || self.dynamic.contains_key(id)
    }

    /// Whether an object currently has a static collider in the scene.
    pub fn has_static(&self, id: &str) -> bool {
        self.statics.contains_key(id)
    }

    pub fn despawn_actor(&mut self, id: &str) {
        // Before the actor: a joint outliving one of its bodies is a dangling
        // pointer the next step follows.
        self.release_all(id);
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

            // A JUMP IS A TELEPORT, NOT A SWING. A kinematic moved by its
            // target sweeps the whole way at the speed of the jump, and the
            // CCD of anything it crosses takes that speed: a hand anchor's
            // first tick (from the origin) or a player's teleport carried a
            // hand through a door and flung the leaf round at 20 rad/s.
            let (now, _) = from_px_transform(&unsafe { (*ptr).get_global_pose() });
            if (now - grip.position).length() > HAND_TELEPORT {
                unsafe { (*ptr).set_global_pose(&target, true) };
            }
            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        // The props that are lost come back, a few a step in turn: under THIS
        // level's floor (`PhysicsWorld::kill_y`, found at load), sunk and
        // settled, or asked for. See `respawn_round`.
        self.clock += dt as f64;
        self.respawn_round();

        // Bodies push doors before the step, which integrates the push; the
        // water bears up and slows what is in it; a hand that has gone too
        // far from what it held has let go.
        self.push_doors(rigs);
        self.float_bodies(dt);
        self.drop_far_grabs();
        self.part_hands();

        if let Err(e) = self.scene.step(
            dt,
            None::<&mut physx_sys::PxBaseTask>,
            Some(&mut self.scratch),
            true,
        ) {
            log::warn!("rigid_physics: simulation step failed: {e:?}");
            return;
        }

        self.settle_bodies(scene);
    }
}
