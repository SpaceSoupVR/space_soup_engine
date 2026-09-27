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

    pub move_speed: f32,
    pub snap_turn_deg: f32,
    pub turn_speed_deg_per_sec: f32,
    pub teleport_range: f32,
    pub max_climb_angle_deg: f32,

    is_teleport_aiming: bool,
    last_turn_stick: f32,
    /// Where the body stood when the last frame finished. See `apply_collision`.
    last_body_xz: Option<(f32, f32)>,
    /// Whether this frame's `update` teleported, so walls do not block it.
    teleported: bool,
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
            last_body_xz: None,
            teleported: false,
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
                        self.teleported = true;
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

    /// Where this frame's movement started, for [`Locomotion::apply_collision`].
    ///
    /// Take it BEFORE `update`. `head` is the head's WORLD position as built
    /// from THIS locomotion's offset and yaw this frame -- a headset's own rig.
    ///
    /// `None` collides the rig origin. That is right for anything moving a
    /// player whose head it did not place: the server simulating a client that
    /// sends no pose receives a rig built from the CLIENT's offset, and
    /// subtracting its own offset from that invents a head that runs away from
    /// the body -- a test walked straight through a wall that way.
    pub fn collision_start(&self, head: Option<Vec3>) -> CollisionStart {
        let head_rel = head
            .map(|h| Vec3::new(h.x - self.player_offset.x, 0.0, h.z - self.player_offset.z))
            .unwrap_or(Vec3::ZERO);
        CollisionStart {
            offset: self.player_offset,
            yaw: self.player_yaw,
            head_rel,
        }
    }

    /// Stops the player's BODY at walls, then stands it on the ground under it.
    ///
    /// The body is the floor point under the HEAD, not the rig origin. The
    /// headset tracks in stage space, so the origin is the centre of the play
    /// area and the head can be a metre or more from it: colliding the origin
    /// stopped an invisible point short of the wall while the camera walked on
    /// through it. Unity's XR character controller and Meta's locomotors centre
    /// the collider on the head for the same reason.
    ///
    /// Walking or leaning into a wall in the real room is stopped too. The rig
    /// is pushed back by exactly the overlap, measured from where the body stood
    /// at the end of the last frame -- so a wall is a barrier however the player
    /// got there, not only when the stick took them.
    ///
    /// Shared by GameRuntime::update (the authoritative server simulation) and
    /// any client that runs its own movement, so both collide identically.
    pub fn apply_collision(&mut self, physics: &PhysicsWorld, start: CollisionStart) {
        // A snap turn rotates the play area about the rig origin, which carries
        // the head round with it.
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
        self.apply_ground_follow(physics, start, head_rel);
        let body = self.player_offset + head_rel;
        self.last_body_xz = Some((body.x, body.z));
    }

    fn apply_wall_collision(&mut self, physics: &PhysicsWorld, from: Vec3, head_rel: Vec3) {
        // A teleport is not a walk: whatever lies between is not in the way.
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
        // Move the whole rig by the overlap, so the BODY ends at `stopped`.
        self.player_offset.x += stopped.x - curr.x;
        self.player_offset.z += stopped.z - curr.z;
    }

    fn apply_ground_follow(&mut self, physics: &PhysicsWorld, start: CollisionStart, head_rel: Vec3) {
        if self.mode == LocomotionMode::Disabled {
            return;
        }

        // Under the body: standing inside a building with the play area's
        // centre outside it must put you on the building's floor.
        //
        // FROM A STEP ABOVE THE FEET, not from 3 m. The ground is the first
        // thing the ray meets going down, so it must start below every ceiling
        // the player can stand under. From 3 m it started ABOVE the roof of any
        // room lower than that -- the 2.6 m hallway added to test_room -- and
        // stood the player on the roof the moment they walked in (headset,
        // 2026-09-24). The halls only worked because their ceilings are at
        // 3.1 m. A step's height still climbs stairs, kerbs and any slope
        // `max_climb_angle_deg` allows, since a frame's walk rises centimetres.
        let body = self.player_offset + head_rel;
        let probe_origin = Vec3::new(body.x, self.player_offset.y + MAX_STEP_UP, body.z);
        let Some((hit_point, normal)) = physics.raycast_down(probe_origin, 50.0) else {
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

/// Where a frame's movement started. See [`Locomotion::collision_start`].
#[derive(Debug, Clone, Copy)]
pub struct CollisionStart {
    offset: Vec3,
    yaw: f32,
    /// The head's floor position relative to the rig origin, in world axes.
    head_rel: Vec3,
}

/// The highest ledge a walk steps up onto, in metres: the ground probe starts
/// this far above the feet. Above a stair riser (~0.18 m) and a kerb, below
/// any ceiling -- a lower ceiling than this is not a room anyone stands in.
pub const MAX_STEP_UP: f32 = 0.5;

/// Further than a head can move between two frames. A body found this far from
/// where the last frame left it was put there -- a respawn, a scene change, a
/// server correction -- and is not walked from.
const MAX_HEAD_JUMP: f32 = 1.0;

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

