use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;

use crate::locomotion::{Locomotion, LocomotionMode};
use crate::runtime::GameRuntime;

impl GameRuntime {
    pub fn world_head_transform(&self, player: PlayerId) -> (Vec3, Quat) {
        let head = self.rigs.get(&player).map(|r| r.head()).unwrap_or_default();
        (head.position, head.rotation)
    }

    pub(crate) fn find_spawn_point(&self) -> Option<(Vec3, f32)> {
        let obj = self.scene.objects.iter().find(|o| o.spawn_point.is_some())?;
        Some((
            obj.cuboid.position,
            Self::yaw_from_forward(obj.cuboid.rotation * Vec3::NEG_Z),
        ))
    }

    pub(crate) fn new_locomotion_at(spawn: Option<(Vec3, f32)>) -> Locomotion {
        let mut locomotion = Locomotion::new(LocomotionMode::Smooth);
        if let Some((position, yaw)) = spawn {
            locomotion.player_offset = position;
            locomotion.player_yaw = yaw;
        }
        locomotion
    }

    pub(crate) fn yaw_from_forward(fwd: Vec3) -> f32 {
        (-fwd.x).atan2(-fwd.z)
    }

    pub(crate) fn player_body_id(player: PlayerId) -> String {
        format!("__player_{}", player.0)
    }
}
