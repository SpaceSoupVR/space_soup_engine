use glam::{Quat, Vec3};
use std::collections::HashMap;

use crate::rig::{JointId, PlayerRig, Transform};

#[derive(Debug, Clone)]
pub struct Attachment {
    pub joint: JointId,
    pub offset_pos: Vec3,
    pub offset_rot: Quat,
    pub rigid: bool,
}

impl Attachment {
    pub fn rigid(joint: JointId) -> Self {
        Self {
            joint,
            offset_pos: Vec3::ZERO,
            offset_rot: Quat::IDENTITY,
            rigid: true,
        }
    }

    pub fn with_offset(joint: JointId, offset_pos: Vec3, offset_rot: Quat) -> Self {
        Self {
            joint,
            offset_pos,
            offset_rot,
            rigid: false,
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
    attachments: HashMap<String, Attachment>,
}

impl AttachmentTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(&mut self, object_id: &str, attachment: Attachment) {
        self.attachments.insert(object_id.to_string(), attachment);
    }

    pub fn detach(&mut self, object_id: &str) {
        self.attachments.remove(object_id);
    }

    pub fn is_attached(&self, object_id: &str) -> bool {
        self.attachments.contains_key(object_id)
    }

    pub fn object_for_joint(&self, joint: JointId) -> Option<&str> {
        self.attachments
            .iter()
            .find(|(_, att)| att.joint == joint)
            .map(|(id, _)| id.as_str())
    }

    pub fn resolve_all(&self, rig: &PlayerRig) -> Vec<(String, Transform)> {
        self.attachments
            .iter()
            .filter_map(|(id, att)| att.resolve(rig).map(|tf| (id.clone(), tf)))
            .collect()
    }

    pub fn resolve_all_with_visibility(&self, rig: &PlayerRig) -> Vec<(String, Option<Transform>)> {
        self.attachments
            .iter()
            .map(|(id, att)| (id.clone(), att.resolve(rig)))
            .collect()
    }
}
