use glam::{Vec3, Quat};

use crate::events::Hand;
use crate::rig::PlayerRig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocomotionMode {
    Teleport,
    Smooth,
    Disabled,
}

impl Default for LocomotionMode {
    fn default() -> Self { LocomotionMode::Smooth }
}

#[derive(Debug, Clone, Default)]
pub struct LocomotionInput {
    pub move_stick: (f32, f32),
    pub turn_stick_x: f32,
    pub teleport_pressed: bool,
    pub teleport_released: bool,
    pub teleport_hand: Hand,
    pub jump_pressed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TeleportTarget {
    pub position: Vec3,
    pub valid:    bool,
}

pub struct Locomotion {
    pub mode:  LocomotionMode,
    pub player_offset: Vec3,
    pub player_yaw: f32,

    pub move_speed:      f32,
    pub snap_turn_deg:   f32,
    pub teleport_range:  f32,

    pub jump_speed:   f32,
    pub gravity:      f32,

    is_teleport_aiming: bool,
    last_turn_stick:    f32,
    vertical_velocity:  f32,
}

impl Default for Locomotion {
    fn default() -> Self {
        Self {
            mode:          LocomotionMode::Smooth,
            player_offset: Vec3::ZERO,
            player_yaw:    0.0,
            move_speed:    1.6,
            snap_turn_deg: 30.0,
            teleport_range: 5.0,
            jump_speed:    3.0,
            gravity:       9.81,
            is_teleport_aiming: false,
            last_turn_stick:    0.0,
            vertical_velocity:  0.0,
        }
    }
}

impl Locomotion {
    pub fn new(mode: LocomotionMode) -> Self {
        Self { mode, ..Default::default() }
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
        dt:    f32,
        input: &LocomotionInput,
        rig:   &PlayerRig,
        teleport_target: Option<TeleportTarget>,
    ) {
        self.update_snap_turn(input, rig);

        match self.mode {
            LocomotionMode::Smooth   => self.update_smooth(dt, input, rig),
            LocomotionMode::Teleport => self.update_teleport(input, teleport_target),
            LocomotionMode::Disabled => {}
        }

        self.update_jump(dt, input);
    }

    fn update_jump(&mut self, dt: f32, input: &LocomotionInput) {
        // Vertical motion lives entirely in player_offset.y; ground is y = 0.
        // Horizontal locomotion never touches y, so this channel is ours alone.
        let grounded = self.player_offset.y <= 0.0 && self.vertical_velocity <= 0.0;

        if input.jump_pressed && grounded {
            self.vertical_velocity = self.jump_speed;
        }

        self.vertical_velocity -= self.gravity * dt;
        self.player_offset.y   += self.vertical_velocity * dt;

        // Land: clamp to the ground and kill downward velocity.
        if self.player_offset.y <= 0.0 {
            self.player_offset.y   = 0.0;
            self.vertical_velocity = 0.0;
        }
    }

    fn update_snap_turn(&mut self, input: &LocomotionInput, rig: &PlayerRig) {
        let x = input.turn_stick_x;
        let threshold = 0.6;

        if x.abs() > threshold && self.last_turn_stick.abs() <= threshold {
            let dir   = if x > 0.0 { -1.0 } else { 1.0 };
            let delta = dir * self.snap_turn_deg.to_radians();

            // Pivot the turn about the head, not the playspace origin. The head
            // world position is `player_offset + R(yaw) * head_pos`, so changing
            // yaw alone swings the player in a circle. Compensate player_offset
            // to keep the head's world position fixed across the turn.
            let head_pos = rig.head().position;
            let old      = Quat::from_rotation_y(self.player_yaw);
            let new      = Quat::from_rotation_y(self.player_yaw + delta);
            self.player_offset += (old * head_pos) - (new * head_pos);

            self.player_yaw += delta;
        }
        self.last_turn_stick = x;
    }

    fn update_smooth(&mut self, dt: f32, input: &LocomotionInput, rig: &PlayerRig) {
        let (sx, sy) = input.move_stick;
        if sx.abs() < 0.08 && sy.abs() < 0.08 { return; }

        let head = rig.head();
        let (_, head_yaw, _) = head.rotation.to_euler(glam::EulerRot::YXZ);
        let total_yaw = head_yaw + self.player_yaw;
        let facing = Quat::from_rotation_y(total_yaw);

        let forward = facing * Vec3::new(0.0, 0.0, -1.0);
        let right   = facing * Vec3::new(1.0, 0.0, 0.0);

        let move_dir = (forward * sy + right * -sx).normalize_or_zero();
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
