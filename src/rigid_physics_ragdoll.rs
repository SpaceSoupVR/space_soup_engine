//! RAGDOLLS: one body made of several rigid bodies held together by joints -- a corpse.
//!
//! Built as Unity's SCP: Blacksite built its corpses (`RagdollBuilder.cs`), on what this engine
//! already uses for doors and grabs: one `PxRigidDynamic` per limb and a `PxD6Joint` from each
//! limb to its parent. Not a PhysX articulation, for this game's reasons (2026-10-18 research,
//! `reports/Ragdolls for Space Soup VR.md`): an articulation's links cannot be kinematic (a hand
//! carrying a limb), its joints cannot break (a limb torn off), the `physx` 0.19 wrapper cannot
//! make its links, and 5.1.3 lacks the later fixes for its spherical joints.
//!
//! What it does, and what Unity's did that it does not:
//! - Each joint's NEUTRAL is the bones' rest pose ([`RagdollBodyDef::rest`]), so a limit means
//!   the same thing however the body died. Its cone is ROUND: a small or very elliptical cone
//!   jitters (PhysX's own guidance), and Unity's swing2 was half its swing1.
//! - Limbs that are not jointed to each other COLLIDE (an arm lies across the chest, not
//!   through it). Unity made every pair ignore every other, because its colliders were
//!   deliberately fat and started out overlapping; these are measured to fit
//!   (`blacksite::ragdoll`), and a joint keeps only its own two limbs apart from collision.
//! - Fast limbs use SPECULATIVE CCD, which needs nothing from the filter shader (the scene's
//!   default shader asks for no CCD contacts, so ordinary CCD does nothing here), so a body
//!   thrown at a wall at 40 m/s does not go through it.
//! - A limb can be torn off, by a monster: a pull past its tear force held for [`TEAR_AFTER_S`]
//!   while a monster's grip holds the body (not PhysX's instant break force, which an impact's
//!   spike tripped; and not by a landing or a skid), or [`PhysicsWorld::tear_ragdoll_limb`].
//! - A hand holds a limb with a SPRING, not by moving it (Unity made the limb kinematic and
//!   moved it, so a dragged body went through walls and trailed stiffly): a kinematic anchor at
//!   the hand, a soft joint to the limb, and a break force so a body snagged on a door frame
//!   comes out of the hand rather than through the frame ([`PhysicsWorld::grip_ragdoll`]).
//! - A settled corpse can be FROZEN: its bodies made kinematic where they lie, so nothing wakes
//!   it and it costs nothing ([`PhysicsWorld::freeze_ragdoll`]).
//!
//! Solver: the scene's (PGS), with each limb solved 16/4 times a step, as the report found
//! for jointed bodies on 5.1.3. No joint drives on the limbs: 5.1.3's drives misbehave under
//! TGS and a powered ragdoll is not needed for a corpse.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use physx::prelude::*;
use physx::traits::Class;
use serde::{Deserialize, Serialize};

use crate::rigid_physics::{from_px_transform, to_px_transform, to_raw_transform, PhysicsWorld, PxRigidDynamic};

/// A limb's collider, in its body's frame. A capsule lies along its own +X, as PhysX's do.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RagdollShape {
    Capsule { radius: f32, half_height: f32 },
    Box { half: [f32; 3] },
    Sphere { radius: f32 },
}

/// One limb: a body on a bone, jointed to its parent's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagdollBodyDef {
    /// The skeleton's bone this body moves.
    pub bone: String,
    /// Index of the body this one hangs from; `None` only for the root, which is the first.
    pub parent: Option<usize>,
    pub shape: RagdollShape,
    /// Where the collider sits in the bone's frame, and how it is turned.
    pub shape_pos: [f32; 3],
    pub shape_rot: [f32; 4],
    pub mass: f32,
    /// The bone at rest, in its parent body's bone frame: where this limb's joint is neutral.
    pub rest_pos: [f32; 3],
    pub rest_rot: [f32; 4],
    /// Along the limb, in the bone's frame (unit): the joint's twist axis.
    pub axis: [f32; 3],
    /// How far the limb may swing away from neutral (a round cone), and twist about its axis,
    /// degrees. Under 1 degree locks the axis.
    pub swing_deg: f32,
    pub twist_deg: f32,
    /// The pull that tears this limb from its parent when held [`TEAR_AFTER_S`] (N); 0 never.
    /// `break_torque` is unused (kept for the file format).
    #[serde(default)]
    pub break_force: f32,
    #[serde(default)]
    pub break_torque: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagdollDef {
    /// The root first; every other body after its parent.
    pub bodies: Vec<RagdollBodyDef>,
}

impl RagdollDef {
    pub fn validate(&self) -> Result<(), String> {
        let Some(root) = self.bodies.first() else { return Err("a ragdoll with no bodies".into()) };
        if root.parent.is_some() {
            return Err("the first body must be the root (no parent)".into());
        }
        for (i, b) in self.bodies.iter().enumerate() {
            match b.parent {
                None if i > 0 => return Err(format!("{}: a second root", b.bone)),
                Some(p) if p >= i => return Err(format!("{}: its parent comes after it", b.bone)),
                _ => {}
            }
            let positive = match b.shape {
                RagdollShape::Capsule { radius, half_height } => radius > 0.0 && half_height >= 0.0,
                RagdollShape::Box { half } => half.iter().all(|h| *h > 0.0),
                RagdollShape::Sphere { radius } => radius > 0.0,
            };
            if !positive || b.mass.is_nan() || b.mass <= 0.0 || Vec3::from(b.axis).length() < 0.5 {
                return Err(format!("{}: a shape, mass or axis that is not positive", b.bone));
            }
        }
        Ok(())
    }
}

/// Where a ragdoll starts: each body's pose (its bone's, world) and velocity.
#[derive(Debug, Clone, PartialEq)]
pub struct RagdollStart {
    pub poses: Vec<(Vec3, Quat)>,
    pub linear: Vec<Vec3>,
    pub angular: Vec<Vec3>,
}

/// Each body's BONE, world, for posing the skin: the root where its body is, and every other
/// bone turned as its body is but placed where its parent says (`rest_pos`), so the skin never
/// stretches however far a joint gave at an impact (a few centimetres; see `spawn_ragdoll`).
pub fn ragdoll_bone_poses(def: &RagdollDef, bodies: &[(Vec3, Quat)]) -> Vec<(Vec3, Quat)> {
    let mut out: Vec<(Vec3, Quat)> = Vec::with_capacity(def.bodies.len());
    for (i, b) in def.bodies.iter().enumerate() {
        let (_, rot) = bodies[i];
        out.push(match b.parent {
            None => bodies[i],
            Some(p) => (out[p].0 + out[p].1 * Vec3::from(b.rest_pos), rot),
        });
    }
    out
}

/// Limbs: Unity's damping (`RagdollBuilder.cs`), so a corpse settles rather than rolls for ever.
const LINEAR_DAMPING: f32 = 0.05;
const ANGULAR_DAMPING: f32 = 1.2;
/// Pushed out of what it starts in at a stumble, not launched (NVIDIA's example value).
const MAX_DEPENETRATION: f32 = 3.0;
/// Each limb's inertia, times what its shape and mass give it (see `spawn_ragdoll`).
const INERTIA_SCALE: f32 = 2.0;
/// The solver passes a limb gets, position and velocity.
const ITERATIONS: (u32, u32) = (16, 4);
/// How hard a hand holds a limb: the spring from hand to limb (N/m), its damping, and how far
/// the hand may get from the point it holds before it lets go ([`GRIP_REACH`]): together, the
/// most it can pull with -- `stiffness x reach`. A player's hand cannot tear a limb off (a body
/// snagged on a frame comes out of the hand), a monster's can (096, two hands).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GripStrength {
    pub stiffness: f32,
    pub damping: f32,
    /// Whether a body held by this can have limbs torn off it: only a monster's grip. Nothing
    /// else tears a limb off -- not a throw, a landing or a skid, whose loads pass the tear
    /// force too (a body skidding 13 m after a catch lost its head and arms).
    pub tears: bool,
    /// How far the hand may get from the point it holds before it lets go (m): a body snagged
    /// on a frame comes out of the hand. Together with `stiffness`, the most it pulls with.
    pub reach: f32,
}

/// A player's hand: at most about 360 N (`800 x 0.45`), and it tears nothing.
pub const PLAYER_GRIP: GripStrength = GripStrength { stiffness: 800.0, damping: 50.0, tears: false, reach: GRIP_REACH };
/// A monster's: at most about 900 N, enough to tear a head or an arm off (096).
pub const MONSTER_GRIP: GripStrength = GripStrength { stiffness: 2000.0, damping: 80.0, tears: true, reach: GRIP_REACH };
/// A hand this far from the point it holds has let go, by default (`GripStrength::reach`): the
/// body snagged (a door frame, a wall) and did not come. As the engine's own grabs let go (`drop_far_grabs`); a joint's break
/// force alone did not count its drive's pull and held on through a wall.
pub const GRIP_REACH: f32 = 0.45;

pub(crate) struct Ragdoll {
    bodies: Vec<*mut physx_sys::PxRigidDynamic>,
    /// Each body's joint to its parent (`None` for the root, and once torn off).
    joints: Vec<Option<*mut physx_sys::PxD6Joint>>,
    frozen: bool,
    /// Each limb's tear force (N, 0 never) and how long its joint has been pulled past it (s).
    tear: Vec<f32>,
    strained: Vec<f32>,
}

/// A limb tears when its joint is pulled past its tear force for this long, straight (s). Not
/// PhysX's break force, which tears on the instant: an impact's spike tore limbs off a body
/// landing. Measured (`joint_loads_impacts_against_pulls`): a 30 m/s body into a wall peaks at
/// 11,000 N but in runs of a few steps; a monster pulling an arm off a held body stays past
/// 600 N for 29 steps (0.48 s); a player's hand never reaches it (300 N).
pub const TEAR_AFTER_S: f32 = 0.25;

pub(crate) struct RagdollGrip {
    ragdoll: String,
    anchor: *mut physx_sys::PxRigidDynamic,
    /// Null once it has let go.
    joint: *mut physx_sys::PxD6Joint,
    /// The limb held, the point held in its frame, and where the hand is.
    limb: *mut physx_sys::PxRigidDynamic,
    point: Vec3,
    hand: Vec3,
    tears: bool,
    reach: f32,
}

fn raw(pos: Vec3, rot: Quat) -> physx_sys::PxTransform {
    to_raw_transform(to_px_transform(pos, rot.normalize()))
}

fn px(v: Vec3) -> physx_sys::PxVec3 {
    physx_sys::PxVec3 { x: v.x, y: v.y, z: v.z }
}

fn as_body(p: *mut physx_sys::PxRigidDynamic) -> *mut physx_sys::PxRigidBody {
    p as *mut physx_sys::PxRigidBody
}

fn as_joint(j: *mut physx_sys::PxD6Joint) -> *mut physx_sys::PxJoint {
    j as *mut physx_sys::PxJoint
}

impl PhysicsWorld {
    /// Makes ragdoll `id` from `def`, each body where `start` puts it and moving as it says.
    /// Replaces a ragdoll of the same id.
    pub fn spawn_ragdoll(&mut self, id: &str, def: &RagdollDef, start: &RagdollStart) -> Result<(), String> {
        def.validate()?;
        let n = def.bodies.len();
        if start.poses.len() != n || start.linear.len() != n || start.angular.len() != n {
            return Err(format!("{id}: a start for {} bodies, the ragdoll has {n}", start.poses.len()));
        }
        self.remove_ragdoll(id);
        let m = self.ragdoll_material().ok_or("could not make a material")?;
        let mut bodies = Vec::with_capacity(n);
        for (b, &(pos, rot)) in def.bodies.iter().zip(&start.poses) {
            let shape_pose = to_px_transform(Vec3::from(b.shape_pos), Quat::from_array(b.shape_rot).normalize());
            let pose = to_px_transform(pos, rot.normalize());
            let created = match b.shape {
                RagdollShape::Capsule { radius, half_height } => {
                    self.foundation.create_rigid_dynamic(pose, &PxCapsuleGeometry::new(radius, half_height.max(1e-4)), self.materials[m].as_mut(), 1.0, shape_pose, ())
                }
                RagdollShape::Box { half } => {
                    self.foundation.create_rigid_dynamic(pose, &PxBoxGeometry::new(half[0], half[1], half[2]), self.materials[m].as_mut(), 1.0, shape_pose, ())
                }
                RagdollShape::Sphere { radius } => self.foundation.create_rigid_dynamic(pose, &PxSphereGeometry::new(radius), self.materials[m].as_mut(), 1.0, shape_pose, ()),
            };
            let Some(mut actor) = created else {
                for &p in &bodies {
                    self.release_body(p);
                }
                return Err(format!("{id}: could not make the body for {}", b.bone));
            };
            let ptr = &mut *actor as *mut PxRigidDynamic as *mut physx_sys::PxRigidDynamic;
            unsafe {
                physx_sys::PxRigidBodyExt_setMassAndUpdateInertia_1(as_body(ptr), b.mass, std::ptr::null(), false);
                let i = physx_sys::PxRigidBody_getMassSpaceInertiaTensor(as_body(ptr));
                let i = physx_sys::PxVec3 { x: i.x * INERTIA_SCALE, y: i.y * INERTIA_SCALE, z: i.z * INERTIA_SCALE };
                physx_sys::PxRigidBody_setMassSpaceInertiaTensor_mut(as_body(ptr), &i);
                physx_sys::PxRigidBody_setLinearDamping_mut(as_body(ptr), LINEAR_DAMPING);
                physx_sys::PxRigidBody_setAngularDamping_mut(as_body(ptr), ANGULAR_DAMPING);
                physx_sys::PxRigidBody_setMaxDepenetrationVelocity_mut(as_body(ptr), MAX_DEPENETRATION);
                physx_sys::PxRigidBody_setRigidBodyFlag_mut(as_body(ptr), physx_sys::PxRigidBodyFlag::EnableSpeculativeCcd, true);
                physx_sys::PxRigidDynamic_setSolverIterationCounts_mut(ptr, ITERATIONS.0, ITERATIONS.1);
                let mut shape: *mut physx_sys::PxShape = std::ptr::null_mut();
                physx_sys::PxRigidActor_getShapes(ptr as *const physx_sys::PxRigidActor, &mut shape, 1, 0);
                if !shape.is_null() {
                    physx_sys::PxShape_setFlag_mut(shape, physx_sys::PxShapeFlag::SceneQueryShape, false);
                }
            }
            self.scene.add_dynamic_actor(actor);
            bodies.push(ptr);
        }

        let mut joints = vec![None; n];
        for (i, b) in def.bodies.iter().enumerate() {
            let Some(p) = b.parent else { continue };
            let axis = Vec3::from(b.axis).normalize();
            let to_axis = Quat::from_rotation_arc(Vec3::X, axis);
            let child_frame = raw(Vec3::ZERO, to_axis);
            let parent_frame = raw(Vec3::from(b.rest_pos), Quat::from_array(b.rest_rot).normalize() * to_axis);
            let joint = unsafe {
                physx_sys::phys_PxD6JointCreate(
                    self.foundation.as_mut_ptr(),
                    bodies[p] as *mut physx_sys::PxRigidActor,
                    &parent_frame,
                    bodies[i] as *mut physx_sys::PxRigidActor,
                    &child_frame,
                )
            };
            if joint.is_null() {
                log::warn!("ragdoll {id}: the joint for {} failed", b.bone);
                continue;
            }
            unsafe {
                use physx_sys::{PxD6Axis as A, PxD6Motion as M};
                for a in [A::X, A::Y, A::Z] {
                    physx_sys::PxD6Joint_setMotion_mut(joint, a, M::Locked);
                }
                let twist = b.twist_deg.to_radians();
                if twist < 1f32.to_radians() {
                    physx_sys::PxD6Joint_setMotion_mut(joint, A::Twist, M::Locked);
                } else {
                    physx_sys::PxD6Joint_setMotion_mut(joint, A::Twist, M::Limited);
                    physx_sys::PxD6Joint_setTwistLimit_mut(joint, &physx_sys::PxJointAngularLimitPair_new(-twist, twist, 0.0));
                }
                let swing = b.swing_deg.to_radians().min(170f32.to_radians());
                if swing < 1f32.to_radians() {
                    physx_sys::PxD6Joint_setMotion_mut(joint, A::Swing1, M::Locked);
                    physx_sys::PxD6Joint_setMotion_mut(joint, A::Swing2, M::Locked);
                } else {
                    physx_sys::PxD6Joint_setMotion_mut(joint, A::Swing1, M::Limited);
                    physx_sys::PxD6Joint_setMotion_mut(joint, A::Swing2, M::Limited);
                    physx_sys::PxD6Joint_setSwingLimit_mut(joint, &physx_sys::PxJointLimitCone_new(swing, swing, 0.0));
                }
            }
            joints[i] = Some(joint);
        }

        for (i, &ptr) in bodies.iter().enumerate() {
            unsafe {
                physx_sys::PxRigidDynamic_setLinearVelocity_mut(ptr, &px(start.linear[i]), true);
                physx_sys::PxRigidDynamic_setAngularVelocity_mut(ptr, &px(start.angular[i]), true);
            }
        }
        let tear = def.bodies.iter().map(|b| b.break_force).collect();
        self.ragdolls.insert(id.to_string(), Ragdoll { bodies, joints, frozen: false, tear, strained: vec![0.0; n] });
        Ok(())
    }

    /// The limbs' material (grippy, dead: a body lands and stays), made the first time.
    fn ragdoll_material(&mut self) -> Option<usize> {
        if let Some(m) = self.ragdoll_material {
            return Some(m);
        }
        let material = self.foundation.create_material(0.7, 0.6, 0.0, ())?;
        self.materials.push(material);
        self.ragdoll_material = Some(self.materials.len() - 1);
        self.ragdoll_material
    }

    fn release_body(&mut self, ptr: *mut physx_sys::PxRigidDynamic) {
        unsafe {
            physx_sys::PxScene_removeActor_mut(self.scene.as_mut_ptr(), ptr as *mut physx_sys::PxActor, false);
            physx_sys::PxActor_release_mut(ptr as *mut physx_sys::PxActor);
        }
    }

    /// Removes ragdoll `id` and anything holding it.
    pub fn remove_ragdoll(&mut self, id: &str) {
        let held: Vec<u32> = self.ragdoll_grips.iter().filter(|(_, g)| g.ragdoll == id).map(|(&k, _)| k).collect();
        for k in held {
            self.release_ragdoll_grip(k);
        }
        let Some(r) = self.ragdolls.remove(id) else { return };
        for j in r.joints.into_iter().flatten() {
            unsafe { physx_sys::PxJoint_release_mut(as_joint(j)) };
        }
        for p in r.bodies {
            self.release_body(p);
        }
    }

    /// Every ragdoll gone, joints first: before the scene that holds their bodies is replaced.
    pub(crate) fn release_all_ragdolls(&mut self) {
        let ids: Vec<String> = self.ragdolls.keys().cloned().collect();
        for id in ids {
            self.remove_ragdoll(&id);
        }
    }

    pub fn ragdoll_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.ragdolls.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Each body's pose, world, in the order of its def.
    pub fn ragdoll_poses(&self, id: &str) -> Option<Vec<(Vec3, Quat)>> {
        let r = self.ragdolls.get(id)?;
        Some(r.bodies.iter().map(|&p| from_px_transform(&unsafe { (*(p as *mut PxRigidDynamic)).get_global_pose() })).collect())
    }

    /// Each body's linear velocity, world.
    pub fn ragdoll_velocities(&self, id: &str) -> Option<Vec<Vec3>> {
        let r = self.ragdolls.get(id)?;
        Some(
            r.bodies
                .iter()
                .map(|&p| {
                    let v = unsafe { physx_sys::PxRigidBody_getLinearVelocity(as_body(p)) };
                    Vec3::new(v.x, v.y, v.z)
                })
                .collect(),
        )
    }

    /// Whether any of its bodies is still moving (awake). A frozen ragdoll is not.
    pub fn ragdoll_awake(&self, id: &str) -> bool {
        self.ragdolls
            .get(id)
            .is_some_and(|r| !r.frozen && r.bodies.iter().any(|&p| !unsafe { physx_sys::PxRigidDynamic_isSleeping(p) }))
    }

    /// The limbs whose joint has torn (by its break force) or been torn off, by body index.
    pub fn ragdoll_torn(&self, id: &str) -> Vec<usize> {
        let Some(r) = self.ragdolls.get(id) else { return Vec::new() };
        (1..r.bodies.len())
            .filter(|&i| match r.joints[i] {
                None => true,
                Some(j) => unsafe { physx_sys::PxJoint_getConstraintFlags(as_joint(j)) }.contains(physx_sys::PxConstraintFlags::Broken),
            })
            .collect()
    }

    /// The force (N) and torque (N m) limb `body`'s joint carried last step, for tuning and tests.
    pub fn ragdoll_joint_load(&self, id: &str, body: usize) -> Option<(f32, f32)> {
        let j = (*self.ragdolls.get(id)?.joints.get(body)?)?;
        unsafe {
            let c = physx_sys::PxJoint_getConstraint(as_joint(j));
            if c.is_null() {
                return None;
            }
            let (mut f, mut t) = (physx_sys::PxVec3 { x: 0.0, y: 0.0, z: 0.0 }, physx_sys::PxVec3 { x: 0.0, y: 0.0, z: 0.0 });
            physx_sys::PxConstraint_getForce(c, &mut f, &mut t);
            Some((Vec3::new(f.x, f.y, f.z).length(), Vec3::new(t.x, t.y, t.z).length()))
        }
    }

    /// Tears limb `body` from its parent.
    pub fn tear_ragdoll_limb(&mut self, id: &str, body: usize) {
        let Some(r) = self.ragdolls.get_mut(id) else { return };
        if let Some(Some(j)) = r.joints.get_mut(body).map(|j| j.take()) {
            unsafe { physx_sys::PxJoint_release_mut(as_joint(j)) };
        }
        if let Some(&p) = r.bodies.get(body) {
            unsafe { physx_sys::PxRigidDynamic_wakeUp_mut(p) };
        }
    }

    /// A blow to limb `body`: an impulse (N s) through its middle.
    pub fn push_ragdoll_limb(&mut self, id: &str, body: usize, impulse: Vec3) {
        let Some(r) = self.ragdolls.get(id) else { return };
        if r.frozen {
            return;
        }
        if let Some(&p) = r.bodies.get(body) {
            unsafe { physx_sys::PxRigidBody_addForce_mut(as_body(p), &px(impulse), physx_sys::PxForceMode::Impulse, true) };
        }
    }

    /// Freezes it where it lies (kinematic: nothing moves or wakes it, and it costs nothing),
    /// or lets it move again.
    pub fn freeze_ragdoll(&mut self, id: &str, frozen: bool) {
        let Some(r) = self.ragdolls.get_mut(id) else { return };
        if r.frozen == frozen {
            return;
        }
        r.frozen = frozen;
        for &p in &r.bodies {
            unsafe {
                if frozen {
                    physx_sys::PxRigidBody_setRigidBodyFlag_mut(as_body(p), physx_sys::PxRigidBodyFlag::EnableSpeculativeCcd, false);
                    physx_sys::PxRigidBody_setRigidBodyFlag_mut(as_body(p), physx_sys::PxRigidBodyFlag::Kinematic, true);
                } else {
                    physx_sys::PxRigidBody_setRigidBodyFlag_mut(as_body(p), physx_sys::PxRigidBodyFlag::Kinematic, false);
                    physx_sys::PxRigidBody_setRigidBodyFlag_mut(as_body(p), physx_sys::PxRigidBodyFlag::EnableSpeculativeCcd, true);
                    physx_sys::PxRigidDynamic_wakeUp_mut(p);
                }
            }
        }
    }

    /// A hand takes hold of limb `body` at `at` (world): a kinematic anchor there and a spring
    /// from it to the limb, the limb free to turn on it. Returns the grip, or `None`.
    pub fn grip_ragdoll(&mut self, id: &str, body: usize, at: Vec3, strength: GripStrength) -> Option<u32> {
        let r = self.ragdolls.get(id)?;
        if r.frozen {
            self.freeze_ragdoll(id, false);
        }
        let limb = *self.ragdolls.get(id)?.bodies.get(body)?;
        let m = self.ragdoll_material()?;
        let mut anchor = self.foundation.create_rigid_dynamic(
            to_px_transform(at, Quat::IDENTITY),
            &PxSphereGeometry::new(0.02),
            self.materials[m].as_mut(),
            1.0,
            PxTransform::default(),
            (),
        )?;
        anchor.set_rigid_body_flag(RigidBodyFlag::Kinematic, true);
        let anchor_ptr = &mut *anchor as *mut PxRigidDynamic as *mut physx_sys::PxRigidDynamic;
        unsafe {
            let mut shape: *mut physx_sys::PxShape = std::ptr::null_mut();
            physx_sys::PxRigidActor_getShapes(anchor_ptr as *const physx_sys::PxRigidActor, &mut shape, 1, 0);
            if !shape.is_null() {
                physx_sys::PxShape_setFlag_mut(shape, physx_sys::PxShapeFlag::SimulationShape, false);
                physx_sys::PxShape_setFlag_mut(shape, physx_sys::PxShapeFlag::SceneQueryShape, false);
            }
        }
        self.scene.add_dynamic_actor(anchor);

        let (lp, lr) = from_px_transform(&unsafe { (*(limb as *mut PxRigidDynamic)).get_global_pose() });
        let in_limb = lr.inverse() * (at - lp);
        let joint = unsafe {
            physx_sys::phys_PxD6JointCreate(
                self.foundation.as_mut_ptr(),
                anchor_ptr as *mut physx_sys::PxRigidActor,
                &raw(Vec3::ZERO, Quat::IDENTITY),
                limb as *mut physx_sys::PxRigidActor,
                &raw(in_limb, lr.inverse()),
            )
        };
        if joint.is_null() {
            self.release_body(anchor_ptr);
            return None;
        }
        unsafe {
            use physx_sys::{PxD6Axis as A, PxD6Drive as D, PxD6Motion as M};
            for a in [A::X, A::Y, A::Z] {
                physx_sys::PxD6Joint_setMotion_mut(joint, a, M::Free);
            }
            for a in [A::Twist, A::Swing1, A::Swing2] {
                physx_sys::PxD6Joint_setMotion_mut(joint, a, M::Free);
            }
            let spring = physx_sys::PxD6JointDrive_new_1(strength.stiffness, strength.damping, f32::MAX, false);
            for d in [D::X, D::Y, D::Z] {
                physx_sys::PxD6Joint_setDrive_mut(joint, d, &spring);
            }
            physx_sys::PxRigidDynamic_wakeUp_mut(limb);
        }
        let key = self.next_ragdoll_grip;
        self.next_ragdoll_grip = self.next_ragdoll_grip.wrapping_add(1);
        self.ragdoll_grips.insert(key, RagdollGrip { ragdoll: id.to_string(), anchor: anchor_ptr, joint, limb, point: in_limb, hand: at, tears: strength.tears, reach: strength.reach });
        Some(key)
    }

    /// Moves a grip's hand to `at` (world), for the next step.
    pub fn move_ragdoll_grip(&mut self, grip: u32, at: Vec3) {
        if let Some(g) = self.ragdoll_grips.get_mut(&grip) {
            g.hand = at;
            let target = raw(at, Quat::IDENTITY);
            unsafe { physx_sys::PxRigidDynamic_setKinematicTarget_mut(g.anchor, &target) };
        }
    }

    /// Whether a grip still holds (it lets go of a limb pulled too hard, or of a body snagged
    /// on something, [`GRIP_REACH`]). A grip that has let go stays until released.
    pub fn ragdoll_grip_holds(&self, grip: u32) -> bool {
        self.ragdoll_grips.get(&grip).is_some_and(|g| {
            !g.joint.is_null() && !unsafe { physx_sys::PxJoint_getConstraintFlags(as_joint(g.joint)) }.contains(physx_sys::PxConstraintFlags::Broken)
        })
    }

    pub fn release_ragdoll_grip(&mut self, grip: u32) {
        if let Some(g) = self.ragdoll_grips.remove(&grip) {
            if !g.joint.is_null() {
                unsafe { physx_sys::PxJoint_release_mut(as_joint(g.joint)) };
            }
            self.release_body(g.anchor);
        }
    }

    /// Tears every limb pulled past its tear force for [`TEAR_AFTER_S`]: after each step, from
    /// the loads that step carried.
    pub(crate) fn tear_strained_limbs(&mut self, dt: f32) {
        let mut tear: Vec<(String, usize)> = Vec::new();
        for (id, r) in &mut self.ragdolls {
            let gripped = self.ragdoll_grips.values().any(|g| g.tears && !g.joint.is_null() && g.ragdoll == *id);
            if r.frozen || !gripped {
                r.strained.iter_mut().for_each(|s| *s = 0.0);
                continue;
            }
            for i in 1..r.bodies.len() {
                let (Some(j), limit) = (r.joints[i], r.tear[i]) else { continue };
                if limit <= 0.0 {
                    continue;
                }
                let load = unsafe {
                    let c = physx_sys::PxJoint_getConstraint(as_joint(j));
                    let (mut f, mut t) = (physx_sys::PxVec3 { x: 0.0, y: 0.0, z: 0.0 }, physx_sys::PxVec3 { x: 0.0, y: 0.0, z: 0.0 });
                    if !c.is_null() {
                        physx_sys::PxConstraint_getForce(c, &mut f, &mut t);
                    }
                    Vec3::new(f.x, f.y, f.z).length()
                };
                r.strained[i] = if load > limit { r.strained[i] + dt } else { 0.0 };
                if r.strained[i] >= TEAR_AFTER_S {
                    tear.push((id.clone(), i));
                }
            }
        }
        for (id, i) in tear {
            self.tear_ragdoll_limb(&id, i);
        }
    }

    /// Lets go of every limb whose hand has gone too far from it: before each step.
    pub(crate) fn drop_far_ragdoll_grips(&mut self) {
        for g in self.ragdoll_grips.values_mut() {
            if g.joint.is_null() {
                continue;
            }
            let (lp, lr) = from_px_transform(&unsafe { (*(g.limb as *mut PxRigidDynamic)).get_global_pose() });
            if (lp + lr * g.point).distance(g.hand) > g.reach {
                unsafe { physx_sys::PxJoint_release_mut(as_joint(g.joint)) };
                g.joint = std::ptr::null_mut();
            }
        }
    }

    /// One step of the simulation and nothing else: for a world with no scene objects to keep
    /// in step with (the headset's own, which holds the level and what it simulates itself).
    pub fn step_simulation(&mut self, dt: f32) {
        self.drop_far_ragdoll_grips();
        if let Err(e) = self.scene.step(dt, None::<&mut physx_sys::PxBaseTask>, Some(&mut self.scratch), true) {
            log::warn!("rigid_physics: simulation step failed: {e:?}");
            return;
        }
        self.tear_strained_limbs(dt);
    }
}

pub(crate) type RagdollMap = HashMap<String, Ragdoll>;
pub(crate) type RagdollGripMap = HashMap<u32, RagdollGrip>;
