use glam::{Vec3, Quat};
use std::collections::HashMap;

use crate::events::Hand;


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FingerJoint {
    Palm,
    Wrist,
    ThumbMeta,  ThumbProx,  ThumbDist,  ThumbTip,
    IndexMeta,  IndexProx,  IndexInter,  IndexDist,  IndexTip,
    MiddleMeta, MiddleProx, MiddleInter, MiddleDist, MiddleTip,
    RingMeta,   RingProx,   RingInter,   RingDist,   RingTip,
    LittleMeta, LittleProx, LittleInter, LittleDist, LittleTip,
}

impl FingerJoint {
    pub const ALL: [FingerJoint; 26] = [
        FingerJoint::Palm,        FingerJoint::Wrist,
        FingerJoint::ThumbMeta,   FingerJoint::ThumbProx,  FingerJoint::ThumbDist,  FingerJoint::ThumbTip,
        FingerJoint::IndexMeta,   FingerJoint::IndexProx,  FingerJoint::IndexInter, FingerJoint::IndexDist, FingerJoint::IndexTip,
        FingerJoint::MiddleMeta,  FingerJoint::MiddleProx, FingerJoint::MiddleInter,FingerJoint::MiddleDist,FingerJoint::MiddleTip,
        FingerJoint::RingMeta,    FingerJoint::RingProx,   FingerJoint::RingInter,  FingerJoint::RingDist,  FingerJoint::RingTip,
        FingerJoint::LittleMeta,  FingerJoint::LittleProx, FingerJoint::LittleInter,FingerJoint::LittleDist,FingerJoint::LittleTip,
    ];

    pub fn from_index(i: usize) -> Option<FingerJoint> {
        Self::ALL.get(i).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            FingerJoint::Palm => "palm", FingerJoint::Wrist => "wrist",
            FingerJoint::ThumbMeta => "thumb_meta", FingerJoint::ThumbProx => "thumb_prox",
            FingerJoint::ThumbDist => "thumb_dist", FingerJoint::ThumbTip => "thumb_tip",
            FingerJoint::IndexMeta => "index_meta", FingerJoint::IndexProx => "index_prox",
            FingerJoint::IndexInter => "index_inter", FingerJoint::IndexDist => "index_dist",
            FingerJoint::IndexTip => "index_tip",
            FingerJoint::MiddleMeta => "middle_meta", FingerJoint::MiddleProx => "middle_prox",
            FingerJoint::MiddleInter => "middle_inter", FingerJoint::MiddleDist => "middle_dist",
            FingerJoint::MiddleTip => "middle_tip",
            FingerJoint::RingMeta => "ring_meta", FingerJoint::RingProx => "ring_prox",
            FingerJoint::RingInter => "ring_inter", FingerJoint::RingDist => "ring_dist",
            FingerJoint::RingTip => "ring_tip",
            FingerJoint::LittleMeta => "little_meta", FingerJoint::LittleProx => "little_prox",
            FingerJoint::LittleInter => "little_inter", FingerJoint::LittleDist => "little_dist",
            FingerJoint::LittleTip => "little_tip",
        }
    }

    pub fn from_name(s: &str) -> Option<FingerJoint> {
        Self::ALL.iter().copied().find(|j| j.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
            "left_grip"  => return Some(JointId::HandGrip(Hand::Left)),
            "right_aim"  => return Some(JointId::HandAim(Hand::Right)),
            "left_aim"   => return Some(JointId::HandAim(Hand::Left)),
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: Vec3::ZERO, rotation: Quat::IDENTITY }
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

#[derive(Debug, Clone, Default)]
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
        self.joints.insert(JointId::Head, Transform::new(position, rotation));
    }

    pub fn set_hand_grip(&mut self, hand: Hand, position: Vec3, rotation: Quat) {
        self.joints.insert(JointId::HandGrip(hand), Transform::new(position, rotation));
    }

    pub fn set_hand_aim(&mut self, hand: Hand, position: Vec3, rotation: Quat) {
        self.joints.insert(JointId::HandAim(hand), Transform::new(position, rotation));
    }

    pub fn set_hand_joints(&mut self, hand: Hand, joints: &[(Vec3, Quat, bool)]) {
        for (i, &(pos, rot, valid)) in joints.iter().enumerate() {
            if !valid { continue; }
            if let Some(fj) = FingerJoint::from_index(i) {
                self.joints.insert(JointId::Finger(hand, fj), Transform::new(pos, rot));
            }
        }
        self.hand_tracking_active.insert(hand, true);
    }

    pub fn clear_hand_tracking(&mut self, hand: Hand) {
        self.hand_tracking_active.insert(hand, false);
    }
}
