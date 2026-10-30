//! MOVING BY THE ARMS: no stick to walk with. A hand that meets fixed
//! geometry stays where it met it, and the body is carried by however far
//! the tracked hand has gone past that point: push the floor back and the
//! body goes forward, swing both arms down hard and it leaves the ground.
//! Let go and the body keeps the speed the hands last gave it, under gravity.
//!
//! The body is the head and a short torso under it (`body_height`), not a
//! standing person: gravity pulls it down until the torso rests on the
//! ground, wherever the player's real head is. Walls stop the head.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::rigid_physics::PhysicsWorld;

/// How arm movement feels, per game (`"locomotion": {"arms": {...}}` in the
/// manifest, every key optional).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArmSettings {
    /// The hand's collision sphere, metres.
    pub hand_radius: f32,
    /// The head's collision sphere, metres: what walls and ceilings stop.
    pub head_radius: f32,
    /// From the eyes down to the bottom of the body, metres: how high the
    /// eyes ride when the body rests on the ground.
    pub body_height: f32,
    /// Metres a second, each second, while no hand holds anything.
    pub gravity: f32,
    /// The release speed is the hands' last push times this: above 1 the
    /// arms throw the body further than they moved it.
    pub fling: f32,
    /// No faster than this, metres a second, however hard the throw.
    pub max_speed: f32,
    /// How quickly a body standing on the ground with its hands free comes
    /// to rest sideways, per second.
    pub ground_drag: f32,
    /// The furthest the hands may move the body in one step, metres: a
    /// tracking jump must not throw the player across the level.
    pub max_push: f32,
    /// How far a held hand must lift off its surface to let go, metres.
    pub release: f32,
}

impl Default for ArmSettings {
    fn default() -> Self {
        Self {
            hand_radius: 0.06,
            head_radius: 0.18,
            body_height: 0.6,
            gravity: 9.8,
            fling: 1.15,
            max_speed: 7.0,
            ground_drag: 8.0,
            max_push: 0.35,
            release: 0.01,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Hold {
    /// Where the hand's sphere sits against the surface, world.
    anchor: Vec3,
    /// The surface's normal there, facing the hand.
    normal: Vec3,
}

/// How many steps of push the release speed is averaged over.
const HISTORY: usize = 4;
/// How far below the body the ground is still looked for, metres: what a
/// real crouch can push the body into in one step.
const GROUND_REACH: f32 = 0.6;
/// Kept between the head and a wall it slides along, metres.
const SKIN: f32 = 0.005;

/// What one step did.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArmStep {
    /// How far the body moved, world.
    pub moved: Vec3,
    /// Whether each hand (left, right) holds a surface after the step.
    pub holding: [bool; 2],
    /// Whether the body rests on the ground.
    pub grounded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ArmLocomotion {
    pub settings: ArmSettings,
    /// The body's speed, metres a second, world.
    pub velocity: Vec3,
    holds: [Option<Hold>; 2],
    /// Where each free hand was last step, world: a new contact is found by
    /// sweeping from here to where the hand is now.
    free_at: [Option<Vec3>; 2],
    pushes: [Vec3; HISTORY],
    pushes_len: usize,
    pushes_next: usize,
    grounded: bool,
}

impl ArmLocomotion {
    #[must_use]
    pub fn new(settings: ArmSettings) -> Self {
        Self { settings, ..Self::default() }
    }

    /// Let go with both hands and forget where they were, keeping the body's
    /// speed: after the player is turned or put somewhere else, the hands'
    /// world positions jumped, and that jump is not a push.
    pub fn release_hands(&mut self) {
        self.holds = [None; 2];
        self.free_at = [None; 2];
        self.pushes_len = 0;
    }

    /// Stop dead and let go: a respawn.
    pub fn reset(&mut self) {
        self.release_hands();
        self.velocity = Vec3::ZERO;
        self.grounded = false;
    }

    #[must_use]
    pub fn is_holding(&self) -> bool {
        self.holds.iter().any(Option::is_some)
    }

    /// One step. `head` is the tracked head and `hands` the tracked hands
    /// (left, right; `None` when untracked), all world, placed with the
    /// body where it is now.
    pub fn step(&mut self, physics: &PhysicsWorld, dt: f32, head: Vec3, hands: [Option<Vec3>; 2]) -> ArmStep {
        let dt = dt.clamp(1e-4, 0.1);
        let held_before = self.is_holding();
        let mut moved = Vec3::ZERO;

        if !held_before {
            self.velocity.y -= self.settings.gravity * dt;
            let want = self.velocity * dt;
            let got = self.move_body(physics, head, want);
            self.stop_where_blocked(want, got, dt);
            moved += got;
        }

        let mut pushes = Vec::with_capacity(2);
        for (i, hand) in hands.into_iter().enumerate() {
            match hand {
                Some(tracked) => {
                    if let Some(push) = self.hand(physics, i, tracked + moved) {
                        pushes.push(push);
                    }
                }
                None => {
                    self.holds[i] = None;
                    self.free_at[i] = None;
                }
            }
        }

        if pushes.is_empty() {
            if held_before {
                self.throw();
            }
        } else {
            let mut push = pushes.iter().copied().sum::<Vec3>() / pushes.len() as f32;
            push = push.clamp_length_max(self.settings.max_push);
            let got = self.move_body(physics, head + moved, push);
            moved += got;
            self.note_push(got / dt);
        }

        if self.grounded && !self.is_holding() {
            let keep = (-self.settings.ground_drag * dt).exp();
            self.velocity.x *= keep;
            self.velocity.z *= keep;
            self.velocity.y = self.velocity.y.max(0.0);
        }

        ArmStep { moved, holding: [self.holds[0].is_some(), self.holds[1].is_some()], grounded: self.grounded }
    }

    /// Hand `i`, tracked at `target` (world, after this step's fall): the
    /// push it asks of the body, if it holds a surface.
    fn hand(&mut self, physics: &PhysicsWorld, i: usize, target: Vec3) -> Option<Vec3> {
        if let Some(hold) = self.holds[i] {
            if (target - hold.anchor).dot(hold.normal) > self.settings.release {
                self.holds[i] = None;
                self.free_at[i] = Some(target);
                return None;
            }
            return Some(hold.anchor - target);
        }
        let from = self.free_at[i].unwrap_or(target);
        self.free_at[i] = Some(target);
        let travel = target - from;
        let length = travel.length();
        if length < 1e-5 {
            return None;
        }
        let dir = travel / length;
        let hit = physics.sphere_sweep(from, self.settings.hand_radius, dir, length).filter(|h| h.fixed)?;
        let anchor = from + dir * hit.distance;
        self.holds[i] = Some(Hold { anchor, normal: hit.normal });
        self.free_at[i] = None;
        Some(anchor - target)
    }

    /// Move the body by `want`: the head slides along what it meets, and the
    /// body's bottom stops on the ground under the head. What it actually moved.
    fn move_body(&mut self, physics: &PhysicsWorld, head: Vec3, want: Vec3) -> Vec3 {
        let mut got = self.slide_head(physics, head, want);
        let head_now = head + got;
        let height = self.settings.body_height;
        let ground = physics.raycast_down(head_now, height + GROUND_REACH).map(|(p, _)| p.y);
        self.grounded = false;
        if let Some(ground_y) = ground {
            let bottom = head_now.y - height;
            if bottom <= ground_y + 0.01 {
                got.y += ground_y - bottom;
                self.grounded = true;
            }
        }
        got
    }

    /// The head's sphere moved by `want`, sliding once along whatever it hits.
    fn slide_head(&self, physics: &PhysicsWorld, head: Vec3, want: Vec3) -> Vec3 {
        let radius = self.settings.head_radius;
        let mut at = head;
        let mut left = want;
        for _ in 0..2 {
            let length = left.length();
            if length < 1e-6 {
                break;
            }
            let dir = left / length;
            match physics.sphere_sweep(at, radius, dir, length).filter(|h| h.fixed) {
                Some(hit) => {
                    at += dir * (hit.distance - SKIN).max(0.0);
                    let rest = left - dir * hit.distance;
                    left = rest - hit.normal * rest.dot(hit.normal).min(0.0);
                }
                None => {
                    at += left;
                    break;
                }
            }
        }
        at - head
    }

    /// After a fall step: speed the body could not use is gone (a wall it
    /// hit, the ground it landed on).
    fn stop_where_blocked(&mut self, want: Vec3, got: Vec3, dt: f32) {
        for axis in 0..3 {
            if (want[axis] - got[axis]).abs() > 1e-4 {
                self.velocity[axis] = got[axis] / dt;
            }
        }
        if self.grounded {
            self.velocity.y = self.velocity.y.max(0.0);
        }
    }

    fn note_push(&mut self, velocity: Vec3) {
        self.pushes[self.pushes_next] = velocity;
        self.pushes_next = (self.pushes_next + 1) % HISTORY;
        self.pushes_len = (self.pushes_len + 1).min(HISTORY);
        self.velocity = velocity;
    }

    /// Both hands just let go: the body flies on at the hands' recent push.
    fn throw(&mut self) {
        if self.pushes_len == 0 {
            return;
        }
        let mean = self.pushes[..self.pushes_len].iter().copied().sum::<Vec3>() / self.pushes_len as f32;
        self.velocity = (mean * self.settings.fling).clamp_length_max(self.settings.max_speed);
        self.pushes_len = 0;
    }
}

#[cfg(test)]
#[path = "arm_locomotion_tests.rs"]
mod tests;
