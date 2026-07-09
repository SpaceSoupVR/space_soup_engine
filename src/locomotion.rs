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
    pub player_offset: Vec3,
    pub player_yaw: f32,

    pub move_speed: f32,
    pub snap_turn_deg: f32,
    pub teleport_range: f32,
    pub max_climb_angle_deg: f32,

    is_teleport_aiming: bool,
    last_turn_stick: f32,
}

impl Default for Locomotion {
    fn default() -> Self {
        Self {
            mode: LocomotionMode::Smooth,
            player_offset: Vec3::ZERO,
            player_yaw: 0.0,
            move_speed: 1.6,
            snap_turn_deg: 30.0,
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
        self.update_snap_turn(input);

        match self.mode {
            LocomotionMode::Smooth => self.update_smooth(dt, input, rig),
            LocomotionMode::Teleport => self.update_teleport(input, teleport_target),
            LocomotionMode::Disabled => {}
        }
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
