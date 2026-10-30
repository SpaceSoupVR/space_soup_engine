//! A player in a test level -- a floor at y = 0, a wall 3 m ahead (-z), a
//! loose crate to the right -- moved only by their tracked hands. The body
//! rests on the floor with the eyes `body_height` above it.

use std::path::Path;

use glam::Vec3;

use super::{ArmLocomotion, ArmSettings};
use crate::rigid_physics::PhysicsWorld;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::Scene;

const DT: f32 = 1.0 / 72.0;

fn level() -> PhysicsWorld {
    let boxed = |id: &str, at: [f32; 3], half: [f32; 3], mode: &str| {
        serde_json::json!({
            "id": id,
            "cuboid": { "position": at, "half_size": half, "rotation": [0.0, 0.0, 0.0, 1.0] },
            "rigid_body": { "mode": mode }
        })
    };
    let scene: Scene = serde_json::from_value(serde_json::json!({
        "name": "arms_test",
        "objects": [
            boxed("floor", [0.0, -0.5, 0.0], [30.0, 0.5, 30.0], "Static"),
            boxed("wall", [0.0, 2.0, -3.2], [5.0, 2.0, 0.2], "Static"),
            boxed("crate", [3.0, 0.5, 0.0], [0.5, 0.5, 0.5], "Dynamic"),
        ]
    }))
    .unwrap();
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    physics
}

/// A player: where their head is, and arm movement.
struct Player {
    head: Vec3,
    arms: ArmLocomotion,
}

/// The eyes' height with the body resting on the floor.
fn resting() -> f32 {
    ArmSettings::default().body_height
}

impl Player {
    /// A player whose body rests `above` metres over the floor at `x`, `z`.
    fn at(x: f32, above: f32, z: f32) -> Self {
        Self { head: Vec3::new(x, resting() + above, z), arms: ArmLocomotion::new(ArmSettings::default()) }
    }

    /// How high the body's bottom is over the floor.
    fn height(&self) -> f32 {
        self.head.y - resting()
    }

    /// One step with the hands where the player holds them, relative to
    /// their head (as a headset tracks them).
    fn step(&mut self, physics: &PhysicsWorld, hands: [Option<Vec3>; 2]) -> super::ArmStep {
        let step = self.arms.step(physics, DT, self.head, hands.map(|h| h.map(|h| self.head + h)));
        self.head += step.moved;
        step
    }

    fn idle(&mut self, physics: &PhysicsWorld, steps: usize) {
        for _ in 0..steps {
            self.step(physics, [None, None]);
        }
    }
}

/// Both hands a little apart, `y` above the floor (the body resting on it),
/// `z` ahead of the head.
fn hands_at(y: f32, z: f32) -> [Option<Vec3>; 2] {
    [Some(Vec3::new(-0.3, y - resting(), z)), Some(Vec3::new(0.3, y - resting(), z))]
}

#[test]
fn a_player_standing_still_stays_put() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 0.0);
    player.idle(&physics, 144);
    assert!((player.head - Vec3::new(0.0, resting(), 0.0)).length() < 0.01, "drifted to {}", player.head);
    assert!(player.arms.velocity.length() < 0.01);
}

#[test]
fn a_player_in_the_air_falls_and_lands_on_the_floor() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 2.0, 0.0);
    player.idle(&physics, 72);
    assert!(player.height().abs() < 0.02, "resting {} m up", player.height());
    let step = player.step(&physics, [None, None]);
    assert!(step.grounded);
}

#[test]
fn pushing_the_floor_back_moves_the_body_forward() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 0.0);
    // Hands come down onto the floor ahead...
    for k in 0..20 {
        player.step(&physics, hands_at(0.3 - k as f32 * 0.015, -0.4));
    }
    let start = player.head;
    // ...and sweep back 40 cm along it.
    let mut held = false;
    for k in 0..20 {
        let step = player.step(&physics, hands_at(0.0, -0.4 + k as f32 * 0.02));
        held |= step.holding[0] && step.holding[1];
    }
    assert!(held, "the hands never took hold of the floor");
    let moved = player.head - start;
    assert!(moved.z < -0.3, "body moved {moved}, not forward");
    assert!(moved.x.abs() < 0.02 && moved.y.abs() < 0.05, "body moved {moved}");
}

#[test]
fn letting_go_after_a_push_flings_the_body_on_and_the_ground_stops_it() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 6.0);
    for k in 0..20 {
        player.step(&physics, hands_at(0.3 - k as f32 * 0.015, -0.4));
    }
    for k in 0..10 {
        player.step(&physics, hands_at(0.0, -0.4 + k as f32 * 0.05));
    }
    let at_release = player.head;
    player.step(&physics, hands_at(0.3, 0.1));
    assert!(!player.arms.is_holding());
    assert!(player.arms.velocity.z < -2.0, "release speed {}", player.arms.velocity);
    player.idle(&physics, 144);
    let glided = at_release.z - player.head.z;
    assert!(glided > 0.2, "the body stopped dead on release ({glided} m)");
    assert!(player.arms.velocity.length() < 0.05, "still sliding at {}", player.arms.velocity);
}

#[test]
fn shoving_down_on_the_floor_jumps_and_the_body_comes_back_down() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 6.0);
    for k in 0..20 {
        player.step(&physics, hands_at(0.5 - k as f32 * 0.025, -0.2));
    }
    // The tracked hands drive 30 cm down "into" the floor in 6 steps: the
    // floor will not give, so the body rises.
    for k in 0..6 {
        player.step(&physics, hands_at(-0.05 * (k + 1) as f32, -0.2));
    }
    assert!(player.height() > 0.2, "body rose only {}", player.height());
    player.step(&physics, hands_at(0.6, -0.2));
    let mut peak = player.height();
    for _ in 0..144 {
        player.step(&physics, hands_at(0.6, -0.2));
        peak = peak.max(player.height());
    }
    assert!(peak > 0.4, "no jump: peak {peak}");
    assert!(player.height().abs() < 0.02, "never landed: {}", player.height());
}

#[test]
fn a_wall_stops_the_head() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, -1.0);
    player.arms.velocity = Vec3::new(0.0, 0.0, -6.0);
    player.idle(&physics, 72);
    let head_z = player.head.z;
    assert!(head_z > -3.0 + 0.18 - 0.02, "head went into the wall: {head_z}");
    assert!(player.arms.velocity.z.abs() < 0.05);
}

#[test]
fn a_tracking_jump_cannot_throw_the_body_far() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 6.0);
    for k in 0..20 {
        player.step(&physics, hands_at(0.3 - k as f32 * 0.015, -0.4));
    }
    let start = player.head;
    player.step(&physics, hands_at(-2.0, -0.4));
    let moved = (player.head - start).length();
    assert!(moved <= ArmSettings::default().max_push + 1e-3, "one step moved the body {moved} m");
}

#[test]
fn a_hand_that_loses_tracking_lets_go() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 6.0);
    for k in 0..20 {
        player.step(&physics, hands_at(0.3 - k as f32 * 0.015, -0.4));
    }
    assert!(player.arms.is_holding());
    player.step(&physics, [None, None]);
    assert!(!player.arms.is_holding());
}

#[test]
fn a_loose_crate_gives_the_hands_nothing_to_push_off() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 0.0);
    // The right hand reaches into the crate's side and pushes on it.
    for k in 0..30 {
        let step = player.step(&physics, [None, Some(Vec3::new(2.0 + k as f32 * 0.02, 0.0, 0.0))]);
        assert!(!step.holding[1], "the hand held the crate");
    }
    assert!(player.head.x.abs() < 0.01, "pushed off the crate: {}", player.head);
}

#[test]
fn a_game_turns_arm_movement_on_in_its_manifest() {
    use crate::locomotion::{Locomotion, LocomotionMode, LocomotionSettings};
    let manifest = serde_json::json!({ "locomotion": { "mode": "arms", "arms": { "fling": 1.5 } } });
    let settings = LocomotionSettings::from_manifest(&manifest, "any");
    assert_eq!(settings.mode, Some(LocomotionMode::Arms));
    assert!((settings.arms.fling - 1.5).abs() < 1e-6);
    assert!((settings.arms.gravity - ArmSettings::default().gravity).abs() < 1e-6, "unset keys keep their defaults");
    let mut locomotion = Locomotion::new(LocomotionMode::Smooth);
    locomotion.apply_settings(&settings);
    assert_eq!(locomotion.mode, LocomotionMode::Arms);
    let walking = LocomotionSettings::from_manifest(&serde_json::json!({ "locomotion": { "walk_speed": 2.0 } }), "any");
    assert_eq!(walking.mode, None, "a game that names no mode keeps the player's");
}

#[test]
fn standing_up_in_real_life_does_not_lift_the_body_off_the_ground() {
    let _lock = PHYSX_TEST_LOCK.lock().unwrap();
    let physics = level();
    let mut player = Player::at(0.0, 0.0, 0.0);
    player.idle(&physics, 10);
    // The player rises half a metre from a crouch: the tracked head goes up.
    player.head.y += 0.5;
    player.idle(&physics, 72);
    assert!(player.height().abs() < 0.02, "the body hangs {} m over the floor on invisible legs", player.height());
}
