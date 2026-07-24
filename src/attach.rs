use glam::{Quat, Vec3};
use std::collections::HashMap;

use space_soup_protocol::PlayerId;

use crate::rig::{JointId, PlayerRig, Transform};

#[derive(Debug, Clone)]
pub struct Attachment {
    pub joint: JointId,
    pub offset_pos: Vec3,
    pub offset_rot: Quat,
    pub rigid: bool,
    pub point: Option<String>,
}

impl Attachment {
    pub fn rigid(joint: JointId) -> Self {
        Self {
            joint,
            offset_pos: Vec3::ZERO,
            offset_rot: Quat::IDENTITY,
            rigid: true,
            point: None,
        }
    }

    pub fn with_offset(joint: JointId, offset_pos: Vec3, offset_rot: Quat) -> Self {
        Self {
            joint,
            offset_pos,
            offset_rot,
            rigid: false,
            point: None,
        }
    }

    pub fn with_grip_point(joint: JointId, offset_pos: Vec3, offset_rot: Quat, point: String) -> Self {
        Self {
            joint,
            offset_pos,
            offset_rot,
            rigid: false,
            point: Some(point),
        }
    }

    pub fn resolve(&self, rig: &PlayerRig) -> Option<Transform> {
        let joint_tf = rig.get(self.joint)?;
        if self.rigid && self.offset_pos == Vec3::ZERO && self.offset_rot == Quat::IDENTITY {
            Some(joint_tf)
        } else {
            Some(joint_tf.apply_offset(self.offset_pos, self.offset_rot))
        }
    }
}

#[derive(Debug, Default)]
pub struct AttachmentTable {
    attachments: HashMap<(String, PlayerId, JointId), Attachment>,
    primary: HashMap<String, (PlayerId, JointId)>,
}

impl AttachmentTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(&mut self, object_id: &str, player: PlayerId, attachment: Attachment) {
        let joint = attachment.joint;
        self.primary
            .entry(object_id.to_string())
            .or_insert((player, joint));
        self.attachments
            .insert((object_id.to_string(), player, joint), attachment);
    }

    pub fn detach(&mut self, object_id: &str) {
        self.attachments.retain(|(id, _, _), _| id != object_id);
        self.primary.remove(object_id);
    }

    pub fn detach_joint(&mut self, object_id: &str, player: PlayerId, joint: JointId) {
        self.attachments.remove(&(object_id.to_string(), player, joint));
        if self.primary.get(object_id) != Some(&(player, joint)) {
            return;
        }
        match self
            .attachments
            .keys()
            .find(|(id, _, _)| id == object_id)
            .map(|(_, p, j)| (*p, *j))
        {
            Some(remaining) => {
                self.primary.insert(object_id.to_string(), remaining);
            }
            None => {
                self.primary.remove(object_id);
            }
        }
    }

    pub fn remove_player(&mut self, player: PlayerId) {
        let affected: Vec<String> = self
            .attachments
            .keys()
            .filter(|(_, p, _)| *p == player)
            .map(|(id, _, _)| id.clone())
            .collect();
        self.attachments.retain(|(_, p, _), _| *p != player);
        for id in affected {
            if self.primary.get(&id).map(|&(p, _)| p) != Some(player) {
                continue;
            }
            match self
                .attachments
                .keys()
                .find(|(oid, _, _)| oid == &id)
                .map(|(_, p, j)| (*p, *j))
            {
                Some(remaining) => {
                    self.primary.insert(id, remaining);
                }
                None => {
                    self.primary.remove(&id);
                }
            }
        }
    }

    pub fn is_attached(&self, object_id: &str) -> bool {
        self.primary.contains_key(object_id)
    }

    pub fn object_for_joint(&self, player: PlayerId, joint: JointId) -> Option<&str> {
        self.attachments
            .iter()
            .find(|((_, p, j), _)| *p == player && *j == joint)
            .map(|((id, _, _), _)| id.as_str())
    }

    pub fn grip_point_at_joint(&self, player: PlayerId, joint: JointId) -> Option<(&str, &str)> {
        self.attachments
            .iter()
            .find(|((_, p, j), _)| *p == player && *j == joint)
            .and_then(|((id, _, _), att)| att.point.as_deref().map(|p| (id.as_str(), p)))
    }

    pub fn point_held_by_other(
        &self,
        object_id: &str,
        point: &str,
        exclude_player: PlayerId,
        exclude_joint: JointId,
    ) -> bool {
        self.attachments.iter().any(|((id, p, j), att)| {
            id == object_id
                && !(*p == exclude_player && *j == exclude_joint)
                && att.point.as_deref() == Some(point)
        })
    }

    pub fn resolve_all_with_visibility(
        &self,
        rigs: &HashMap<PlayerId, PlayerRig>,
    ) -> Vec<(String, Option<Transform>)> {
        self.primary
            .iter()
            .filter_map(|(id, (player, joint))| {
                let att = self.attachments.get(&(id.clone(), *player, *joint))?;
                let resolved = rigs.get(player).and_then(|rig| att.resolve(rig));
                Some((id.clone(), resolved))
            })
            .collect()
    }
}

