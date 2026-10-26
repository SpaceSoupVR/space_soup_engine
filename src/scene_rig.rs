use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::events::Hand;

pub const LEGACY_CURL_MAX_DEG: f32 = 80.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigAttachmentDef {
    pub joint: String,
    #[serde(default)]
    pub offset: [f32; 3],
}

fn identity_quat_arr() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn one_vec3_arr() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPoseDef {
    #[serde(default)]
    pub hand_offset_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub hand_offset_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
}

impl GripPoseDef {

    pub fn hand_pose(&self) -> avatar_ik::HandPose {
        avatar_ik::HandPose::from_curl(
            avatar_ik::HandCurl::from_finger_curl(&self.finger_curl, 0.0),
            LEGACY_CURL_MAX_DEG,
        )
    }
}

impl Default for GripPoseDef {
    fn default() -> Self {
        Self {
            hand_offset_pos: [0.0, 0.0, 0.0],
            hand_offset_rot: identity_quat_arr(),
            hand_offset_scale: one_vec3_arr(),
            finger_curl: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum GripKind {
    #[default]
    Snap,

    Free,

    Pinch,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GripPointDef {
    pub name: String,
    #[serde(default)]
    pub kind: GripKind,
    #[serde(default)]
    pub hand: Hand,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
    #[serde(default)]
    pub grab_range: Option<f32>,

    #[serde(default)]
    pub hand_offset_pos: Option<[f32; 3]>,
    #[serde(default)]
    pub hand_offset_rot: Option<[f32; 4]>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finger_pose: Option<avatar_ik::HandPose>,

    #[serde(default)]
    pub part: Option<String>,

    #[serde(default)]
    pub support: bool,
}

impl GripPointDef {

    pub fn hand_pose(&self) -> avatar_ik::HandPose {
        match &self.finger_pose {
            Some(p) => p.clamped(),
            None => avatar_ik::HandPose::from_curl(
                avatar_ik::HandCurl::from_finger_curl(&self.finger_curl, 0.0),
                LEGACY_CURL_MAX_DEG,
            ),
        }
    }

    pub fn base(
        &self,
        obj_pos: Vec3,
        obj_rot: Quat,
        part: Option<(Vec3, Quat)>,
    ) -> (Vec3, Quat) {
        match (self.part.as_ref(), part) {
            (Some(_), Some(p)) => p,
            _ => (obj_pos, obj_rot),
        }
    }

    pub fn anchor_world(
        &self,
        obj_pos: Vec3,
        obj_rot: Quat,
        part: Option<(Vec3, Quat)>,
    ) -> (Vec3, Quat) {
        let (bp, br) = self.base(obj_pos, obj_rot, part);
        (bp + br * Vec3::from(self.local_pos), br * Quat::from_array(self.local_rot))
    }

    pub fn hand_world(
        &self,
        obj_pos: Vec3,
        obj_rot: Quat,
        part: Option<(Vec3, Quat)>,
    ) -> (Vec3, Quat) {
        let (bp, br) = self.base(obj_pos, obj_rot, part);
        let local_pos = Vec3::from(self.hand_offset_pos.unwrap_or(self.local_pos));
        let local_rot = Quat::from_array(self.hand_offset_rot.unwrap_or(self.local_rot));
        (bp + br * local_pos, br * local_rot)
    }
}

fn default_slider_axis() -> [f32; 3] {
    [1.0, 0.0, 0.0]
}
fn default_slider_travel() -> f32 {
    0.02
}
fn default_slider_stiffness() -> f32 {
    400.0
}
fn default_slider_damping() -> f32 {
    20.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliderJointDef {
    pub parent: String,

    #[serde(default = "default_slider_axis")]
    pub axis: [f32; 3],

    #[serde(default = "default_slider_travel")]
    pub travel: f32,

    #[serde(default = "default_slider_stiffness")]
    pub spring_stiffness: f32,
    #[serde(default = "default_slider_damping")]
    pub spring_damping: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TerrainColliderDef {
    #[serde(default)]
    pub node_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketDef {
    pub name: String,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],

    #[serde(default)]
    pub part: Option<String>,
}
