use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use physx::prelude::*;

use physx::scene::Scene as _;

use physx::traits::Class;

use physx::scene::SceneFlags;

use physx::cooking::{
    create_triangle_mesh, PxCookingParams, PxTriangleMeshDesc, TriangleMeshCookingResult,
};
use physx::triangle_mesh::TriangleMesh;

use space_soup_protocol::PlayerId;

use crate::events::Hand;
use crate::rig::PlayerRig;
use crate::scene::{
    BodyMode, ColliderShape, GameObject, GripKind, GripPointDef, RigidBodyDef, Scene,
    TerrainColliderDef,
};

fn to_px_transform(pos: Vec3, rot: Quat) -> PxTransform {
    PxTransform::from_translation_rotation(
        &PxVec3::new(pos.x, pos.y, pos.z),
        &PxQuat::new(rot.x, rot.y, rot.z, rot.w),
    )
}

fn from_px_transform(t: &PxTransform) -> (Vec3, Quat) {
    let p = t.translation();
    let r = t.rotation();
    (
        Vec3::new(p.x(), p.y(), p.z()),
        Quat::from_xyzw(r.x(), r.y(), r.z(), r.w()),
    )
}

fn to_px_vec3(v: [f32; 3]) -> PxVec3 {
    PxVec3::new(v[0], v[1], v[2])
}

const DEFAULT_DENSITY: f32 = 500.0;

fn calculated_mass(half_size: Vec3, density: f32) -> f32 {
    let volume = (half_size.x * 2.0) * (half_size.y * 2.0) * (half_size.z * 2.0);
    (volume * density).max(0.001)
}

fn to_raw_transform(t: PxTransform) -> physx_sys::PxTransform {
    t.into()
}

type PxMaterial = physx::material::PxMaterial<()>;
type PxShape = physx::shape::PxShape<(), PxMaterial>;
type PxArticulationLink = physx::articulation_link::PxArticulationLink<(), PxShape>;
type PxRigidStatic = physx::rigid_static::PxRigidStatic<(), PxShape>;
type PxRigidDynamic = physx::rigid_dynamic::PxRigidDynamic<(), PxShape>;
type PxArticulationReducedCoordinate =
    physx::articulation_reduced_coordinate::PxArticulationReducedCoordinate<(), PxArticulationLink>;

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
    fn on_collision(
        &mut self,
        _header: &physx_sys::PxContactPairHeader,
        _pairs: &[physx_sys::PxContactPair],
    ) {
    }
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

fn gravity() -> PxVec3 {
    PxVec3::new(0.0, -9.81, 0.0)
}

struct DynamicActor {
    ptr: *mut PxRigidDynamic,
    spawn_pos: Vec3,
    spawn_rot: Quat,
    respawn_interval: Option<f32>,
    elapsed: f32,
}

struct GrabState {
    joint: *mut physx_sys::PxJoint,
    point_name: String,
}

#[derive(Clone, Copy)]
struct Drive {
    stiffness: f32,
    damping: f32,
}

pub struct PhysicsWorld {
    scene: Owner<PxScene>,
    materials: Vec<Owner<PxMaterial>>,
    dynamic: HashMap<String, DynamicActor>,
    kinematic: HashMap<String, *mut PxRigidDynamic>,

    /// One kinematic anchor per (player, hand), created lazily on first
    /// sight of that player via `ensure_player` rather than eagerly for a
    /// fixed player count — this is what lets N simultaneous players each
    /// grab things with PhysX instead of just one.
    hand_anchors: HashMap<(PlayerId, Hand), *mut PxRigidDynamic>,
    grabs: HashMap<(PlayerId, String, Hand), GrabState>,
    scratch: ScratchBuffer,
    foundation: PxFoundation,
    terrain_meshes: Vec<Owner<TriangleMesh>>,
}

fn new_px_scene(foundation: &mut PxFoundation) -> Owner<PxScene> {
    foundation
        .create(SceneDescriptor {
            gravity: gravity(),
            thread_count: 1,

            flags: SceneFlags::EnablePcm | SceneFlags::EnableCcd,
            ..SceneDescriptor::new(0u64)
        })
        .expect("space_soup_engine: failed to create PxScene")
}

fn create_hand_anchor(
    foundation: &mut PxFoundation,
    scene: &mut Owner<PxScene>,
    materials: &mut Vec<Owner<PxMaterial>>,
    player: PlayerId,
    hand: Hand,
) -> Option<*mut PxRigidDynamic> {
    let Some(mut material) = foundation.create_material(0.0, 0.0, 0.0, ()) else {
        log::warn!("rigid_physics: failed to create hand-anchor material for {player:?}/{hand:?}");
        return None;
    };
    let geo = PxSphereGeometry::new(0.035);
    let Some(mut actor) = foundation.create_rigid_dynamic(
        PxTransform::default(),
        &geo,
        material.as_mut(),
        1.0,
        PxTransform::default(),
        (),
    ) else {
        log::warn!("rigid_physics: failed to create hand anchor for {player:?}/{hand:?}");
        return None;
    };
    actor.set_rigid_body_flag(RigidBodyFlag::Kinematic, true);
    // Hand anchors are a physics proxy for holding/dragging grabbed objects,
    // not real world geometry — without this, player-locomotion scene
    // queries (ground-follow, wall-collision) can raycast straight into the
    // player's own hand (routinely right in front of their chest) and
    // mistake it for solid ground/a wall, freezing all movement.
    for shape in actor.get_shapes_mut() {
        shape.set_flag(ShapeFlag::SceneQueryShape, false);
    }
    let ptr: *mut PxRigidDynamic = &mut *actor as *mut PxRigidDynamic;
    scene.add_dynamic_actor(actor);
    materials.push(material);
    Some(ptr)
}

/// Walks a glTF node tree collecting `(mesh_index, world_matrix)` for every node with a mesh
/// whose name starts with `node_filter` (all mesh nodes, if `node_filter` is `None`).
fn collect_terrain_instances(doc: &gltf::Document, node_filter: Option<&str>) -> Vec<(usize, Mat4)> {
    fn walk(node: gltf::Node, parent: Mat4, filter: Option<&str>, out: &mut Vec<(usize, Mat4)>) {
        let local = Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent * local;

        if let Some(mesh) = node.mesh() {
            let matches = match filter {
                Some(f) => node.name().is_some_and(|n| n.starts_with(f)),
                None => true,
            };
            if matches {
                out.push((mesh.index(), world));
            }
        }

        for child in node.children() {
            walk(child, world, filter, out);
        }
    }

    let mut out = Vec::new();
    for scene in doc.scenes() {
        for node in scene.nodes() {
            walk(node, Mat4::IDENTITY, node_filter, &mut out);
        }
    }
    out
}

/// Reads a mesh's raw local-space (untransformed) positions and triangle indices, concatenated
/// across all of its primitives.
fn read_mesh_geometry(mesh: &gltf::Mesh, buffers: &[gltf::buffer::Data]) -> (Vec<PxVec3>, Vec<u32>) {
    let mut points = Vec::new();
    let mut indices = Vec::new();

    for prim in mesh.primitives() {
        let reader = prim.reader(|b| Some(&buffers[b.index()]));
        let base = points.len() as u32;

        let Some(pos_iter) = reader.read_positions() else {
            continue;
        };
        for p in pos_iter {
            points.push(PxVec3::new(p[0], p[1], p[2]));
        }

        let point_count = points.len() as u32 - base;
        let prim_indices: Vec<u32> = match reader.read_indices() {
            Some(it) => it.into_u32().collect(),
            None => (0..point_count).collect(),
        };
        indices.extend(prim_indices.into_iter().map(|i| base + i));
    }

    (points, indices)
}

/// Cooks a static triangle mesh from raw local-space geometry, ready to be instanced by any
/// number of `PxTriangleMeshGeometry` shapes with per-instance position/rotation/scale.
fn cook_triangle_mesh(
    foundation: &mut PxFoundation,
    points: &[PxVec3],
    indices: &[u32],
) -> Option<Owner<TriangleMesh>> {
    let params = PxCookingParams::new(foundation)?;

    let mut desc = PxTriangleMeshDesc::new();
    desc.obj.points.count = points.len() as u32;
    desc.obj.points.stride = std::mem::size_of::<PxVec3>() as u32;
    desc.obj.points.data = points.as_ptr() as *const std::ffi::c_void;

    desc.obj.triangles.count = (indices.len() / 3) as u32;
    desc.obj.triangles.stride = (std::mem::size_of::<u32>() * 3) as u32;
    desc.obj.triangles.data = indices.as_ptr() as *const std::ffi::c_void;

    match create_triangle_mesh(foundation, &params, &desc) {
        TriangleMeshCookingResult::Success(mesh) => Some(mesh),
        _ => None,
    }
}

impl PhysicsWorld {
    pub fn new() -> Self {
        let mut foundation: PxFoundation = PhysicsFoundation::default();
        let scene = new_px_scene(&mut foundation);
        Self {
            foundation,
            scene,
            materials: Vec::new(),
            dynamic: HashMap::new(),
            kinematic: HashMap::new(),
            hand_anchors: HashMap::new(),
            grabs: HashMap::new(),

            scratch: unsafe { ScratchBuffer::new(4) },
            terrain_meshes: Vec::new(),
        }
    }

    /// Ensures `player` has PhysX kinematic hand anchors, creating them on
    /// first sight. Idempotent — safe to call every tick for every active
    /// player (`GameRuntime::update` does exactly that).
    pub fn ensure_player(&mut self, player: PlayerId) {
        for hand in [Hand::Left, Hand::Right] {
            if self.hand_anchors.contains_key(&(player, hand)) {
                continue;
            }
            if let Some(ptr) = create_hand_anchor(
                &mut self.foundation,
                &mut self.scene,
                &mut self.materials,
                player,
                hand,
            ) {
                self.hand_anchors.insert((player, hand), ptr);
            }
        }
    }

    /// Tears down `player`'s hand anchors (and releases any grab they were
    /// holding first, so we don't leave a joint referencing a freed actor) —
    /// call this when a player disconnects, or their anchors/joints just sit
    /// in the scene forever wasting memory.
    pub fn remove_player(&mut self, player: PlayerId) {
        let held_by_player: Vec<(PlayerId, String, Hand)> = self
            .grabs
            .keys()
            .filter(|(p, _, _)| *p == player)
            .cloned()
            .collect();
        for key in held_by_player {
            if let Some(state) = self.grabs.remove(&key) {
                unsafe { physx_sys::PxJoint_release_mut(state.joint) };
            }
        }

        for hand in [Hand::Left, Hand::Right] {
            let Some(ptr) = self.hand_anchors.remove(&(player, hand)) else {
                continue;
            };
            if ptr.is_null() {
                continue;
            }
            // Detach from the scene, then release the actor itself — PhysX's
            // `removeActor` only unregisters it from simulation, it doesn't
            // free the object.
            unsafe {
                self.scene.remove_actor(&mut *ptr, false);
                physx_sys::PxActor_release_mut(ptr as *mut physx_sys::PxActor);
            }
        }
    }

    pub fn rebuild(&mut self, scene: &Scene, game_dir: &Path) {
        self.dynamic.clear();
        self.kinematic.clear();
        self.materials.clear();
        self.terrain_meshes.clear();

        self.grabs.clear();
        self.hand_anchors.clear();
        self.scene = new_px_scene(&mut self.foundation);

        for obj in &scene.objects {
            let Some(def) = &obj.rigid_body else { continue };
            self.spawn_actor(obj, def);
        }

        for obj in &scene.objects {
            let Some(def) = &obj.slider_joint else {
                continue;
            };
            self.spawn_slider_joint(obj, def);
        }

        for obj in &scene.objects {
            let Some(def) = &obj.terrain_collider else {
                continue;
            };
            self.spawn_terrain_colliders(obj, def, game_dir);
        }
    }

    fn spawn_actor(&mut self, obj: &GameObject, def: &RigidBodyDef) {
        let transform = to_px_transform(obj.cuboid.position, obj.cuboid.rotation);
        let mass = def
            .mass
            .unwrap_or_else(|| calculated_mass(obj.cuboid.half_size, DEFAULT_DENSITY));
        let collider_half = def
            .collider_half_size
            .map(Vec3::from)
            .unwrap_or(obj.cuboid.half_size);
        let shape_transform = to_px_transform(Vec3::from(def.collider_offset), Quat::IDENTITY);

        let Some(mut material) =
            self.foundation
                .create_material(def.friction, def.friction, def.restitution, ())
        else {
            log::warn!("rigid_physics: failed to create material for '{}'", obj.id);
            return;
        };

        match def.mode {
            BodyMode::Static => {
                let created = match def.shape {
                    ColliderShape::Box => {
                        let geo =
                            PxBoxGeometry::new(collider_half.x, collider_half.y, collider_half.z);
                        self.foundation.create_rigid_static(
                            transform,
                            &geo,
                            material.as_mut(),
                            shape_transform,
                            (),
                        )
                    }
                    ColliderShape::Sphere { radius } => {
                        let geo = PxSphereGeometry::new(radius);
                        self.foundation.create_rigid_static(
                            transform,
                            &geo,
                            material.as_mut(),
                            shape_transform,
                            (),
                        )
                    }
                    ColliderShape::Capsule {
                        radius,
                        half_height,
                    } => {
                        let geo = PxCapsuleGeometry::new(radius, half_height);
                        self.foundation.create_rigid_static(
                            transform,
                            &geo,
                            material.as_mut(),
                            shape_transform,
                            (),
                        )
                    }
                };
                match created {
                    Some(actor) => self.scene.add_static_actor(actor),
                    None => log::warn!(
                        "rigid_physics: failed to create static actor for '{}'",
                        obj.id
                    ),
                }
            }
            BodyMode::Kinematic | BodyMode::Dynamic => {
                let created = match def.shape {
                    ColliderShape::Box => {
                        let geo =
                            PxBoxGeometry::new(collider_half.x, collider_half.y, collider_half.z);
                        self.foundation.create_rigid_dynamic(
                            transform,
                            &geo,
                            material.as_mut(),
                            1.0,
                            shape_transform,
                            (),
                        )
                    }
                    ColliderShape::Sphere { radius } => {
                        let geo = PxSphereGeometry::new(radius);
                        self.foundation.create_rigid_dynamic(
                            transform,
                            &geo,
                            material.as_mut(),
                            1.0,
                            shape_transform,
                            (),
                        )
                    }
                    ColliderShape::Capsule {
                        radius,
                        half_height,
                    } => {
                        let geo = PxCapsuleGeometry::new(radius, half_height);
                        self.foundation.create_rigid_dynamic(
                            transform,
                            &geo,
                            material.as_mut(),
                            1.0,
                            shape_transform,
                            (),
                        )
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

                            actor.set_solver_iteration_counts(8, 2);
                            let vel = to_px_vec3(def.linear_velocity);
                            actor.set_linear_velocity(&vel, true);
                            let ptr: *mut PxRigidDynamic = &mut *actor as *mut PxRigidDynamic;
                            self.scene.add_dynamic_actor(actor);
                            self.dynamic.insert(
                                obj.id.clone(),
                                DynamicActor {
                                    ptr,
                                    spawn_pos: obj.cuboid.position,
                                    spawn_rot: obj.cuboid.rotation,
                                    respawn_interval: def.respawn_interval,
                                    elapsed: 0.0,
                                },
                            );
                        }
                    }
                    None => log::warn!(
                        "rigid_physics: failed to create dynamic actor for '{}'",
                        obj.id
                    ),
                }
            }
        }

        self.materials.push(material);
    }

    fn spawn_slider_joint(&mut self, obj: &GameObject, def: &crate::scene::SliderJointDef) {
        let Some(child) = self.dynamic.get(&obj.id) else {
            log::warn!(
                "rigid_physics: slider_joint on '{}' failed — not a tracked Dynamic body",
                obj.id
            );
            return;
        };
        let Some(parent) = self.dynamic.get(&def.parent) else {
            log::warn!("rigid_physics: slider_joint on '{}' failed — parent '{}' is not a tracked Dynamic body", obj.id, def.parent);
            return;
        };

        let axis = Vec3::from(def.axis);
        if axis.length_squared() < 1e-6 {
            log::warn!(
                "rigid_physics: slider_joint on '{}' has a degenerate axis {:?}",
                obj.id,
                def.axis
            );
            return;
        }
        let axis = axis.normalize();
        let frame_rot = Quat::from_rotation_arc(Vec3::X, axis);
        let frame = to_raw_transform(PxTransform::from_translation_rotation(
            &PxVec3::new(0.0, 0.0, 0.0),
            &PxQuat::new(frame_rot.x, frame_rot.y, frame_rot.z, frame_rot.w),
        ));

        let joint = unsafe {
            physx_sys::phys_PxD6JointCreate(
                self.foundation.as_mut_ptr(),
                parent.ptr as *mut physx_sys::PxRigidActor,
                &frame,
                child.ptr as *mut physx_sys::PxRigidActor,
                &frame,
            )
        };
        if joint.is_null() {
            log::warn!(
                "rigid_physics: D6 slider joint creation failed for '{}'",
                obj.id
            );
            return;
        }

        unsafe {
            physx_sys::PxD6Joint_setMotion_mut(
                joint,
                physx_sys::PxD6Axis::X,
                physx_sys::PxD6Motion::Limited,
            );
            physx_sys::PxD6Joint_setMotion_mut(
                joint,
                physx_sys::PxD6Axis::Y,
                physx_sys::PxD6Motion::Limited,
            );
            physx_sys::PxD6Joint_setMotion_mut(
                joint,
                physx_sys::PxD6Axis::Z,
                physx_sys::PxD6Motion::Limited,
            );
            physx_sys::PxD6Joint_setMotion_mut(
                joint,
                physx_sys::PxD6Axis::Twist,
                physx_sys::PxD6Motion::Locked,
            );
            physx_sys::PxD6Joint_setMotion_mut(
                joint,
                physx_sys::PxD6Axis::Swing1,
                physx_sys::PxD6Motion::Locked,
            );
            physx_sys::PxD6Joint_setMotion_mut(
                joint,
                physx_sys::PxD6Axis::Swing2,
                physx_sys::PxD6Motion::Locked,
            );

            let hard_spring = physx_sys::PxSpring_new(0.0, 0.0);
            let limit =
                physx_sys::PxJointLinearLimitPair_new_1(0.0, def.travel.max(0.001), &hard_spring);
            physx_sys::PxD6Joint_setLinearLimit_mut(joint, physx_sys::PxD6Axis::X, &limit);

            let side_spring = physx_sys::PxSpring_new(4000.0, 120.0);
            let side_limit = physx_sys::PxJointLinearLimitPair_new_1(-0.0005, 0.0005, &side_spring);
            physx_sys::PxD6Joint_setLinearLimit_mut(joint, physx_sys::PxD6Axis::Y, &side_limit);
            physx_sys::PxD6Joint_setLinearLimit_mut(joint, physx_sys::PxD6Axis::Z, &side_limit);

            let drive = physx_sys::PxD6JointDrive_new_1(
                def.spring_stiffness,
                def.spring_damping,
                1.0e6,
                false,
            );
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::X, &drive);
            let rest_pose = to_raw_transform(PxTransform::default());
            physx_sys::PxD6Joint_setDrivePosition_mut(joint, &rest_pose, true);
        }
    }

    fn spawn_terrain_colliders(&mut self, obj: &GameObject, def: &TerrainColliderDef, game_dir: &Path) {
        let Some(mesh_ref) = &obj.mesh else {
            log::warn!(
                "rigid_physics: terrain_collider on '{}' has no mesh to source geometry from",
                obj.id
            );
            return;
        };

        let full_path = game_dir.join(&mesh_ref.path);
        let (doc, buffers, _images) = match gltf::import(&full_path) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "rigid_physics: terrain_collider on '{}' failed to load '{}': {e}",
                    obj.id,
                    full_path.display()
                );
                return;
            }
        };

        let instances = collect_terrain_instances(&doc, def.node_filter.as_deref());
        if instances.is_empty() {
            log::warn!(
                "rigid_physics: terrain_collider on '{}' matched no nodes (node_filter {:?})",
                obj.id,
                def.node_filter
            );
            return;
        }
        log::info!(
            "rigid_physics: terrain_collider on '{}' matched {} node instance(s) (node_filter {:?})",
            obj.id,
            instances.len(),
            def.node_filter
        );

        let Some(mut material) = self.foundation.create_material(0.8, 0.8, 0.0, ()) else {
            log::warn!(
                "rigid_physics: failed to create terrain material for '{}'",
                obj.id
            );
            return;
        };

        let object_mat = Mat4::from_scale_rotation_translation(
            mesh_ref.scale,
            obj.cuboid.rotation * mesh_ref.rotation_offset,
            obj.cuboid.position,
        );

        let mut cooked: HashMap<usize, usize> = HashMap::new();
        let mut spawned = 0u32;

        for (mesh_index, node_mat) in instances {
            let mesh_idx_in_pool = if let Some(&i) = cooked.get(&mesh_index) {
                i
            } else {
                let mesh = doc.meshes().nth(mesh_index).expect("mesh index from node tree");
                let (points, tri_indices) = read_mesh_geometry(&mesh, &buffers);
                if points.is_empty() || tri_indices.is_empty() {
                    log::warn!(
                        "rigid_physics: terrain_collider on '{}' found an empty mesh (index {mesh_index})",
                        obj.id
                    );
                    continue;
                }
                match cook_triangle_mesh(&mut self.foundation, &points, &tri_indices) {
                    Some(owned) => {
                        self.terrain_meshes.push(owned);
                        let i = self.terrain_meshes.len() - 1;
                        cooked.insert(mesh_index, i);
                        i
                    }
                    None => {
                        log::warn!(
                            "rigid_physics: terrain_collider on '{}' failed to cook mesh (index {mesh_index})",
                            obj.id
                        );
                        continue;
                    }
                }
            };

            let world = object_mat * node_mat;
            let (scale, rotation, translation) = world.to_scale_rotation_translation();

            let scale_px = PxVec3::new(scale.x, scale.y, scale.z);
            let rot_px = PxQuat::new(rotation.x, rotation.y, rotation.z, rotation.w);
            let mesh_scale = unsafe { physx_sys::PxMeshScale_new_3(scale_px.as_ptr(), rot_px.as_ptr()) };

            let geo = PxTriangleMeshGeometry::new(
                self.terrain_meshes[mesh_idx_in_pool].as_mut(),
                &mesh_scale,
                MeshGeometryFlags::empty(),
            );

            let transform = to_px_transform(translation, rotation);
            match self.foundation.create_rigid_static(
                transform,
                &geo,
                material.as_mut(),
                PxTransform::default(),
                (),
            ) {
                Some(actor) => {
                    self.scene.add_static_actor(actor);
                    spawned += 1;
                }
                None => log::warn!(
                    "rigid_physics: terrain_collider on '{}' failed to create static actor",
                    obj.id
                ),
            }
        }
        log::info!(
            "rigid_physics: terrain_collider on '{}' spawned {spawned} static collider(s) from {} unique cooked mesh(es)",
            obj.id,
            cooked.len()
        );

        self.materials.push(material);
    }

    pub fn raycast_down(&self, origin: Vec3, max_distance: f32) -> Option<(Vec3, Vec3)> {
        self.raycast(origin, Vec3::NEG_Y, max_distance)
    }

    /// General-direction single raycast against every collider in the
    /// scene (static and dynamic) — `raycast_down` is just this with a
    /// fixed downward direction; player wall-collision uses this directly
    /// with a horizontal direction instead.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<(Vec3, Vec3)> {
        let origin_px = physx_sys::PxVec3 {
            x: origin.x,
            y: origin.y,
            z: origin.z,
        };
        let dir_px = physx_sys::PxVec3 {
            x: dir.x,
            y: dir.y,
            z: dir.z,
        };
        let mut hit: physx_sys::PxRaycastHit = unsafe { std::mem::zeroed() };
        let hit_flags = physx_sys::PxHitFlags::Position | physx_sys::PxHitFlags::Normal;
        // filterData is taken by reference (never null) in the C++ API — a real default-
        // constructed value must be passed, unlike filterCall/cache which tolerate null.
        let filter_data = unsafe { physx_sys::PxQueryFilterData_new() };
        let found = unsafe {
            physx_sys::PxSceneQueryExt_raycastSingle(
                self.scene.as_ptr(),
                &origin_px,
                &dir_px,
                max_distance,
                hit_flags,
                &mut hit,
                &filter_data,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if !found {
            return None;
        }
        let p = hit.position;
        let n = hit.normal;
        Some((Vec3::new(p.x, p.y, p.z), Vec3::new(n.x, n.y, n.z)))
    }

    pub fn grab(&mut self, player: PlayerId, object_id: &str, hand: Hand, point: &GripPointDef) {
        self.release(player, object_id, hand);

        // Blocks a second hand from grabbing the same spot on the same
        // object — whether that's this player's other hand (as before
        // multiplayer) or a different player's hand entirely.
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

    pub fn step(&mut self, dt: f32, scene: &mut Scene, rigs: &HashMap<PlayerId, PlayerRig>) {
        for (id, &ptr) in &self.kinematic {
            let Some(obj) = scene.find_object(id) else {
                continue;
            };
            let target = to_px_transform(obj.cuboid.position, obj.cuboid.rotation);

            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        for (&(player, hand), &ptr) in &self.hand_anchors {
            if ptr.is_null() {
                continue;
            }
            let Some(rig) = rigs.get(&player) else {
                continue;
            };
            let grip = rig.hand_grip(hand);
            let target = to_px_transform(grip.position, grip.rotation);

            unsafe { (*ptr).set_kinematic_target(&target) };
        }

        let zero = PxVec3::new(0.0, 0.0, 0.0);
        for state in self.dynamic.values_mut() {
            const KILL_Y: f32 = -15.0;

            let fell_out = unsafe { (*state.ptr).get_global_pose() }.translation().y() < KILL_Y;

            let due_for_timed_respawn = state
                .respawn_interval
                .map(|interval| {
                    state.elapsed += dt;
                    state.elapsed >= interval
                })
                .unwrap_or(false);

            if !fell_out && !due_for_timed_respawn {
                continue;
            }
            state.elapsed = 0.0;
            let spawn = to_px_transform(state.spawn_pos, state.spawn_rot);

            unsafe {
                (*state.ptr).set_global_pose(&spawn, true);
                (*state.ptr).set_linear_velocity(&zero, true);
                (*state.ptr).set_angular_velocity(&zero, true);
            }
        }

        if let Err(e) = self.scene.step(
            dt,
            None::<&mut physx_sys::PxBaseTask>,
            Some(&mut self.scratch),
            true,
        ) {
            log::warn!("rigid_physics: simulation step failed: {e:?}");
            return;
        }

        for (id, state) in &self.dynamic {
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
    fn default() -> Self {
        Self::new()
    }
}
