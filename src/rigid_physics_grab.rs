use physx::prelude::*;
use physx::traits::Class;

use space_soup_protocol::PlayerId;

use crate::events::Hand;
use glam::{Quat, Vec3};

use crate::rigid_physics::{
    from_px_transform, to_px_transform, to_px_vec3, to_raw_transform, Drive, GrabState, Parting, PhysicsWorld,
    PxFoundation,
};
use crate::scene::{GripKind, GripPointDef};

/// THE MOST ONE HAND HOLDS A LOOSE BODY WITH, newtons: what an arm lifts at
/// reach and keeps lifting. An 8 kg rock (78 N) leaves one hand a hundred
/// newtons to move it with -- 13 m/s2, a heave -- and a pebble all of it.
pub const HAND_FORCE: f32 = 180.0;
/// The most one hand turns a held body with, newton metres: a wrist.
pub const HAND_TORQUE: f32 = 12.0;
/// How quickly a held body follows the hand, hertz: a dead-beat spring this
/// stiff for its mass. Above the 60 steps a second's reach it only rings.
pub const HOLD_HZ: f32 = 10.0;
/// Further than this from what it holds, a hand has let go, metres.
pub const HOLD_BREAK: f32 = 0.75;
/// A body let go of has left the hand when its hold point is this far from
/// it, metres -- or after this long, seconds, whichever comes first.
pub const PARTED: f32 = 0.12;
pub const PARTING_TIME: f64 = 0.4;

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

        let anchor_ra = anchor_ptr.cast::<physx_sys::PxRigidActor>();
        let object_ra = state.ptr.cast::<physx_sys::PxRigidActor>();
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
                local: None,
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
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::X, &raw const linear_drive);
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::Y, &raw const linear_drive);
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::Z, &raw const linear_drive);
            if let Some(a) = angular {
                let angular_drive =
                    physx_sys::PxD6JointDrive_new_1(a.stiffness, a.damping, 1.0e6, false);
                physx_sys::PxD6Joint_setDrive_mut(
                    joint,
                    physx_sys::PxD6Drive::Slerp,
                    &raw const angular_drive,
                );
            }
            let rest = to_raw_transform(PxTransform::default());
            physx_sys::PxD6Joint_setDrivePosition_mut(joint, &raw const rest, true);

            let zero = physx_sys::PxVec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            };
            physx_sys::PxD6Joint_setDriveVelocity_mut(joint, &raw const zero, &raw const zero, false);

            joint.cast::<physx_sys::PxJoint>()
        }
    }

    /// A HAND TAKES HOLD OF A LOOSE BODY WHERE IT TOUCHES IT: a rock, a crate,
    /// anything dynamic with no authored grip. The body is held at the point
    /// of its collider nearest the hand (inside it: where the hand is), which
    /// a spring draws to the hand and keeps turned as it was in it.
    ///
    /// WEIGHT is the hand's strength: the spring is as stiff as the body is
    /// heavy -- the same quick, dead-beat following for a pebble and a
    /// boulder -- but pulls with no more than [`HAND_FORCE`] and turns with no
    /// more than [`HAND_TORQUE`]. A pebble (1 N) goes wherever the hand goes
    /// and leaves it at the hand's speed; an 8 kg rock (78 N) can be lifted
    /// and carried but lags a quick hand and cannot be flung; a second hand
    /// (`grab_free` again with the other) doubles what holds it. And the
    /// limit is what keeps a hand pushed through a wall from firing what it
    /// holds through it: it presses, it does not teleport.
    ///
    /// Letting go (`release`) removes the spring and nothing else: the body
    /// keeps the velocity it had, which is the throw.
    ///
    /// Returns whether the hand has it. Not for doors (`grab_door`).
    pub fn grab_free(&mut self, player: PlayerId, object_id: &str, hand: Hand, hand_pos: Vec3, hand_rot: Quat) -> bool {
        self.release(player, object_id, hand);
        if self.doors.contains_key(object_id) {
            return false;
        }
        let Some(state) = self.dynamic.get(object_id) else {
            log::warn!("rigid_physics: free grab of '{object_id}' failed -- not a dynamic body");
            return false;
        };
        let Some(&anchor) = self.hand_anchors.get(&(player, hand)).filter(|p| !p.is_null()) else {
            log::warn!("rigid_physics: free grab of '{object_id}' failed -- no hand anchor for {player:?}/{hand:?}");
            return false;
        };
        let (pos, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
        let local = (rot.inverse() * (hand_pos - pos)).clamp(-state.half, state.half);
        let body = state.ptr.cast::<physx_sys::PxRigidBody>();
        let mass = unsafe { physx_sys::PxRigidBody_getMass(body) }.max(1e-3);
        let inertia = unsafe { physx_sys::PxRigidBody_getMassSpaceInertiaTensor(body) };
        let inertia = inertia.x.max(inertia.y).max(inertia.z).max(1e-7);
        let w = std::f32::consts::TAU * HOLD_HZ;
        let joint = Self::create_held_joint(
            &mut self.foundation,
            anchor.cast::<physx_sys::PxRigidActor>(),
            state.ptr.cast::<physx_sys::PxRigidActor>(),
            &to_raw_transform(to_px_transform(local, rot.inverse() * hand_rot)),
            (mass * w * w, 2.0 * mass * w, HAND_FORCE),
            Some((inertia * w * w, 2.0 * inertia * w, HAND_TORQUE)),
        );
        if joint.is_null() {
            log::warn!("rigid_physics: joint creation failed for a free grab of '{object_id}'");
            return false;
        }
        unsafe { physx_sys::PxRigidDynamic_wakeUp_mut(state.ptr.cast::<physx_sys::PxRigidDynamic>()) };
        self.grabs.insert(
            (player, object_id.to_string(), hand),
            GrabState { joint, point_name: format!("free_{}", hand.as_str()), local: Some(local) },
        );
        self.refilter(object_id);
        true
    }

    /// A HAND TAKES A DOOR BY ITS HANDLE: a spring from the handle's hold
    /// point, on the face the hand is on, to the hand -- no stronger than an
    /// arm's steady pull (`door::DOOR_PULL_MAX`), and with nothing turning
    /// the leaf but its hinge. The leaf swings after the hand with its own
    /// mass. While held the latch is drawn.
    ///
    /// The spring is a torque `push_doors` puts on the hinge with the body's
    /// push, the closer and the friction, not a drive of the joint made here
    /// (which only stops hand and leaf colliding): so a pull INTO A STOP is
    /// nothing, as a body's lean into one is -- as a joint drive, a hand that
    /// went on past the stop rattled the leaf against it a quarter of a
    /// degree a tick -- and the headset's own hinge (`step_hinge_held`) is
    /// the same sum, term for term.
    ///
    /// Returns whether the hand has it: the door must have a handle
    /// (`DoorDef::handle`) and the hand be within `door::HANDLE_REACH` of it.
    pub fn grab_door(&mut self, player: PlayerId, object_id: &str, hand: Hand, hand_pos: Vec3) -> bool {
        self.release(player, object_id, hand);
        let (Some(geom), Some(latch)) = (self.doors.get(object_id), self.door_latches.get(object_id).map(|l| l.latch)) else {
            return false;
        };
        let Some(state) = self.dynamic.get(object_id) else { return false };
        let Some(&anchor) = self.hand_anchors.get(&(player, hand)).filter(|p| !p.is_null()) else {
            return false;
        };
        let pose = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
        if latch.distance(pose, geom.half.z, hand_pos) > crate::door::HANDLE_REACH {
            return false;
        }
        let local = latch.hold_point(pose, geom.half.z, hand_pos);
        let joint = Self::create_held_joint(
            &mut self.foundation,
            anchor.cast::<physx_sys::PxRigidActor>(),
            state.ptr.cast::<physx_sys::PxRigidActor>(),
            &to_raw_transform(to_px_transform(local, Quat::IDENTITY)),
            (0.0, 0.0, 0.0),
            None,
        );
        if joint.is_null() {
            return false;
        }
        unsafe { physx_sys::PxRigidDynamic_wakeUp_mut(state.ptr.cast::<physx_sys::PxRigidDynamic>()) };
        self.grabs.insert(
            (player, object_id.to_string(), hand),
            GrabState { joint, point_name: "handle".to_string(), local: Some(local) },
        );
        self.refilter(object_id);
        true
    }

    /// A free joint from a hand's anchor (at its centre) to `object_frame` on
    /// a body, driven together by a spring of `(stiffness, damping, most
    /// force)` and, if given, turned together by one of `(stiffness, damping,
    /// most torque)`. The limits are forces, whatever the step.
    fn create_held_joint(
        foundation: &mut PxFoundation,
        anchor: *mut physx_sys::PxRigidActor,
        object: *mut physx_sys::PxRigidActor,
        object_frame: &physx_sys::PxTransform,
        linear: (f32, f32, f32),
        angular: Option<(f32, f32, f32)>,
    ) -> *mut physx_sys::PxJoint {
        let anchor_frame = to_raw_transform(PxTransform::default());
        unsafe {
            let joint = physx_sys::phys_PxD6JointCreate(foundation.as_mut_ptr(), anchor, &raw const anchor_frame, object, object_frame);
            if joint.is_null() {
                return std::ptr::null_mut();
            }
            use physx_sys::{PxD6Axis as A, PxD6Drive as D, PxD6Motion as M};
            for axis in [A::X, A::Y, A::Z, A::Twist, A::Swing1, A::Swing2] {
                physx_sys::PxD6Joint_setMotion_mut(joint, axis, M::Free);
            }
            physx_sys::PxJoint_setConstraintFlag_mut(
                joint.cast::<physx_sys::PxJoint>(),
                physx_sys::PxConstraintFlag::DriveLimitsAreForces,
                true,
            );
            let drive = physx_sys::PxD6JointDrive_new_1(linear.0, linear.1, linear.2, false);
            for d in [D::X, D::Y, D::Z] {
                physx_sys::PxD6Joint_setDrive_mut(joint, d, &raw const drive);
            }
            if let Some((stiffness, damping, most)) = angular {
                let drive = physx_sys::PxD6JointDrive_new_1(stiffness, damping, most, false);
                physx_sys::PxD6Joint_setDrive_mut(joint, D::Slerp, &raw const drive);
            }
            let rest = to_raw_transform(PxTransform::default());
            physx_sys::PxD6Joint_setDrivePosition_mut(joint, &raw const rest, true);
            let zero = physx_sys::PxVec3 { x: 0.0, y: 0.0, z: 0.0 };
            physx_sys::PxD6Joint_setDriveVelocity_mut(joint, &raw const zero, &raw const zero, false);
            joint.cast::<physx_sys::PxJoint>()
        }
    }

    /// Have `PhysX` decide again which of a body's touching pairs collide. Two
    /// bodies a joint ties do not collide -- but a pair ALREADY touching when
    /// the joint is made keeps its contact, and a hand always touches what it
    /// takes hold of: the anchor's sphere kept pushing a held pebble its own
    /// radius (35 mm) off the hand. After a joint is made, and after one is
    /// released (so the hand pushes the thing again).
    fn refilter(&mut self, object_id: &str) {
        if let Some(state) = self.dynamic.get(object_id) {
            unsafe {
                physx_sys::PxScene_resetFiltering_mut(self.scene.as_mut_ptr(), state.ptr.cast::<physx_sys::PxActor>());
            }
        }
    }

    /// A HAND TOO FAR FROM WHAT IT HOLDS HAS LOST IT: a free grab or a door's
    /// handle whose hold point is more than [`HOLD_BREAK`] from the hand's
    /// anchor is let go -- the rock wedged behind a wall the player walked
    /// round, the door the player teleported away from. Each grab, each step:
    /// as many as there are hands holding things.
    pub(crate) fn drop_far_grabs(&mut self) {
        let mut lost: Vec<(PlayerId, String, Hand)> = Vec::new();
        for (key, grab) in &self.grabs {
            let Some(local) = grab.local else { continue };
            let (Some(state), Some(&anchor)) = (self.dynamic.get(&key.1), self.hand_anchors.get(&(key.0, key.2))) else {
                lost.push(key.clone());
                continue;
            };
            if anchor.is_null() {
                continue;
            }
            let (pos, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
            let (hand, _) = from_px_transform(&unsafe { (*anchor).get_global_pose() });
            if (pos + rot * local).distance(hand) > HOLD_BREAK {
                lost.push(key.clone());
            }
        }
        for (player, id, hand) in lost {
            log::info!("rigid_physics: {player:?}'s {hand:?} hand lost '{id}' (too far from it)");
            self.release(player, &id, hand);
        }
    }

    /// Whether this object is a dynamic body that is not a door: what a free
    /// grab can take.
    pub fn is_dynamic(&self, object_id: &str) -> bool {
        self.dynamic.contains_key(object_id) && !self.doors.contains_key(object_id)
    }

    /// Every hand lets go of this body.
    pub fn release_all(&mut self, object_id: &str) {
        let keys: Vec<_> = self.grabs.keys().filter(|(_, id, _)| id == object_id).cloned().collect();
        for key in keys {
            if let Some(state) = self.grabs.remove(&key) {
                unsafe { physx_sys::PxJoint_release_mut(state.joint) };
            }
        }
        self.parting.retain(|p| {
            if p.id == object_id {
                unsafe { physx_sys::PxJoint_release_mut(p.joint) };
            }
            p.id != object_id
        });
    }

    /// Whether any hand holds this body by a physics grab.
    pub fn is_held(&self, object_id: &str) -> bool {
        self.grabs.keys().any(|(_, id, _)| id == object_id)
    }

    /// A hand lets go. The body keeps the velocity it has: that is the throw.
    ///
    /// A free hold's joint is not taken away at once but left with nothing
    /// driving it until the body has LEFT THE HAND (`part_hands`): tied by a
    /// joint, hand and body do not collide, and a hand lets go of a thing it
    /// is wrapped round -- the anchor's sphere is inside the pebble's box.
    /// Colliding again on the instant, the pebble was pushed out of the hand
    /// sideways and left at a third of the speed it was thrown with.
    pub fn release(&mut self, player: PlayerId, object_id: &str, hand: Hand) {
        let Some(state) = self.grabs.remove(&(player, object_id.to_string(), hand)) else { return };
        let Some(local) = state.local else {
            unsafe { physx_sys::PxJoint_release_mut(state.joint) };
            return;
        };
        unsafe {
            let slack = physx_sys::PxD6JointDrive_new_1(0.0, 0.0, 0.0, false);
            let joint = state.joint.cast::<physx_sys::PxD6Joint>();
            use physx_sys::PxD6Drive as D;
            for d in [D::X, D::Y, D::Z, D::Slerp] {
                physx_sys::PxD6Joint_setDrive_mut(joint, d, &raw const slack);
            }
        }
        self.parting.push(Parting { joint: state.joint, player, hand, id: object_id.to_string(), local, since: self.clock });
    }

    /// The holds let go of whose body has left the hand -- its hold point
    /// [`PARTED`] from the anchor -- or that were let go [`PARTING_TIME`]
    /// ago: their joints go, and hand and body collide again.
    pub(crate) fn part_hands(&mut self) {
        if self.parting.is_empty() {
            return;
        }
        let now = self.clock;
        let mut parted: Vec<String> = Vec::new();
        let (dynamic, anchors) = (&self.dynamic, &self.hand_anchors);
        self.parting.retain(|p| {
            let apart = match (dynamic.get(&p.id), anchors.get(&(p.player, p.hand))) {
                (Some(state), Some(&anchor)) if !anchor.is_null() => {
                    let (pos, rot) = from_px_transform(&unsafe { (*state.ptr).get_global_pose() });
                    let (hand, _) = from_px_transform(&unsafe { (*anchor).get_global_pose() });
                    (pos + rot * p.local).distance(hand) > PARTED
                }
                _ => true,
            };
            if apart || now - p.since > PARTING_TIME {
                unsafe { physx_sys::PxJoint_release_mut(p.joint) };
                parted.push(p.id.clone());
                return false;
            }
            true
        });
        for id in parted {
            self.refilter(&id);
        }
    }

    pub fn held_by(&self, player: PlayerId, hand: Hand) -> Option<(&str, &str)> {
        self.grabs
            .iter()
            .find(|((p, _, h), _)| *p == player && *h == hand)
            .map(|((_, id, _), state)| (id.as_str(), state.point_name.as_str()))
    }
}
