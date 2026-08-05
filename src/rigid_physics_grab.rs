use physx::prelude::*;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::events::Hand;
use crate::rigid_physics::{to_px_vec3, to_raw_transform, Drive, GrabState, PhysicsWorld, PxFoundation};
use crate::scene::{GripKind, GripPointDef};

impl PhysicsWorld {
    pub fn grab(&mut self, player: PlayerId, object_id: &str, hand: Hand, point: &GripPointDef) {
        self.release(player, object_id, hand);

        if self.grabs.iter().any(|((p, id, h), state)| {
            id == object_id && !(*p == player && *h == hand) && state.point_name == point.name
        }) {
            log::warn!(
                "rigid_physics: grab '{object_id}' at '{}' failed — already held by another hand",
                point.name
            );
            return;
        }

        let Some(state) = self.dynamic.get(object_id) else {
            log::warn!(
                "rigid_physics: grab '{object_id}' at '{}' failed — not a tracked Dynamic body",
                point.name
            );
            return;
        };
        let Some(&anchor_ptr) = self.hand_anchors.get(&(player, hand)) else {
            log::warn!(
                "rigid_physics: grab '{object_id}' failed — no hand anchor for {player:?}/{hand:?} (ensure_player not called yet?)"
            );
            return;
        };
        if anchor_ptr.is_null() {
            log::warn!(
                "rigid_physics: grab '{object_id}' failed — hand anchor for {player:?}/{hand:?} is null"
            );
            return;
        }

        let anchor_frame = to_raw_transform(PxTransform::default());
        let local_rot = point.local_rot;
        let object_frame = to_raw_transform(PxTransform::from_translation_rotation(
            &to_px_vec3(point.local_pos),
            &PxQuat::new(local_rot[0], local_rot[1], local_rot[2], local_rot[3]),
        ));

        let (linear, angular): (Drive, Option<Drive>) = match point.kind {
            GripKind::Snap => (
                Drive {
                    stiffness: 20000.0,
                    damping: 300.0,
                },
                Some(Drive {
                    stiffness: 2000.0,
                    damping: 80.0,
                }),
            ),
            GripKind::Free => (
                Drive {
                    stiffness: 20000.0,
                    damping: 300.0,
                },
                None,
            ),
            GripKind::Pinch => (
                Drive {
                    stiffness: 6000.0,
                    damping: 150.0,
                },
                Some(Drive {
                    stiffness: 6000.0,
                    damping: 150.0,
                }),
            ),
        };

        let anchor_ra = anchor_ptr as *mut physx_sys::PxRigidActor;
        let object_ra = state.ptr as *mut physx_sys::PxRigidActor;
        let joint = Self::create_driven_joint(
            &mut self.foundation,
            anchor_ra,
            &anchor_frame,
            object_ra,
            &object_frame,
            linear,
            angular,
        );

        if joint.is_null() {
            log::warn!(
                "rigid_physics: joint creation failed for '{object_id}' at '{}'",
                point.name
            );
            return;
        }

        self.grabs.insert(
            (player, object_id.to_string(), hand),
            GrabState {
                joint,
                point_name: point.name.clone(),
            },
        );
    }

    fn create_driven_joint(
        foundation: &mut PxFoundation,
        actor0: *mut physx_sys::PxRigidActor,
        frame0: &physx_sys::PxTransform,
        actor1: *mut physx_sys::PxRigidActor,
        frame1: &physx_sys::PxTransform,
        linear: Drive,
        angular: Option<Drive>,
    ) -> *mut physx_sys::PxJoint {
        unsafe {
            let joint = physx_sys::phys_PxD6JointCreate(
                foundation.as_mut_ptr(),
                actor0,
                frame0,
                actor1,
                frame1,
            );
            if joint.is_null() {
                return std::ptr::null_mut();
            }

            for axis in [
                physx_sys::PxD6Axis::X,
                physx_sys::PxD6Axis::Y,
                physx_sys::PxD6Axis::Z,
                physx_sys::PxD6Axis::Twist,
                physx_sys::PxD6Axis::Swing1,
                physx_sys::PxD6Axis::Swing2,
            ] {
                physx_sys::PxD6Joint_setMotion_mut(joint, axis, physx_sys::PxD6Motion::Free);
            }

            let linear_drive =
                physx_sys::PxD6JointDrive_new_1(linear.stiffness, linear.damping, 1.0e6, false);
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::X, &linear_drive);
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::Y, &linear_drive);
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::Z, &linear_drive);
            if let Some(a) = angular {
                let angular_drive =
                    physx_sys::PxD6JointDrive_new_1(a.stiffness, a.damping, 1.0e6, false);
                physx_sys::PxD6Joint_setDrive_mut(
                    joint,
                    physx_sys::PxD6Drive::Slerp,
                    &angular_drive,
                );
            }
            let rest = to_raw_transform(PxTransform::default());
            physx_sys::PxD6Joint_setDrivePosition_mut(joint, &rest, true);

            let zero = physx_sys::PxVec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            };
            physx_sys::PxD6Joint_setDriveVelocity_mut(joint, &zero, &zero, false);

            joint as *mut physx_sys::PxJoint
        }
    }

    pub fn release(&mut self, player: PlayerId, object_id: &str, hand: Hand) {
        if let Some(state) = self.grabs.remove(&(player, object_id.to_string(), hand)) {
            unsafe { physx_sys::PxJoint_release_mut(state.joint) };
        }
    }

    pub fn held_by(&self, player: PlayerId, hand: Hand) -> Option<(&str, &str)> {
        self.grabs
            .iter()
            .find(|((p, _, h), _)| *p == player && *h == hand)
            .map(|((_, id, _), state)| (id.as_str(), state.point_name.as_str()))
    }
}
