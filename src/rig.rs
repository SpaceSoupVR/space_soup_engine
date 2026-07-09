use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::events::Hand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FingerJoint {
    Palm,
    Wrist,
    ThumbMeta,
    ThumbProx,
    ThumbDist,
    ThumbTip,
    IndexMeta,
    IndexProx,
    IndexInter,
    IndexDist,
    IndexTip,
    MiddleMeta,
    MiddleProx,
    MiddleInter,
    MiddleDist,
    MiddleTip,
    RingMeta,
    RingProx,
    RingInter,
    RingDist,
    RingTip,
    LittleMeta,
    LittleProx,
    LittleInter,
    LittleDist,
    LittleTip,
}

impl FingerJoint {
    pub const ALL: [FingerJoint; 26] = [
        FingerJoint::Palm,
        FingerJoint::Wrist,
        FingerJoint::ThumbMeta,
        FingerJoint::ThumbProx,
        FingerJoint::ThumbDist,
        FingerJoint::ThumbTip,
        FingerJoint::IndexMeta,
        FingerJoint::IndexProx,
        FingerJoint::IndexInter,
        FingerJoint::IndexDist,
        FingerJoint::IndexTip,
        FingerJoint::MiddleMeta,
        FingerJoint::MiddleProx,
        FingerJoint::MiddleInter,
        FingerJoint::MiddleDist,
        FingerJoint::MiddleTip,
        FingerJoint::RingMeta,
        FingerJoint::RingProx,
        FingerJoint::RingInter,
        FingerJoint::RingDist,
        FingerJoint::RingTip,
        FingerJoint::LittleMeta,
        FingerJoint::LittleProx,
        FingerJoint::LittleInter,
        FingerJoint::LittleDist,
        FingerJoint::LittleTip,
    ];

    pub fn from_index(i: usize) -> Option<FingerJoint> {
        Self::ALL.get(i).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            FingerJoint::Palm => "palm",
            FingerJoint::Wrist => "wrist",
            FingerJoint::ThumbMeta => "thumb_meta",
            FingerJoint::ThumbProx => "thumb_prox",
            FingerJoint::ThumbDist => "thumb_dist",
            FingerJoint::ThumbTip => "thumb_tip",
            FingerJoint::IndexMeta => "index_meta",
            FingerJoint::IndexProx => "index_prox",
            FingerJoint::IndexInter => "index_inter",
            FingerJoint::IndexDist => "index_dist",
            FingerJoint::IndexTip => "index_tip",
            FingerJoint::MiddleMeta => "middle_meta",
            FingerJoint::MiddleProx => "middle_prox",
            FingerJoint::MiddleInter => "middle_inter",
            FingerJoint::MiddleDist => "middle_dist",
            FingerJoint::MiddleTip => "middle_tip",
            FingerJoint::RingMeta => "ring_meta",
            FingerJoint::RingProx => "ring_prox",
            FingerJoint::RingInter => "ring_inter",
            FingerJoint::RingDist => "ring_dist",
            FingerJoint::RingTip => "ring_tip",
            FingerJoint::LittleMeta => "little_meta",
            FingerJoint::LittleProx => "little_prox",
            FingerJoint::LittleInter => "little_inter",
            FingerJoint::LittleDist => "little_dist",
            FingerJoint::LittleTip => "little_tip",
        }
    }

    pub fn from_name(s: &str) -> Option<FingerJoint> {
        Self::ALL.iter().copied().find(|j| j.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JointId {
    Head,
    HandGrip(Hand),
    HandAim(Hand),
    Finger(Hand, FingerJoint),
}

impl JointId {
    pub fn from_name(s: &str) -> Option<JointId> {
        match s {
            "head" => return Some(JointId::Head),
            "right_grip" => return Some(JointId::HandGrip(Hand::Right)),
            "left_grip" => return Some(JointId::HandGrip(Hand::Left)),
            "right_aim" => return Some(JointId::HandAim(Hand::Right)),
            "left_aim" => return Some(JointId::HandAim(Hand::Left)),
            _ => {}
        }

        if let Some(rest) = s.strip_prefix("right_") {
            return FingerJoint::from_name(rest).map(|j| JointId::Finger(Hand::Right, j));
        }
        if let Some(rest) = s.strip_prefix("left_") {
            return FingerJoint::from_name(rest).map(|j| JointId::Finger(Hand::Left, j));
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

impl Transform {
    pub fn new(position: Vec3, rotation: Quat) -> Self {
        Self { position, rotation }
    }

    pub fn apply_offset(&self, offset_pos: Vec3, offset_rot: Quat) -> Transform {
        Transform {
            position: self.position + self.rotation * offset_pos,
            rotation: self.rotation * offset_rot,
        }
    }
}

/// Plain, JSON-safe mirror of `PlayerRig` — `serde_json` rejects non-string
/// map keys, so the wire form is a `Vec` of pairs instead of the enum-keyed
/// `HashMap`s `PlayerRig` uses internally. `#[serde(into/from)]` below
/// routes (de)serialization through this automatically.
#[derive(Serialize, Deserialize)]
struct PlayerRigWire {
    joints: Vec<(JointId, Transform)>,
    hand_tracking_active: Vec<(Hand, bool)>,
}

impl From<PlayerRig> for PlayerRigWire {
    fn from(rig: PlayerRig) -> Self {
        Self {
            joints: rig.joints.into_iter().collect(),
            hand_tracking_active: rig.hand_tracking_active.into_iter().collect(),
        }
    }
}

impl From<PlayerRigWire> for PlayerRig {
    fn from(wire: PlayerRigWire) -> Self {
        Self {
            joints: wire.joints.into_iter().collect(),
            hand_tracking_active: wire.hand_tracking_active.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(into = "PlayerRigWire", from = "PlayerRigWire")]
pub struct PlayerRig {
    pub joints: HashMap<JointId, Transform>,
    pub hand_tracking_active: HashMap<Hand, bool>,
}

impl PlayerRig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: JointId) -> Option<Transform> {
        self.joints.get(&id).copied()
    }

    pub fn head(&self) -> Transform {
        self.get(JointId::Head).unwrap_or_default()
    }

    pub fn hand_grip(&self, hand: Hand) -> Transform {
        self.get(JointId::HandGrip(hand)).unwrap_or_default()
    }

    pub fn hand_aim(&self, hand: Hand) -> Transform {
        self.get(JointId::HandAim(hand)).unwrap_or_default()
    }

    pub fn finger(&self, hand: Hand, joint: FingerJoint) -> Transform {
        self.get(JointId::Finger(hand, joint)).unwrap_or_default()
    }

    pub fn set_head(&mut self, position: Vec3, rotation: Quat) {
        self.joints
            .insert(JointId::Head, Transform::new(position, rotation));
    }

    pub fn set_hand_grip(&mut self, hand: Hand, position: Vec3, rotation: Quat) {
        self.joints
            .insert(JointId::HandGrip(hand), Transform::new(position, rotation));
    }

    pub fn set_hand_aim(&mut self, hand: Hand, position: Vec3, rotation: Quat) {
        self.joints
            .insert(JointId::HandAim(hand), Transform::new(position, rotation));
    }

    pub fn set_hand_joints(&mut self, hand: Hand, joints: &[(Vec3, Quat, bool)]) {
        for (i, &(pos, rot, valid)) in joints.iter().enumerate() {
            if !valid {
                continue;
            }
            if let Some(fj) = FingerJoint::from_index(i) {
                self.joints
                    .insert(JointId::Finger(hand, fj), Transform::new(pos, rot));
            }
        }
        self.hand_tracking_active.insert(hand, true);
    }

    pub fn clear_hand_tracking(&mut self, hand: Hand) {
        self.hand_tracking_active.insert(hand, false);
    }
}

#[cfg(test)]
mod wire_test {
    use super::*;

    #[test]
    fn player_rig_round_trips_through_json() {
        let mut rig = PlayerRig::new();
        rig.set_head(Vec3::new(1.0, 2.0, 3.0), Quat::from_rotation_y(0.5));
        rig.set_hand_grip(Hand::Right, Vec3::new(0.1, 0.2, 0.3), Quat::IDENTITY);
        rig.set_hand_joints(
            Hand::Left,
            &[(Vec3::ZERO, Quat::IDENTITY, true); FingerJoint::ALL.len()],
        );

        let json = serde_json::to_string(&rig).expect("serialize PlayerRig to JSON");
        let restored: PlayerRig = serde_json::from_str(&json).expect("deserialize PlayerRig from JSON");

        assert_eq!(restored.head(), rig.head());
        assert_eq!(restored.hand_grip(Hand::Right), rig.hand_grip(Hand::Right));
        assert_eq!(
            restored.hand_tracking_active.get(&Hand::Left),
            rig.hand_tracking_active.get(&Hand::Left)
        );
        assert_eq!(
            restored.finger(Hand::Left, FingerJoint::Wrist),
            rig.finger(Hand::Left, FingerJoint::Wrist)
        );
    }
}
