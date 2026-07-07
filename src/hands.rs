use glam::{Quat, Vec3};

use crate::attach::Attachment;
use crate::events::Hand;
use crate::rig::{FingerJoint, JointId};
use crate::runtime::GameRuntime;
use crate::scene::{Color3, CuboidDef, CuboidStyle, GameObject};

fn joint_half_size(joint: FingerJoint) -> Vec3 {
    match joint {
        FingerJoint::Palm => Vec3::new(0.04, 0.012, 0.05),
        FingerJoint::Wrist => Vec3::new(0.025, 0.012, 0.025),
        FingerJoint::ThumbMeta
        | FingerJoint::IndexMeta
        | FingerJoint::MiddleMeta
        | FingerJoint::RingMeta
        | FingerJoint::LittleMeta => Vec3::new(0.009, 0.009, 0.014),
        FingerJoint::ThumbTip
        | FingerJoint::IndexTip
        | FingerJoint::MiddleTip
        | FingerJoint::RingTip
        | FingerJoint::LittleTip => Vec3::new(0.006, 0.006, 0.008),
        _ => Vec3::new(0.007, 0.007, 0.011),
    }
}

fn joint_color(hand: Hand) -> Color3 {
    match hand {
        Hand::Left => Color3(220, 180, 150, 255),
        Hand::Right => Color3(230, 190, 160, 255),
    }
}

pub fn spawn_hand_rig(runtime: &mut GameRuntime, hand: Hand) {
    let prefix = match hand {
        Hand::Left => "lhand",
        Hand::Right => "rhand",
    };
    let color = joint_color(hand);

    for joint in FingerJoint::ALL {
        let id = format!("{prefix}_{}", joint.name());

        let obj = GameObject {
            id: id.clone(),
            cuboid: CuboidDef {
                position: Vec3::ZERO,
                half_size: joint_half_size(joint),
                rotation: Quat::IDENTITY,
                color,
                wire_color: Color3(0, 0, 0, 0),
                style: CuboidStyle::Solid,
            },
            mesh: None,
            is_trigger: false,
            hidden: false,
            script: None,
            animations: vec![],
            animation_bindings: Vec::new(),
            rig_attachment: None,
            grip_pose_legacy: None,
            grip_pose_left: None,
            grip_pose_right: None,
            rigid_body: None,
            grip_points: Vec::new(),
            slider_joint: None,
            terrain_collider: None,
        };

        runtime.scene_mut().objects.push(obj);
        runtime
            .attachments
            .attach(&id, Attachment::rigid(JointId::Finger(hand, joint)));
    }

    log::info!("spawn_hand_rig: spawned 26 joint cuboids for {prefix}");
}

pub fn spawn_both_hand_rigs(runtime: &mut GameRuntime) {
    spawn_hand_rig(runtime, Hand::Left);
    spawn_hand_rig(runtime, Hand::Right);
}

pub fn despawn_hand_rig(runtime: &mut GameRuntime, hand: Hand) {
    let prefix = match hand {
        Hand::Left => "lhand",
        Hand::Right => "rhand",
    };
    for joint in FingerJoint::ALL {
        let id = format!("{prefix}_{}", joint.name());
        runtime.scene_mut().objects.retain(|o| o.id != id);
        runtime.attachments.detach(&id);
    }
}
