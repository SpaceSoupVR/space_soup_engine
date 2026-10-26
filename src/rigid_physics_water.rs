//! What the water does to the bodies in it, and the bodies that are lost:
//! buoyancy and drag (`float_bodies`), a body's going in seen once and its
//! pose written back (`settle_bodies`), and the props put back where they
//! started (`respawn_round`).
//!
//! WHAT A STEP COSTS, BY DESIGN. Everything here is per AWAKE body: a body
//! `PhysX` has put to sleep is not read, floated, watched or written back (its
//! pose went to the scene on the step it fell asleep), so a beach of a
//! thousand rocks nobody is throwing costs one flag read each. An awake body
//! costs its pose, one lookup of each water footprint (a grid cell: no ray,
//! no search of the level), and eight small forces while it is in water. The
//! respawn checks take a few bodies a step in turn (`RESPAWN_CHECKS_A_STEP`),
//! not all of them every step. The entries told of are capped by energy
//! (`water::keep_most_energetic`).

use glam::Vec3;
use physx::prelude::*;

use crate::rigid_physics::{from_px_transform, to_px_transform, PhysicsWorld, StepStats};
use crate::scene::Scene;

/// Kilograms a cubic metre of water.
pub const WATER_DENSITY: f32 = 1000.0;
/// A blunt body's drag coefficient in water: a rock, a crate.
pub const WATER_DRAG: f32 = 1.0;
/// What still water takes from a body's speed besides, a second: the slow
/// end, where the square law leaves a float bobbing for a minute.
pub const WATER_DAMPING: f32 = 3.0;
/// And from its turning, a second, fully under.
pub const WATER_SPIN_DAMPING: f32 = 2.5;
/// How many bodies are looked at for a respawn each step, in turn. At the
/// server's 60 steps a second a level of 1,000 props is gone round in two
/// seconds; a body falling out of the world at 50 m/s is that much lower
/// when it is found, and comes back all the same.
pub const RESPAWN_CHECKS_A_STEP: usize = 8;
/// Slower than this, a body under water is resting, m/s.
const RESTING_SPEED: f32 = 0.15;

fn v3(v: physx_sys::PxVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn px(v: Vec3) -> physx_sys::PxVec3 {
    physx_sys::PxVec3 { x: v.x, y: v.y, z: v.z }
}

impl PhysicsWorld {
    /// What the last step cost and saw.
    #[must_use]
    pub fn step_stats(&self) -> StepStats {
        self.stats
    }

    /// How many dynamic bodies this world holds, doors included.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.dynamic.len()
    }

    /// Whether a dynamic body is asleep (nothing if it is not one).
    #[must_use]
    pub fn is_asleep(&self, id: &str) -> Option<bool> {
        let state = self.dynamic.get(id)?;
        Some(unsafe { physx_sys::PxRigidDynamic_isSleeping(state.ptr as *const physx_sys::PxRigidDynamic) })
    }

    /// A dynamic body's linear velocity, m/s.
    #[must_use]
    pub fn velocity_of(&self, id: &str) -> Option<Vec3> {
        let state = self.dynamic.get(id)?;
        Some(v3(unsafe { physx_sys::PxRigidBody_getLinearVelocity(state.ptr as *const physx_sys::PxRigidBody) }))
    }

    /// A dynamic body's centre and rotation as physics has them now.
    #[must_use]
    pub fn pose_of(&self, id: &str) -> Option<(Vec3, glam::Quat)> {
        let state = self.dynamic.get(id)?;
        Some(from_px_transform(&unsafe { (*state.ptr).get_global_pose() }))
    }

    /// A dynamic body's mass, kilograms.
    #[must_use]
    pub fn mass_of(&self, id: &str) -> Option<f32> {
        let state = self.dynamic.get(id)?;
        Some(unsafe { (*state.ptr).get_mass() })
    }

    /// PUT EVERY PROP BACK WHERE IT STARTED, at the next step: every dynamic
    /// body that is not a door and that no hand holds. What a tester's
    /// "bring the rocks back" button asks for.
    pub fn respawn_props(&mut self) {
        self.respawn_all = true;
    }

    /// THE WATER BEARS UP WHAT IS IN IT AND SLOWS IT. Each awake body in
    /// water is taken as eight equal parts (the octants of the box round its
    /// collider); each part under the surface is borne up by the weight of
    /// the water it pushes aside, at its own middle -- so a crate floats
    /// level, a third under, and rights itself, and a rock, heavier than its
    /// water, sinks at the speed its drag allows (about 1 to 1.5 m/s). Drag
    /// is the square law on the body's face (its volume to the two-thirds)
    /// with a little linear damping, taken from the velocity implicitly, so
    /// no speed or step makes it unstable.
    ///
    /// Against the STILL surface: physics knows where water stands, not its
    /// waves, so a float rides the sea's level and not its swell.
    pub(crate) fn float_bodies(&mut self, dt: f32) {
        if self.water.is_empty() {
            return;
        }
        let g = 9.81f32;
        for (id, state) in &self.dynamic {
            if self.doors.contains_key(id) {
                continue;
            }
            let dynamic = state.ptr.cast::<physx_sys::PxRigidDynamic>();
            if unsafe { physx_sys::PxRigidDynamic_isSleeping(dynamic) } {
                continue;
            }
            let body = state.ptr.cast::<physx_sys::PxRigidBody>();
            let (pos, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
            let Some(surface) = self.water.iter().filter_map(|w| w.surface_at(pos.x, pos.z)).reduce(f32::max) else { continue };
            let extent = crate::water::world_half_extents(state.half, rot);
            if pos.y - extent.y >= surface {
                continue;
            }
            let part = state.volume / 8.0;
            let tall = extent.y.max(1e-4);
            let mut under = 0.0f32;
            for k in 0..8 {
                let sign = Vec3::new(if k & 1 == 0 { -0.5 } else { 0.5 }, if k & 2 == 0 { -0.5 } else { 0.5 }, if k & 4 == 0 { -0.5 } else { 0.5 });
                let at = pos + rot * (state.half * sign);
                let share = ((surface - (at.y - 0.5 * tall)) / tall).clamp(0.0, 1.0);
                if share <= 0.0 {
                    continue;
                }
                under += share / 8.0;
                let lift = px(Vec3::Y * (WATER_DENSITY * g * part * share));
                unsafe { physx_sys::PxRigidBodyExt_addForceAtPos(body, &raw const lift, &px(at), physx_sys::PxForceMode::Force, false) };
            }
            if under <= 0.0 {
                continue;
            }
            let mass = unsafe { (*state.ptr).get_mass() }.max(1e-4);
            let v = v3(unsafe { physx_sys::PxRigidBody_getLinearVelocity(body) });
            let face = state.volume.max(0.0).powf(2.0 / 3.0);
            let rate = under * (0.5 * WATER_DENSITY * WATER_DRAG * face * v.length() / mass + WATER_DAMPING);
            let slowed = px(v / (1.0 + rate * dt));
            let w = v3(unsafe { physx_sys::PxRigidBody_getAngularVelocity(body) });
            let turned = px(w / (1.0 + under * WATER_SPIN_DAMPING * dt));
            unsafe {
                physx_sys::PxRigidDynamic_setLinearVelocity_mut(dynamic, &raw const slowed, false);
                physx_sys::PxRigidDynamic_setAngularVelocity_mut(dynamic, &raw const turned, false);
            }
        }
    }

    /// THE PROPS THAT ARE LOST COME BACK: a few bodies a step, in turn
    /// (`RESPAWN_CHECKS_A_STEP`), each put back where it started if it has
    /// fallen out of the level (`kill_height`), if its fixed interval is up
    /// (`RigidBodyDef::respawn_interval`), or if it has rested under water
    /// for as long as its author gave it (`RigidBodyDef::respawn_sunk`) -- and
    /// every prop at once when asked (`respawn_props`). A body a hand holds
    /// is never taken from it, and a door is never moved.
    pub(crate) fn respawn_round(&mut self) {
        let count = self.dynamic.len();
        if count == 0 {
            self.respawn_all = false;
            return;
        }
        let all = std::mem::take(&mut self.respawn_all);
        let (first, take) = if all { (0, count) } else { (self.respawn_cursor % count, RESPAWN_CHECKS_A_STEP.min(count)) };
        self.respawn_cursor = (first + take) % count;
        let (kill_y, now) = (self.kill_y, self.clock);
        let zero = PxVec3::new(0.0, 0.0, 0.0);
        let mut respawned = 0;
        for (skip, n) in [(first, take.min(count - first)), (0, (first + take).saturating_sub(count))] {
          for (id, state) in self.dynamic.iter_mut().skip(skip).take(n) {
            let since = (now - state.checked_at).max(0.0) as f32;
            state.checked_at = now;
            if self.doors.contains_key(id) {
                continue;
            }
            let pose = unsafe { (*state.ptr).get_global_pose() };
            let (pos, rot) = from_px_transform(&pose);
            let fell_out = pos.y < kill_y;
            let timed = state.respawn_interval.is_some_and(|interval| {
                state.elapsed += since;
                state.elapsed >= interval
            });
            let sunk = state.respawn_sunk.is_some_and(|after| {
                let top = pos.y + crate::water::world_half_extents(state.half, rot).y;
                let under = self.water.iter().filter_map(|w| w.surface_at(pos.x, pos.z)).reduce(f32::max).is_some_and(|s| top < s);
                let speed = v3(unsafe { physx_sys::PxRigidBody_getLinearVelocity(state.ptr as *const physx_sys::PxRigidBody) }).length();
                state.sunk_for = if under && speed < RESTING_SPEED { state.sunk_for + since } else { 0.0 };
                state.sunk_for >= after
            });
            let held = self.grabs.keys().any(|(_, held, _)| held == id);
            if held || !(fell_out || timed || sunk || all) {
                continue;
            }
            state.elapsed = 0.0;
            state.sunk_for = 0.0;
            state.water = crate::water::WaterState::default();
            let spawn = to_px_transform(state.spawn_pos, state.spawn_rot);
            unsafe {
                (*state.ptr).set_global_pose(&spawn, true);
                (*state.ptr).set_linear_velocity(&zero, true);
                (*state.ptr).set_angular_velocity(&zero, true);
            }
            respawned += 1;
          }
        }
        self.stats.respawned = respawned;
    }

    /// AFTER THE STEP: each awake body's pose goes to its scene object, and
    /// its going into the water is seen, with the speed and the mass physics
    /// has for it (`take_water_entries`). A body asleep, whose pose went the
    /// step it fell asleep, is passed over.
    pub(crate) fn settle_bodies(&mut self, scene: &mut Scene) {
        let now = self.clock;
        let mut stats = StepStats { bodies: self.dynamic.len(), respawned: self.stats.respawned, ..Default::default() };
        let mut entries: Vec<crate::water::WaterEntry> = Vec::new();
        for (id, state) in &mut self.dynamic {
            let asleep = unsafe { physx_sys::PxRigidDynamic_isSleeping(state.ptr as *const physx_sys::PxRigidDynamic) };
            if asleep && state.asleep {
                continue;
            }
            state.asleep = asleep;
            stats.awake += 1;
            let pose = unsafe { (*state.ptr).get_global_pose() };
            let (pos, rot) = from_px_transform(&pose);
            if scene.objects.get(state.object).is_none_or(|o| o.id != *id) {
                let Some(index) = scene.objects.iter().position(|o| o.id == *id) else { continue };
                state.object = index;
            }
            let entity = &mut scene.objects[state.object];
            entity.cuboid.position = pos;
            entity.cuboid.rotation = rot;
            if self.water.is_empty() {
                continue;
            }
            let (v, mass) = unsafe { ((*state.ptr).get_linear_velocity(), (*state.ptr).get_mass()) };
            let half = crate::water::world_half_extents(state.half, rot);
            let surface = self.water.iter().filter_map(|w| w.surface_at(pos.x, pos.z)).reduce(f32::max);
            if let Some(mut e) = state.water.see(Some(now), pos, half, Vec3::new(v.x(), v.y(), v.z()), mass, surface) {
                e.id.clone_from(id);
                entries.push(e);
            }
            stats.in_water += usize::from(state.water.is_in());
        }
        stats.entries_seen = entries.len();
        crate::water::keep_most_energetic(&mut entries, crate::water::MAX_ENTRIES_A_STEP);
        stats.entries_told = entries.len();
        self.water_entries.extend(entries);
        crate::water::keep_most_energetic(&mut self.water_entries, 256);
        self.stats = stats;
    }
}
