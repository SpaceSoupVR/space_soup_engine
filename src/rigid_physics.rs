//! Rigid-body physics via NVIDIA PhysX (EmbarkStudios' `physx`/`physx-sys`
//! bindings). Separate from `physics.rs`, which stays as the AABB
//! trigger/event pass for `is_trigger` objects — this module owns
//! `cuboid.position`/`.rotation` for any `GameObject` with a `rigid_body`.
//!
//! The type aliases and no-op simulation-event callback structs below are
//! required boilerplate: `PxScene`'s generic signature has ten type
//! parameters (including all four callback kinds) even though this module
//! doesn't use collision/trigger/wake/advance callbacks — copied from the
//! crate's own `examples/ball_physx.rs`.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use physx::prelude::*;
// `physx::prelude::*` also brings in a `Scene` trait, but our own
// `crate::scene::Scene` struct (imported by name below) shadows it — pull
// the trait in unnamed so its methods (`add_static_actor`, `step`, ...)
// still resolve without a naming collision.
use physx::scene::Scene as _;
// Needed for `.as_mut_ptr()`/`.as_ptr()` on the high-level wrapper types —
// not re-exported by the prelude.
use physx::traits::Class;
// `SceneFlags` (the bitflags container, as opposed to `SceneFlag` the enum
// which the prelude does re-export) isn't in the prelude either.
use physx::scene::SceneFlags;

use crate::events::Hand;
use crate::rig::PlayerRig;
use crate::scene::{BodyMode, ColliderShape, GameObject, GripKind, GripPointDef, RigidBodyDef, Scene};

// `physx`'s own "glam" feature interop is pinned to glam 0.23, a different
// (and thus incompatible, per Rust's type system) `Mat4`/`Vec3`/`Quat` from
// the 0.29 this crate otherwise uses everywhere — so these are hand-rolled
// against `PxVec3`/`PxQuat`/`PxTransform`'s own public constructors instead
// of relying on that feature (which is deliberately left off in Cargo.toml).
fn to_px_transform(pos: Vec3, rot: Quat) -> PxTransform {
    PxTransform::from_translation_rotation(
        &PxVec3::new(pos.x, pos.y, pos.z),
        &PxQuat::new(rot.x, rot.y, rot.z, rot.w),
    )
}

fn from_px_transform(t: &PxTransform) -> (Vec3, Quat) {
    let p = t.translation();
    let r = t.rotation();
    (Vec3::new(p.x(), p.y(), p.z()), Quat::from_xyzw(r.x(), r.y(), r.z(), r.w()))
}

fn to_px_vec3(v: [f32; 3]) -> PxVec3 {
    PxVec3::new(v[0], v[1], v[2])
}

/// kg/m³ — roughly wood/plastic-ish, used when an object's `rigid_body.mass`
/// is left unset so relative weights (a house heavier than a duck) come out
/// believable without hand-authoring every mass.
const DEFAULT_DENSITY: f32 = 500.0;

fn calculated_mass(half_size: Vec3, density: f32) -> f32 {
    let volume = (half_size.x * 2.0) * (half_size.y * 2.0) * (half_size.z * 2.0);
    (volume * density).max(0.001)
}

/// The joint-creation FFI (`phys_PxFixedJointCreate`/`phys_PxSphericalJointCreate`)
/// takes raw `physx_sys::PxTransform`, not the high-level wrapper — this
/// crate's own `From<PxTransform> for physx_sys::PxTransform` (`.into()`)
/// does the conversion, so this is just a short name for that at call sites.
fn to_raw_transform(t: PxTransform) -> physx_sys::PxTransform {
    t.into()
}

fn hand_index(hand: Hand) -> usize {
    match hand {
        Hand::Left => 0,
        Hand::Right => 1,
    }
}

type PxMaterial = physx::material::PxMaterial<()>;
type PxShape = physx::shape::PxShape<(), PxMaterial>;
type PxArticulationLink = physx::articulation_link::PxArticulationLink<(), PxShape>;
type PxRigidStatic = physx::rigid_static::PxRigidStatic<(), PxShape>;
type PxRigidDynamic = physx::rigid_dynamic::PxRigidDynamic<(), PxShape>;
type PxArticulationReducedCoordinate =
    physx::articulation_reduced_coordinate::PxArticulationReducedCoordinate<(), PxArticulationLink>;
// The scene's own user-data type param is `u64`, not `()` — `SceneDescriptor`
// packs user data by reinterpreting `&self.user_data` as a `*const *mut
// c_void` and dereferencing it when `size_of::<U>() <= size_of::<*mut
// c_void>()`. `()` has alignment 1, so that pointer isn't guaranteed 8-byte
// aligned and this segfaults (confirmed by hitting exactly that crash with
// `()`); `u64` has the matching size *and* alignment, so the same code path
// reads correctly. Actor-level user data (still `()` below) doesn't hit
// this — it goes through `UserData::init_user_data`, which writes into a
// real, already-aligned FFI struct field rather than reinterpreting a local
// stack variable's address.
type PxScene = physx::scene::PxScene<
    u64,
    PxArticulationLink,
    PxRigidStatic,
    PxRigidDynamic,
    PxArticulationReducedCoordinate,
    OnCollision,
    OnTrigger,
    OnConstraintBreak,
    OnWakeSleep,
    OnAdvance,
>;
type PxFoundation = PhysicsFoundation<physx::foundation::DefaultAllocator, PxShape>;

struct OnCollision;
impl CollisionCallback for OnCollision {
    fn on_collision(&mut self, _header: &physx_sys::PxContactPairHeader, _pairs: &[physx_sys::PxContactPair]) {}
}
struct OnTrigger;
impl TriggerCallback for OnTrigger {
    fn on_trigger(&mut self, _pairs: &[physx_sys::PxTriggerPair]) {}
}
struct OnConstraintBreak;
impl ConstraintBreakCallback for OnConstraintBreak {
    fn on_constraint_break(&mut self, _constraints: &[physx_sys::PxConstraintInfo]) {}
}
struct OnWakeSleep;
impl WakeSleepCallback<PxArticulationLink, PxRigidStatic, PxRigidDynamic> for OnWakeSleep {
    fn on_wake_sleep(
        &mut self,
        _actors: &[&physx::actor::ActorMap<PxArticulationLink, PxRigidStatic, PxRigidDynamic>],
        _is_waking: bool,
    ) {
    }
}
struct OnAdvance;
impl AdvanceCallback<PxArticulationLink, PxRigidDynamic> for OnAdvance {
    fn on_advance(
        &self,
        _actors: &[&physx::rigid_body::RigidBodyMap<PxArticulationLink, PxRigidDynamic>],
        _transforms: &[PxTransform],
    ) {
    }
}

fn gravity() -> PxVec3 { PxVec3::new(0.0, -9.81, 0.0) }

/// A tracked `Dynamic` actor — beyond the raw pointer used to read its pose
/// back each frame, this also remembers where it spawned so `step` can
/// teleport it back there on a timer when `respawn_interval` is set (see
/// `RigidBodyDef::respawn_interval`), making a falling object loop instead
/// of settling permanently.
struct DynamicActor {
    ptr: *mut PxRigidDynamic,
    spawn_pos: Vec3,
    spawn_rot: Quat,
    respawn_interval: Option<f32>,
    elapsed: f32,
}

/// One PhysX scene per `space_soup_engine::Scene` — rebuilt wholesale on
/// every `GameRuntime::load`/`load_scene` (matches how `players`/
/// `collisions`/`attachments`/`script_host` already fully reset there).
///
/// Actor pointers are captured *before* handing ownership to `scene` via
/// `add_dynamic_actor`/`add_static_actor` (those consume the `Owner<T>` and
/// hand it fully to the C++ scene — there is no handle returned). PhysX
/// never relocates actors once created, and `PhysicsWorld` owns the `scene`
/// that keeps them alive, so the pointers stay valid for exactly as long as
/// `self.scene` does; `rebuild` always replaces `scene` and the pointer maps
/// together as a unit, so no pointer can ever outlive its scene.
///
/// Field order matters here: Rust drops struct fields top-to-bottom (the
/// opposite of local variables, which drop bottom-to-top), and `scene`/
/// `materials` hold PhysX objects that reference `foundation` internally —
/// dropping `foundation` first crashes deep inside PhysX's own cleanup
/// (confirmed by reproducing it standalone). `foundation` must stay last.
/// An active grab — a physics joint between a hand anchor and the object's
/// actor, plus the grip point's name so `held_by` can report back which
/// point is currently held (for cosmetic hand-mesh alignment).
struct GrabState {
    joint: *mut physx_sys::PxJoint,
    point_name: String,
}

pub struct PhysicsWorld {
    scene: Owner<PxScene>,
    materials: Vec<Owner<PxMaterial>>,
    dynamic: HashMap<String, DynamicActor>,
    kinematic: HashMap<String, *mut PxRigidDynamic>,
    /// Small kinematic sphere actors, one per hand, pushed from
    /// `PlayerRig::hand_grip` every frame — grab joints connect to these
    /// rather than to any `GameObject`, since a hand isn't a scene object.
    hand_anchors: [*mut PxRigidDynamic; 2],
    grabs: HashMap<(String, Hand), GrabState>,
    scratch: ScratchBuffer,
    foundation: PxFoundation,
}

fn new_px_scene(foundation: &mut PxFoundation) -> Owner<PxScene> {
    foundation
        .create(SceneDescriptor {
            gravity: gravity(),
            thread_count: 1,
            // Continuous collision detection — without it, a fast-moving
            // `Dynamic` body (e.g. something that's fallen for a second or
            // two) can tunnel clean through thin geometry within a single
            // step of discrete collision detection (confirmed: the rifle
            // fell straight through the — very thin, 2cm — ground plane
            // without this). Scene-level flag plus the matching per-actor
            // `RigidBodyFlag::EnableCcd` (set in `spawn_actor`) are both
            // required for it to actually take effect.
            flags: SceneFlags::EnablePcm | SceneFlags::EnableCcd,
            ..SceneDescriptor::new(0u64)
        })
        .expect("space_soup_engine: failed to create PxScene")
}

/// Creates the two per-hand kinematic anchor actors used as grab-joint
/// endpoints, adding them to `scene` and pushing their material into
/// `materials` to keep it alive. Small sphere collider — this does mean a
/// hand anchor can nudge dynamic objects it passes through even without an
/// active grab, which reads as "your hand can push things," a reasonable
/// default rather than a bug.
fn create_hand_anchors(
    foundation: &mut PxFoundation,
    scene: &mut Owner<PxScene>,
    materials: &mut Vec<Owner<PxMaterial>>,
) -> [*mut PxRigidDynamic; 2] {
    let mut anchors = [std::ptr::null_mut(); 2];
    for hand in [Hand::Left, Hand::Right] {
        let Some(mut material) = foundation.create_material(0.0, 0.0, 0.0, ()) else {
            log::warn!("rigid_physics: failed to create hand-anchor material for {hand:?}");
            continue;
        };
        let geo = PxSphereGeometry::new(0.05);
        let Some(mut actor) = foundation.create_rigid_dynamic(
            PxTransform::default(), &geo, material.as_mut(), 1.0, PxTransform::default(), (),
        ) else {
            log::warn!("rigid_physics: failed to create hand anchor for {hand:?}");
            continue;
        };
        actor.set_rigid_body_flag(RigidBodyFlag::Kinematic, true);
        let ptr: *mut PxRigidDynamic = &mut *actor as *mut PxRigidDynamic;
        scene.add_dynamic_actor(actor);
        materials.push(material);
        anchors[hand_index(hand)] = ptr;
    }
    anchors
}

impl PhysicsWorld {
    pub fn new() -> Self {
        let mut foundation: PxFoundation = PhysicsFoundation::default();
        let mut scene = new_px_scene(&mut foundation);
        let mut materials = Vec::new();
        let hand_anchors = create_hand_anchors(&mut foundation, &mut scene, &mut materials);
        Self {
            foundation,
            scene,
            materials,
            dynamic: HashMap::new(),
            kinematic: HashMap::new(),
            hand_anchors,
            grabs: HashMap::new(),
            // SAFETY: freed on drop of the ScratchBuffer itself; must simply
            // outlive any in-flight `scene.step()` call, which it does as a
            // sibling field alongside `scene`.
            scratch: unsafe { ScratchBuffer::new(4) },
        }
    }

    /// Full teardown + recreate — simplest correct thing given scene
    /// switches are already a full reset everywhere else in `GameRuntime`.
    pub fn rebuild(&mut self, scene: &Scene) {
        self.dynamic.clear();
        self.kinematic.clear();
        self.materials.clear();
        // Joints and hand anchors from the old scene are destroyed along
        // with it (see `grabs` clear below) — no individual release needed,
        // same reasoning as `dynamic`/`kinematic`'s plain `.clear()`.
        self.grabs.clear();
        self.scene = new_px_scene(&mut self.foundation);
        self.hand_anchors = create_hand_anchors(&mut self.foundation, &mut self.scene, &mut self.materials);

        for obj in &scene.objects {
            let Some(def) = &obj.rigid_body else { continue };
            self.spawn_actor(obj, def);
        }
    }

    fn spawn_actor(&mut self, obj: &GameObject, def: &RigidBodyDef) {
        let transform = to_px_transform(obj.cuboid.position, obj.cuboid.rotation);
        let mass = def.mass.unwrap_or_else(|| calculated_mass(obj.cuboid.half_size, DEFAULT_DENSITY));
        let collider_half = def.collider_half_size.map(Vec3::from).unwrap_or(obj.cuboid.half_size);
        let shape_transform = to_px_transform(Vec3::from(def.collider_offset), Quat::IDENTITY);

        let Some(mut material) =
            self.foundation.create_material(def.friction, def.friction, def.restitution, ())
        else {
            log::warn!("rigid_physics: failed to create material for '{}'", obj.id);
            return;
        };

        match def.mode {
            BodyMode::Static => {
                let created = match def.shape {
                    ColliderShape::Box => {
                        let geo = PxBoxGeometry::new(collider_half.x, collider_half.y, collider_half.z);
                        self.foundation.create_rigid_static(transform, &geo, material.as_mut(), shape_transform, ())
                    }
                    ColliderShape::Sphere { radius } => {
                        let geo = PxSphereGeometry::new(radius);
                        self.foundation.create_rigid_static(transform, &geo, material.as_mut(), shape_transform, ())
                    }
                    ColliderShape::Capsule { radius, half_height } => {
                        let geo = PxCapsuleGeometry::new(radius, half_height);
                        self.foundation.create_rigid_static(transform, &geo, material.as_mut(), shape_transform, ())
                    }
                };
                match created {
                    Some(actor) => self.scene.add_static_actor(actor),
                    None => log::warn!("rigid_physics: failed to create static actor for '{}'", obj.id),
                }
            }
            BodyMode::Kinematic | BodyMode::Dynamic => {
                let created = match def.shape {
                    ColliderShape::Box => {
                        let geo = PxBoxGeometry::new(collider_half.x, collider_half.y, collider_half.z);
                        self.foundation.create_rigid_dynamic(transform, &geo, material.as_mut(), 1.0, shape_transform, ())
                    }
                    ColliderShape::Sphere { radius } => {
                        let geo = PxSphereGeometry::new(radius);
                        self.foundation.create_rigid_dynamic(transform, &geo, material.as_mut(), 1.0, shape_transform, ())
                    }
                    ColliderShape::Capsule { radius, half_height } => {
                        let geo = PxCapsuleGeometry::new(radius, half_height);
                        self.foundation.create_rigid_dynamic(transform, &geo, material.as_mut(), 1.0, shape_transform, ())
                    }
                };
                match created {
                    Some(mut actor) => {
                        actor.set_mass(mass);
                        if def.mode == BodyMode::Kinematic {
                            actor.set_rigid_body_flag(RigidBodyFlag::Kinematic, true);
                            let ptr: *mut PxRigidDynamic = &mut *actor as *mut PxRigidDynamic;
                            self.scene.add_dynamic_actor(actor);
                            self.kinematic.insert(obj.id.clone(), ptr);
                        } else {
                            actor.set_rigid_body_flag(RigidBodyFlag::EnableCcd, true);
                            // Default (4, 1) position/velocity solver iterations are too
                            // weak for small/thin dynamic shapes against the scene's much
                            // larger static geometry (confirmed: small props were sinking
                            // partway through the ground on contact instead of resting) —
                            // more position iterations resolve penetration more firmly.
                            actor.set_solver_iteration_counts(8, 2);
                            let vel = to_px_vec3(def.linear_velocity);
                            actor.set_linear_velocity(&vel, true);
                            let ptr: *mut PxRigidDynamic = &mut *actor as *mut PxRigidDynamic;
                            self.scene.add_dynamic_actor(actor);
                            self.dynamic.insert(obj.id.clone(), DynamicActor {
                                ptr,
                                spawn_pos: obj.cuboid.position,
                                spawn_rot: obj.cuboid.rotation,
                                respawn_interval: def.respawn_interval,
                                elapsed: 0.0,
                            });
                        }
                    }
                    None => log::warn!("rigid_physics: failed to create dynamic actor for '{}'", obj.id),
                }
            }
        }

        self.materials.push(material);
    }

    /// Grabs `object_id` at the named `point` with `hand` — creates a fixed
    /// (`GripKind::Snap`) or spherical (`GripKind::Free`) joint between that
    /// hand's anchor actor and the object's actor, local to `point`'s
    /// offset. No-ops (with a warning) if the object isn't a tracked
    /// `Dynamic` body. Grabbing the same `(object_id, hand)` again first
    /// releases the previous joint.
    pub fn grab(&mut self, object_id: &str, hand: Hand, point: &GripPointDef) {
        self.release(object_id, hand);

        let Some(state) = self.dynamic.get(object_id) else {
            log::warn!("rigid_physics: grab '{object_id}' at '{}' failed — not a tracked Dynamic body", point.name);
            return;
        };
        let anchor_ptr = self.hand_anchors[hand_index(hand)];
        if anchor_ptr.is_null() {
            log::warn!("rigid_physics: grab '{object_id}' failed — no hand anchor for {hand:?}");
            return;
        }

        let anchor_frame = to_raw_transform(PxTransform::default());
        let local_rot = point.local_rot;
        let object_frame = to_raw_transform(PxTransform::from_translation_rotation(
            &to_px_vec3(point.local_pos),
            &PxQuat::new(local_rot[0], local_rot[1], local_rot[2], local_rot[3]),
        ));

        // SAFETY: `anchor_ptr`/`state.ptr` are both actors owned by
        // `self.scene`, guaranteed alive for this call (see struct doc
        // comment); the joint create functions borrow them only for the
        // duration of the call and return an owned `*mut PxJoint` this
        // struct is responsible for releasing (`release`, or implicitly via
        // scene teardown in `rebuild`).
        let joint = unsafe {
            match point.kind {
                GripKind::Snap => physx_sys::phys_PxFixedJointCreate(
                    self.foundation.as_mut_ptr(),
                    anchor_ptr as *mut physx_sys::PxRigidActor,
                    &anchor_frame,
                    state.ptr as *mut physx_sys::PxRigidActor,
                    &object_frame,
                ) as *mut physx_sys::PxJoint,
                GripKind::Free => physx_sys::phys_PxSphericalJointCreate(
                    self.foundation.as_mut_ptr(),
                    anchor_ptr as *mut physx_sys::PxRigidActor,
                    &anchor_frame,
                    state.ptr as *mut physx_sys::PxRigidActor,
                    &object_frame,
                ) as *mut physx_sys::PxJoint,
            }
        };

        if joint.is_null() {
            log::warn!("rigid_physics: joint creation failed for '{object_id}' at '{}'", point.name);
            return;
        }

        self.grabs.insert((object_id.to_string(), hand), GrabState { joint, point_name: point.name.clone() });
    }

    /// Releases `hand`'s grab on `object_id`, if any.
    pub fn release(&mut self, object_id: &str, hand: Hand) {
        if let Some(state) = self.grabs.remove(&(object_id.to_string(), hand)) {
            // SAFETY: `state.joint` was created by `grab` and never handed
            // out elsewhere; releasing it here is the sole owner's release.
            unsafe { physx_sys::PxJoint_release_mut(state.joint) };
        }
    }

    /// What `hand` currently has grabbed, if anything — `(object_id,
    /// point_name)`. Used to keep the rendered hand mesh aligned to the
    /// grip it's actually holding.
    pub fn held_by(&self, hand: Hand) -> Option<(&str, &str)> {
        self.grabs.iter()
            .find(|((_, h), _)| *h == hand)
            .map(|((id, _), state)| (id.as_str(), state.point_name.as_str()))
    }

    /// Pushes current `Kinematic` transforms and hand-anchor positions into
    /// PhysX, steps the simulation, then writes `Dynamic` results back into
    /// `scene`. Called from `GameRuntime::update` right before render-list
    /// collection, so it always has the final say over
    /// `cuboid.position`/`.rotation` for any object with a `rigid_body` —
    /// see that call site for the script-write-vs-physics ordering
    /// rationale.
    pub fn step(&mut self, dt: f32, scene: &mut Scene, rig: &PlayerRig) {
        for (id, &ptr) in &self.kinematic {
            let Some(obj) = scene.find_object(id) else { continue };
            let target = to_px_transform(obj.cuboid.position, obj.cuboid.rotation);
            // SAFETY: `ptr` was captured from an actor now owned by
            // `self.scene`, which is guaranteed alive for the duration of
            // this call (see struct doc comment).
            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        for hand in [Hand::Left, Hand::Right] {
            let ptr = self.hand_anchors[hand_index(hand)];
            if ptr.is_null() { continue; }
            let grip = rig.hand_grip(hand);
            let target = to_px_transform(grip.position, grip.rotation);
            // SAFETY: see above.
            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        let zero = PxVec3::new(0.0, 0.0, 0.0);
        for state in self.dynamic.values_mut() {
            // Safety net: an unlucky contact (e.g. a Kinematic body like the
            // wandering duck clipping a small/light prop) can occasionally
            // impart enough velocity to launch a `Dynamic` body clean off
            // the playable area — once it's outside every collider's
            // footprint it just free-falls forever with nothing left to
            // catch it, i.e. "disappears". Rather than chase every possible
            // source of an energetic contact, guarantee it can never be
            // lost for good: falling below a generous kill plane always
            // teleports back to spawn, exactly like the timed
            // `respawn_interval` reset below.
            const KILL_Y: f32 = -15.0;
            // SAFETY: see above.
            let fell_out = unsafe { (*state.ptr).get_global_pose() }.translation().y() < KILL_Y;

            let due_for_timed_respawn = state.respawn_interval.map(|interval| {
                state.elapsed += dt;
                state.elapsed >= interval
            }).unwrap_or(false);

            if !fell_out && !due_for_timed_respawn { continue; }
            state.elapsed = 0.0;
            let spawn = to_px_transform(state.spawn_pos, state.spawn_rot);
            // SAFETY: see above.
            unsafe {
                (*state.ptr).set_global_pose(&spawn, true);
                (*state.ptr).set_linear_velocity(&zero, true);
                (*state.ptr).set_angular_velocity(&zero, true);
            }
        }

        if let Err(e) = self.scene.step(dt, None::<&mut physx_sys::PxBaseTask>, Some(&mut self.scratch), true) {
            log::warn!("rigid_physics: simulation step failed: {e:?}");
            return;
        }

        for (id, state) in &self.dynamic {
            // SAFETY: see above.
            let pose = unsafe { (*state.ptr).get_global_pose() };
            let (pos, rot) = from_px_transform(&pose);
            if let Some(obj) = scene.find_object_mut(id) {
                obj.cuboid.position = pos;
                obj.cuboid.rotation = rot;
            }
        }
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self { Self::new() }
}
