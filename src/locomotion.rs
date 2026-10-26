use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::events::Hand;
use crate::rig::PlayerRig;
use crate::rigid_physics::PhysicsWorld;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LocomotionMode {
    Teleport,
    #[default]
    Smooth,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TurnMode {
    Smooth,
    #[default]
    Snap,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocomotionInput {
    pub move_stick: (f32, f32),
    pub turn_stick_x: f32,
    pub teleport_pressed: bool,
    pub teleport_released: bool,
    pub teleport_hand: Hand,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TeleportTarget {
    pub position: Vec3,
    pub valid: bool,
}

pub struct Locomotion {
    pub mode: LocomotionMode,
    pub turn_mode: TurnMode,
    pub player_offset: Vec3,
    pub player_yaw: f32,

    /// The walk, metres a second (`LocomotionSettings::walk_speed`).
    pub move_speed: f32,
    /// The run, metres a second, while running (`set_running`;
    /// `LocomotionSettings::run_speed`).
    pub run_speed: f32,
    pub snap_turn_deg: f32,
    pub turn_speed_deg_per_sec: f32,
    pub teleport_range: f32,
    pub max_climb_angle_deg: f32,

    is_teleport_aiming: bool,
    last_turn_stick: f32,

    last_body_xz: Option<(f32, f32)>,

    teleported: bool,
    /// Running: until the stick comes back to the middle. See `toggle_run`.
    running: bool,
    /// The speed the walk is at now, eased toward the walk's or the run's
    /// (`SPEED_EASE_SECONDS`), so starting to run is not a jolt.
    speed_now: f32,
}

/// How long the speed takes to close most of its gap to the walk's or the
/// run's, seconds (a time constant): a stride, so a run starts as a
/// quickening, not a jump -- the kind of acceleration that unsettles a
/// stomach in a headset is a sudden one.
pub const SPEED_EASE_SECONDS: f32 = 0.2;

/// THE WALK AND THE RUN, per game and per scene: `"locomotion"` in the
/// game's `manifest.json`, and `"locomotion_scenes": {"<scene>": {...}}`
/// there for one scene's own. Every key optional; absent, the defaults.
/// Data, because another game wants another pace.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocomotionSettings {
    /// Metres a second, the stick pushed: a brisk walk (people walk at
    /// about 1.4).
    pub walk_speed: f32,
    /// Metres a second while running (the left stick's click).
    pub run_speed: f32,
}

impl Default for LocomotionSettings {
    fn default() -> Self {
        Self { walk_speed: 1.6, run_speed: 3.0 }
    }
}

impl LocomotionSettings {
    /// The game's settings for `scene`, from `game_dir/manifest.json`:
    /// the game's `"locomotion"`, then the scene's own over it. A missing
    /// or unreadable file, or a key out of range, keeps the defaults.
    pub fn load(game_dir: &std::path::Path, scene: &str) -> Self {
        std::fs::read_to_string(game_dir.join("manifest.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .map_or_else(Self::default, |v| Self::from_manifest(&v, scene))
    }

    /// [`Self::load`] from the manifest's JSON.
    #[must_use]
    pub fn from_manifest(manifest: &serde_json::Value, scene: &str) -> Self {
        let mut out = Self::default();
        for layer in [manifest.get("locomotion"), manifest.get("locomotion_scenes").and_then(|s| s.get(scene))].into_iter().flatten() {
            let speed = |key: &str| layer.get(key).and_then(serde_json::Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite() && *v > 0.0 && *v <= 12.0);
            if let Some(v) = speed("walk_speed") {
                out.walk_speed = v;
            }
            if let Some(v) = speed("run_speed") {
                out.run_speed = v;
            }
        }
        out
    }
}

impl Default for Locomotion {
    fn default() -> Self {
        Self {
            mode: LocomotionMode::Smooth,
            turn_mode: TurnMode::Snap,
            player_offset: Vec3::ZERO,
            player_yaw: 0.0,
            move_speed: LocomotionSettings::default().walk_speed,
            run_speed: LocomotionSettings::default().run_speed,
            snap_turn_deg: 45.0,
            turn_speed_deg_per_sec: 90.0,
            teleport_range: 5.0,
            max_climb_angle_deg: 45.0,
            is_teleport_aiming: false,
            last_turn_stick: 0.0,
            last_body_xz: None,
            teleported: false,
            running: false,
            speed_now: 0.0,
        }
    }
}

impl Locomotion {
    #[must_use]
    pub fn new(mode: LocomotionMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    /// The walk and the run from the game's settings.
    pub fn apply_settings(&mut self, settings: &LocomotionSettings) {
        self.move_speed = settings.walk_speed;
        self.run_speed = settings.run_speed;
    }

    /// RUN, or walk again: a click of the stick. A run lasts until the stick
    /// comes back to the middle (the player stops), as in most games with a
    /// stick to walk with, so it never has to be clicked off.
    pub fn toggle_run(&mut self) {
        self.running = !self.running;
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_mode(&mut self, mode: LocomotionMode) {
        self.mode = mode;
        self.is_teleport_aiming = false;
    }

    #[must_use]
    pub fn is_teleport_aiming(&self) -> bool {
        self.is_teleport_aiming
    }

    pub fn update(
        &mut self,
        dt: f32,
        input: &LocomotionInput,
        rig: &PlayerRig,
        teleport_target: Option<TeleportTarget>,
    ) {
        self.teleported = false;
        if self.mode != LocomotionMode::Disabled {
            match self.turn_mode {
                TurnMode::Smooth => self.update_smooth_turn(dt, input),
                TurnMode::Snap => self.update_snap_turn(input),
            }
        }

        match self.mode {
            LocomotionMode::Smooth => self.update_smooth(dt, input, rig),
            LocomotionMode::Teleport => self.update_teleport(input, teleport_target),
            LocomotionMode::Disabled => {}
        }
    }

    fn update_smooth_turn(&mut self, dt: f32, input: &LocomotionInput) {
        let x = input.turn_stick_x;
        const DEADZONE: f32 = 0.15;
        if x.abs() < DEADZONE {
            return;
        }
        let magnitude = (x.abs() - DEADZONE) / (1.0 - DEADZONE);
        self.player_yaw -= x.signum() * magnitude * self.turn_speed_deg_per_sec.to_radians() * dt;
    }

    fn update_snap_turn(&mut self, input: &LocomotionInput) {
        let x = input.turn_stick_x;
        let threshold = 0.6;

        if x.abs() > threshold && self.last_turn_stick.abs() <= threshold {
            let dir = if x > 0.0 { -1.0 } else { 1.0 };
            self.player_yaw += dir * self.snap_turn_deg.to_radians();
        }
        self.last_turn_stick = x;
    }

    fn update_smooth(&mut self, dt: f32, input: &LocomotionInput, rig: &PlayerRig) {
        let (sx, sy) = input.move_stick;
        if sx.abs() < 0.08 && sy.abs() < 0.08 {
            self.running = false;
            self.speed_now = 0.0;
            return;
        }
        let target = if self.running { self.run_speed } else { self.move_speed };
        self.speed_now = if self.speed_now <= 0.0 {
            target
        } else {
            self.speed_now + (target - self.speed_now) * (1.0 - (-dt.max(0.0) / SPEED_EASE_SECONDS).exp())
        };

        let head_rot = rig.head().rotation;
        let fwd = head_rot * Vec3::new(0.0, 0.0, -1.0);

        let mut heading = Vec3::new(fwd.x, 0.0, fwd.z);
        if heading.length_squared() < 1e-4 {
            let up = head_rot * Vec3::Y;
            heading = Vec3::new(up.x, 0.0, up.z);
        }

        let forward = heading.normalize_or_zero();
        let right = Vec3::new(-forward.z, 0.0, forward.x);

        let move_dir = (forward * sy + right * sx).normalize_or_zero();
        self.player_offset += move_dir * self.speed_now * dt;
    }

    fn update_teleport(&mut self, input: &LocomotionInput, target: Option<TeleportTarget>) {
        if input.teleport_pressed {
            self.is_teleport_aiming = true;
        }
        if input.teleport_released {
            if self.is_teleport_aiming {
                if let Some(t) = target {
                    if t.valid {
                        self.player_offset = t.position;
                        self.teleported = true;
                    }
                }
            }
            self.is_teleport_aiming = false;
        }
    }

    #[must_use]
    pub fn apply_to_head(&self, tracked_position: Vec3, tracked_rotation: Quat) -> (Vec3, Quat) {
        let yaw_rot = Quat::from_rotation_y(self.player_yaw);
        let position = self.player_offset + yaw_rot * tracked_position;
        let rotation = yaw_rot * tracked_rotation;
        (position, rotation)
    }

    #[must_use]
    pub fn collision_start(&self, head: Option<Vec3>) -> CollisionStart {
        let head_rel = head
            .map_or(Vec3::ZERO, |h| Vec3::new(h.x - self.player_offset.x, 0.0, h.z - self.player_offset.z));
        CollisionStart {
            offset: self.player_offset,
            yaw: self.player_yaw,
            head_rel,
        }
    }

    pub fn apply_collision(&mut self, physics: &PhysicsWorld, start: CollisionStart) {

        let head_rel = Quat::from_rotation_y(self.player_yaw - start.yaw) * start.head_rel;
        let start_body = start.offset + start.head_rel;
        let from = match self.last_body_xz {
            Some((x, z))
                if Vec3::new(x - start_body.x, 0.0, z - start_body.z).length() <= MAX_HEAD_JUMP =>
            {
                Vec3::new(x, 0.0, z)
            }
            _ => start_body,
        };
        self.apply_wall_collision(physics, from, head_rel);
        self.apply_ground_follow(physics, start, from, head_rel);
        let body = self.player_offset + head_rel;
        self.last_body_xz = Some((body.x, body.z));
    }

    fn apply_wall_collision(&mut self, physics: &PhysicsWorld, from: Vec3, head_rel: Vec3) {

        if self.mode == LocomotionMode::Disabled || self.teleported {
            return;
        }

        const PLAYER_RADIUS: f32 = 0.25;
        const PROBE_HEIGHT: f32 = 1.0;

        let y = self.player_offset.y + PROBE_HEIGHT;
        let body = self.player_offset + head_rel;
        let prev = Vec3::new(from.x, y, from.z);
        let curr = Vec3::new(body.x, y, body.z);
        let delta = curr - prev;
        let dist = delta.length();
        if dist < 1e-5 {
            return;
        }
        let dir = delta / dist;

        let Some((hit_point, _normal)) = physics.raycast(prev, dir, dist + PLAYER_RADIUS) else {
            return;
        };

        let clear_dist = (prev.distance(hit_point) - PLAYER_RADIUS).max(0.0);
        let stopped = prev + dir * clear_dist;

        self.player_offset.x += stopped.x - curr.x;
        self.player_offset.z += stopped.z - curr.z;
    }

    fn apply_ground_follow(&mut self, physics: &PhysicsWorld, start: CollisionStart, from: Vec3, head_rel: Vec3) {
        if self.mode == LocomotionMode::Disabled {
            return;
        }

        let body = self.player_offset + head_rel;
        let probe_origin = Vec3::new(body.x, self.player_offset.y + MAX_STEP_UP, body.z);
        let hit = physics.raycast_down(probe_origin, 50.0);

        if wades_out_of_depth(physics, from, body, probe_origin.y, hit.map(|(p, _)| p.y)) {

            self.player_offset.x += from.x - body.x;
            self.player_offset.z += from.z - body.z;
            return;
        }
        let Some((hit_point, normal)) = hit else {
            return;
        };

        let slope_deg = normal.dot(Vec3::Y).clamp(-1.0, 1.0).acos().to_degrees();
        if slope_deg <= self.max_climb_angle_deg {
            self.player_offset.y = hit_point.y;
        } else {
            self.player_offset.x = start.offset.x;
            self.player_offset.z = start.offset.z;
        }
    }
}

fn wades_out_of_depth(physics: &PhysicsWorld, from: Vec3, to: Vec3, probe_y: f32, to_ground: Option<f32>) -> bool {
    let depth_at = |p: Vec3, ground: Option<f32>| physics.water_wade_excess_at(p.x, p.z, ground.unwrap_or(f32::NEG_INFINITY));
    let Some(depth) = depth_at(to, to_ground) else {
        return false;
    };
    if depth <= 0.0 {
        return false;
    }
    let from_ground = physics.raycast_down(Vec3::new(from.x, probe_y, from.z), 50.0).map(|(p, _)| p.y);
    depth > depth_at(from, from_ground).unwrap_or(f32::NEG_INFINITY)
}

#[derive(Debug, Clone, Copy)]
pub struct CollisionStart {
    offset: Vec3,
    yaw: f32,

    head_rel: Vec3,
}

pub const MAX_STEP_UP: f32 = 0.5;

pub const MAX_WADE_DEPTH: f32 = crate::water::DEFAULT_WADE_DEPTH;

const MAX_HEAD_JUMP: f32 = 1.0;

#[cfg(test)]
mod turn_test {
    use super::*;
    use crate::rig::PlayerRig;

    /// THE WALK AND THE RUN: the walk at once from rest, the run eased in by
    /// a click and ended by stopping; the speeds from the game's manifest
    /// and a scene's own, out-of-range values ignored.
    #[test]
    fn a_click_runs_until_the_stick_is_let_go_and_the_paces_are_data() {
        let mut l = Locomotion::new(LocomotionMode::Smooth);
        let rig = PlayerRig::new();
        let push = LocomotionInput { move_stick: (0.0, 1.0), ..LocomotionInput::default() };
        let rest = LocomotionInput::default();
        let dt = 1.0 / 36.0;
        let step = |l: &mut Locomotion, input: &LocomotionInput| {
            let before = l.player_offset;
            l.update(dt, input, &rig, None);
            (l.player_offset - before).length() / dt
        };
        assert!((step(&mut l, &push) - 1.6).abs() < 1e-4, "the walk starts at once");
        l.toggle_run();
        let first = step(&mut l, &push);
        assert!(first > 1.6 && first < 2.0, "the run eases in: {first}");
        for _ in 0..36 {
            step(&mut l, &push);
        }
        assert!((step(&mut l, &push) - 3.0).abs() < 0.01, "a second on, running");
        step(&mut l, &rest);
        assert!(!l.is_running(), "stopping ends the run");
        assert!((step(&mut l, &push) - 1.6).abs() < 1e-4);
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"locomotion": {"walk_speed": 1.8, "run_speed": 3.2}, "locomotion_scenes": {"cave": {"run_speed": 2.2}, "bad": {"walk_speed": -1, "run_speed": "fast"}}}"#,
        )
        .unwrap();
        assert_eq!(LocomotionSettings::from_manifest(&manifest, "test_room"), LocomotionSettings { walk_speed: 1.8, run_speed: 3.2 });
        assert_eq!(LocomotionSettings::from_manifest(&manifest, "cave"), LocomotionSettings { walk_speed: 1.8, run_speed: 2.2 });
        assert_eq!(LocomotionSettings::from_manifest(&manifest, "bad"), LocomotionSettings { walk_speed: 1.8, run_speed: 3.2 });
        assert_eq!(LocomotionSettings::from_manifest(&serde_json::json!({}), "x"), LocomotionSettings::default());
    }

    fn turn_input(turn_stick_x: f32) -> LocomotionInput {
        LocomotionInput {
            turn_stick_x,
            ..LocomotionInput::default()
        }
    }

    fn smooth_turn_loco() -> Locomotion {
        Locomotion {
            turn_mode: TurnMode::Smooth,
            ..Locomotion::new(LocomotionMode::Smooth)
        }
    }

    #[test]
    fn default_turn_mode_is_snap_at_45_degrees() {
        let loco = Locomotion::default();
        assert_eq!(loco.turn_mode, TurnMode::Snap);
        assert_eq!(loco.snap_turn_deg, 45.0);
    }

    #[test]
    fn snap_turn_moves_by_exactly_snap_turn_deg_regardless_of_movement_mode() {
        let mut loco = Locomotion::new(LocomotionMode::Smooth);
        assert_eq!(loco.turn_mode, TurnMode::Snap);
        let rig = PlayerRig::new();

        loco.update(1.0 / 90.0, &turn_input(1.0), &rig, None);
        assert!(
            (loco.player_yaw.to_degrees().abs() - 45.0).abs() < 1e-3,
            "a single stick flick should snap by exactly 45 degrees, got {}",
            loco.player_yaw.to_degrees()
        );

        for _ in 0..30 {
            loco.update(1.0 / 90.0, &turn_input(1.0), &rig, None);
        }
        assert!(
            (loco.player_yaw.to_degrees().abs() - 45.0).abs() < 1e-3,
            "holding the stick should not keep accumulating snap turns, got {}",
            loco.player_yaw.to_degrees()
        );
    }

    #[test]
    fn holding_the_stick_keeps_turning_every_frame_in_smooth_turn_mode() {
        let mut loco = smooth_turn_loco();
        let rig = PlayerRig::new();
        let input = turn_input(1.0);

        loco.update(1.0 / 90.0, &input, &rig, None);
        let yaw_after_one_frame = loco.player_yaw;
        assert_ne!(yaw_after_one_frame, 0.0, "a single frame of full stick should already turn");

        for _ in 0..89 {
            loco.update(1.0 / 90.0, &input, &rig, None);
        }
        assert!(
            loco.player_yaw.abs() > yaw_after_one_frame.abs() * 2.0,
            "holding the stick for a full second should keep accumulating yaw, not stop after the first frame (got {})",
            loco.player_yaw
        );
    }

    #[test]
    fn stick_right_turns_right_matching_snap_turn_direction() {
        let mut smooth = smooth_turn_loco();
        let rig = PlayerRig::new();
        smooth.update(1.0 / 90.0, &turn_input(1.0), &rig, None);

        let mut snap = Locomotion::new(LocomotionMode::Teleport);
        snap.update(1.0 / 90.0, &turn_input(1.0), &rig, None);

        assert!(smooth.player_yaw < 0.0, "stick right should turn right (negative yaw), got {}", smooth.player_yaw);
        assert!(snap.player_yaw < 0.0, "stick right should turn right (negative yaw), got {}", snap.player_yaw);
    }

    #[test]
    fn small_stick_deflection_within_deadzone_does_not_turn_in_smooth_turn_mode() {
        let mut loco = smooth_turn_loco();
        let rig = PlayerRig::new();
        loco.update(1.0 / 90.0, &turn_input(0.05), &rig, None);
        assert_eq!(loco.player_yaw, 0.0, "deflection inside the deadzone shouldn't turn at all");
    }
}
