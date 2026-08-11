use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::events::Hand;

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
pub enum GripKind {
    Snap,

    Free,

    Pinch,
}

impl Default for GripKind {
    fn default() -> Self {
        Self::Snap
    }
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

    // The hand's grip transform, authored independently of the reach anchor (local_pos/
    // local_rot). When absent, the hand sits on the anchor. Lets the designer place the
    // reach zone on one spot (e.g. the trigger) while the hand grips a few cm away.
    #[serde(default)]
    pub hand_offset_pos: Option<[f32; 3]>,
    #[serde(default)]
    pub hand_offset_rot: Option<[f32; 4]>,

    /// Anchor this grip to a model part, so the hand rides that part's animated
    /// pose instead of the object's origin. Mirrors `SocketDef::part`.
    ///
    /// A charging handle grip has to be on the handle -- once the bolt is 3 cm
    /// back, a grip measured from the receiver origin leaves the hand floating
    /// inside the gun while the part it is supposedly pulling has moved away.
    ///
    /// Resolved on the headset, which is the only place a posed part exists, and
    /// falls back to the object origin when that part has not been reported --
    /// better a hand at the pivot than a hand at the world origin.
    ///
    /// Intended for grips that pose a hand onto a moving part (a pull grip), not
    /// for the grip that carries the object: carrying from a part would feed the
    /// object's own pose back into the part that derives from it.
    #[serde(default)]
    pub part: Option<String>,
}

impl GripPointDef {
    /// Where this grip's local offsets are measured from.
    ///
    /// `part` is the posed world transform of `self.part`, which only the client
    /// can supply -- it is the only place a skinned pose exists. Passing None for
    /// a part-scoped grip falls back to the object, so a mesh that has never been
    /// posed puts the grip at the pivot rather than at the world origin.
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

    /// World transform of the reach anchor -- what a hand has to get near to grab.
    pub fn anchor_world(
        &self,
        obj_pos: Vec3,
        obj_rot: Quat,
        part: Option<(Vec3, Quat)>,
    ) -> (Vec3, Quat) {
        let (bp, br) = self.base(obj_pos, obj_rot, part);
        (bp + br * Vec3::from(self.local_pos), br * Quat::from_array(self.local_rot))
    }

    /// World transform the hand itself is drawn at.
    ///
    /// Honours `hand_offset_*` when authored, so the reach zone and the hand can
    /// sit apart -- reach at the trigger, hand a few cm down the grip.
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

// An object-to-object attach point (magazine well, battery bay, etc.) -- distinct from
// GripPointDef, which is always hand-to-object. A child snapped into a socket is kinematically
// carried by the parent's transform each frame rather than by a player rig joint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketDef {
    pub name: String,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],

    /// Anchor this socket to a model part, so it rides that part's animated pose
    /// instead of the object's origin.
    ///
    /// An ejection port has to be where the port actually is with the bolt back;
    /// a muzzle has to follow the barrel. Without this, anything spawned at a
    /// socket appears at the object pivot and stays there while the mechanism
    /// moves around it.
    ///
    /// `local_pos`/`local_rot` are then relative to that part, not the object.
    #[serde(default)]
    pub part: Option<String>,
}
