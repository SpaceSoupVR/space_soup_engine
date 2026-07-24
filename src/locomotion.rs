use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::events::Hand;
use crate::rig::PlayerRig;

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

    pub move_speed: f32,
    pub snap_turn_deg: f32,
    pub turn_speed_deg_per_sec: f32,
    pub teleport_range: f32,
    pub max_climb_angle_deg: f32,

    is_teleport_aiming: bool,
    last_turn_stick: f32,
}

impl Default for Locomotion {
    fn default() -> Self {
        Self {
            mode: LocomotionMode::Smooth,
            turn_mode: TurnMode::Snap,
            player_offset: Vec3::ZERO,
            player_yaw: 0.0,
            move_speed: 1.6,
            snap_turn_deg: 45.0,
            turn_speed_deg_per_sec: 90.0,
            teleport_range: 5.0,
            max_climb_angle_deg: 45.0,
            is_teleport_aiming: false,
            last_turn_stick: 0.0,
        }
    }
}

impl Locomotion {
    pub fn new(mode: LocomotionMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    pub fn set_mode(&mut self, mode: LocomotionMode) {
        self.mode = mode;
        self.is_teleport_aiming = false;
    }

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
            return;
        }

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
        self.player_offset += move_dir * self.move_speed * dt;
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
                    }
                }
            }
            self.is_teleport_aiming = false;
        }
    }

    pub fn apply_to_head(&self, tracked_position: Vec3, tracked_rotation: Quat) -> (Vec3, Quat) {
        let yaw_rot = Quat::from_rotation_y(self.player_yaw);
        let position = self.player_offset + yaw_rot * tracked_position;
        let rotation = yaw_rot * tracked_rotation;
        (position, rotation)
    }
}

#[cfg(test)]
mod turn_test {
    use super::*;
    use crate::rig::PlayerRig;

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

