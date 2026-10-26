use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use physx::prelude::*;

use physx::cooking::{
    create_convex_mesh, create_height_field, create_triangle_mesh, ConvexMeshCookingResult, PxConvexMeshDesc,
    PxCookingParams, PxHeightFieldDesc, PxTriangleMeshDesc, TriangleMeshCookingResult,
};
use physx::height_field::{HeightFieldMaterial, HeightFieldSample};
use physx::traits::Class;
use physx::triangle_mesh::TriangleMesh;

use crate::rigid_physics::{
    calculated_mass, to_px_transform, to_px_vec3, to_raw_transform, DynamicActor, PhysicsWorld,
    PxFoundation, PxRigidDynamic, PxRigidStatic, TerrainCollider, DEFAULT_DENSITY,
};
use crate::scene::{BodyMode, ColliderShape, GameObject, RigidBodyDef, SliderJointDef, TerrainColliderDef};
use crate::brush::{self, BrushDef};

pub(crate) fn brush_collision_triangles(def: &BrushDef) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut points: Vec<[f32; 3]> = Vec::new();
    let mut indices = Vec::new();
    for solid in def.evaluate() {
        for (i, poly) in brush::solid_polygons(&solid).into_iter().enumerate() {
            let Some(poly) = poly else { continue };
            if poly.len() < 3 {
                continue;
            }
            let n = solid.faces[i].plane().n;
            let ordered: Vec<brush::Vec3> = if brush::polygon_winding(&poly, n) < 0.0 {
                poly.into_iter().rev().collect()
            } else {
                poly
            };
            let base = points.len() as u32;
            points.extend(ordered.iter().map(|q| [q[0] as f32, q[1] as f32, q[2] as f32]));
            for k in 1..ordered.len() as u32 - 1 {
                indices.extend([base, base + k, base + k + 1]);
            }
        }
    }
    (points, indices)
}

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

fn cook_triangle_mesh(
    foundation: &mut PxFoundation,
    points: &[PxVec3],
    indices: &[u32],
) -> Option<Owner<TriangleMesh>> {
    let params = PxCookingParams::new(foundation)?;

    let mut desc = PxTriangleMeshDesc::new();
    desc.obj.points.count = points.len() as u32;
    desc.obj.points.stride = std::mem::size_of::<PxVec3>() as u32;
    desc.obj.points.data = points.as_ptr().cast::<std::ffi::c_void>();

    desc.obj.triangles.count = (indices.len() / 3) as u32;
    desc.obj.triangles.stride = (std::mem::size_of::<u32>() * 3) as u32;
    desc.obj.triangles.data = indices.as_ptr().cast::<std::ffi::c_void>();

    match create_triangle_mesh(foundation, &params, &desc) {
        TriangleMeshCookingResult::Success(mesh) => Some(mesh),
        _ => None,
    }
}

/// THE GROUND THE CAVES PUT BACK, AS MESHES TO COOK: the triangles of a
/// [`crate::terrain::GroundPatch`] that `region` takes, in batches of a few
/// thousand, each with the vertices it uses and no others.
///
/// One mesh of all of them would do, and cost what one large cook costs: the
/// working memory of a cook follows the mesh it is given, and a process keeps
/// what its largest moment took (48 caves' patches, 504,048 triangles: 131 MB
/// as one mesh). Cooked a batch at a time the working memory is a batch's,
/// used again for the next.
///
/// A batch is whole CONNECTED PIECES -- triangles that share a vertex stay in
/// one mesh -- so no edge inside a piece of ground becomes the rim of a mesh,
/// which a sliding body would feel as a ridge. A piece larger than a batch is
/// a batch by itself. Pieces are taken in the order the patch stores them,
/// which is a cave at a time, so a batch is also a place.
///
/// COST: one pass over the patch's triangles and vertices; nothing of the
/// level's size.
pub(crate) fn ground_patch_batches(extra: &crate::terrain::GroundPatch, region: crate::physics::Aabb) -> Vec<(Vec<PxVec3>, Vec<u32>)> {
    /// Triangles after which a batch is closed.
    const BATCH: usize = 8192;

    let triangles: Vec<&[u32]> = extra.triangles_in(region).collect();
    let mut piece: Vec<u32> = (0..extra.positions.len() as u32).collect();
    fn root(piece: &mut [u32], mut v: u32) -> u32 {
        while piece[v as usize] != v {
            piece[v as usize] = piece[piece[v as usize] as usize];
            v = piece[v as usize];
        }
        v
    }
    for tri in &triangles {
        for &v in &tri[1..] {
            let (a, b) = (root(&mut piece, tri[0]), root(&mut piece, v));
            piece[a.max(b) as usize] = a.min(b);
        }
    }

    const NONE: u32 = u32::MAX;
    let mut batch_of_piece = vec![NONE; extra.positions.len()];
    let mut piece_size = vec![0u32; extra.positions.len()];
    for tri in &triangles {
        piece_size[root(&mut piece, tri[0]) as usize] += 1;
    }
    let mut batch_triangles: Vec<usize> = Vec::new();
    for tri in &triangles {
        let r = root(&mut piece, tri[0]) as usize;
        if batch_of_piece[r] == NONE {
            if batch_triangles.last().is_none_or(|n| *n >= BATCH) {
                batch_triangles.push(0);
            }
            batch_of_piece[r] = (batch_triangles.len() - 1) as u32;
            *batch_triangles.last_mut().unwrap() += piece_size[r] as usize;
        }
    }

    let mut batches: Vec<(Vec<PxVec3>, Vec<u32>)> =
        batch_triangles.iter().map(|n| (Vec::new(), Vec::with_capacity(n * 3))).collect();
    let mut slot = vec![NONE; extra.positions.len()];
    for tri in &triangles {
        let (points, indices) = &mut batches[batch_of_piece[root(&mut piece, tri[0]) as usize] as usize];
        for &v in *tri {
            if slot[v as usize] == NONE {
                let p = extra.positions[v as usize];
                slot[v as usize] = points.len() as u32;
                points.push(PxVec3::new(p.x, p.y, p.z));
            }
            indices.push(slot[v as usize]);
        }
    }
    batches
}

impl PhysicsWorld {
    pub(crate) fn spawn_actor(&mut self, obj: &GameObject, def: &RigidBodyDef) {

        if !def.enabled {
            return;
        }

        if self.statics.contains_key(&obj.id) || self.dynamic.contains_key(&obj.id) {
            log::debug!("rigid_physics: '{}' already has a collider", obj.id);
            return;
        }
        let transform = to_px_transform(obj.cuboid.position, obj.cuboid.rotation);
        let mass = def
            .mass
            .unwrap_or_else(|| calculated_mass(obj.cuboid.half_size, DEFAULT_DENSITY));
        let collider_half = def
            .collider_half_size
            .map_or(obj.cuboid.half_size, Vec3::from);
        let shape_transform = to_px_transform(Vec3::from(def.collider_offset), Quat::IDENTITY);

        let Some(mut material) =
            self.foundation
                .create_material(def.friction, def.friction, def.restitution, ())
        else {
            log::warn!("rigid_physics: failed to create material for '{}'", obj.id);
            return;
        };

        let hull = match (def.shape, def.mode) {
            (ColliderShape::Hull, BodyMode::Kinematic | BodyMode::Dynamic) => self.hull_for(obj),
            _ => None,
        };
        let reach = match (def.shape, hull) {
            (ColliderShape::Hull, Some((_, centre, half, _))) => {
                crate::water::world_half_extents(half, obj.cuboid.rotation).y - (obj.cuboid.rotation * centre).y
            }
            (ColliderShape::Box | ColliderShape::Hull, _) => crate::water::world_half_extents(collider_half, obj.cuboid.rotation).y,
            (ColliderShape::Sphere { radius }, _) => radius.abs(),
            (ColliderShape::Capsule { radius, half_height }, _) => radius.abs() + half_height.abs(),
        };
        let centre = obj.cuboid.position + obj.cuboid.rotation * Vec3::from(def.collider_offset);
        self.note_floor(centre.y - reach);

        match def.mode {
            BodyMode::Static => {
                let created = match def.shape {
                    ColliderShape::Box | ColliderShape::Hull => {
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
                    Some(mut actor) => {

                        let ptr: *mut PxRigidStatic = &raw mut *actor;
                        self.scene.add_static_actor(actor);
                        self.statics.insert(obj.id.clone(), ptr);
                    }
                    None => log::warn!(
                        "rigid_physics: failed to create static actor for '{}'",
                        obj.id
                    ),
                }
            }
            BodyMode::Kinematic | BodyMode::Dynamic => {
                let (bounds_half, volume) = match (def.shape, hull) {
                    (ColliderShape::Hull, Some((_, _, half, volume))) => (half, volume),
                    (ColliderShape::Box | ColliderShape::Hull, _) => (collider_half.abs(), 8.0 * collider_half.abs().element_product()),
                    (ColliderShape::Sphere { radius }, _) => (Vec3::splat(radius.abs()), 4.0 / 3.0 * std::f32::consts::PI * radius.abs().powi(3)),
                    (ColliderShape::Capsule { radius, half_height }, _) => {
                        let r = radius.abs();
                        (
                            Vec3::new(r + half_height.abs(), r, r),
                            std::f32::consts::PI * r * r * (2.0 * half_height.abs() + 4.0 / 3.0 * r),
                        )
                    }
                };
                let created = match (def.shape, hull) {
                    (ColliderShape::Hull, Some((index, ..))) => {
                        let scale = unsafe { physx_sys::PxMeshScale_new() };
                        let geo = PxConvexMeshGeometry::new(
                            self.hull_meshes[index].as_mut(),
                            &scale,
                            ConvexMeshGeometryFlags::TightBounds,
                        );
                        self.foundation.create_rigid_dynamic(
                            transform,
                            &geo,
                            material.as_mut(),
                            1.0,
                            PxTransform::default(),
                            (),
                        )
                    }
                    (ColliderShape::Box | ColliderShape::Hull, _) => {
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
                    (ColliderShape::Sphere { radius }, _) => {
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
                    (ColliderShape::Capsule { radius, half_height }, _) => {
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
                        unsafe {
                            physx_sys::PxRigidBodyExt_setMassAndUpdateInertia_1(
                                &raw mut *actor as *mut physx_sys::PxRigidBody,
                                mass,
                                std::ptr::null(),
                                false,
                            );
                        }
                        if def.mode == BodyMode::Kinematic {
                            actor.set_rigid_body_flag(RigidBodyFlag::Kinematic, true);
                            let ptr: *mut PxRigidDynamic = &raw mut *actor;
                            self.scene.add_dynamic_actor(actor);
                            self.kinematic.insert(obj.id.clone(), ptr);
                        } else {
                            actor.set_rigid_body_flag(RigidBodyFlag::EnableCcd, true);

                            actor.set_solver_iteration_counts(8, 2);
                            unsafe {
                                physx_sys::PxRigidBody_setMaxDepenetrationVelocity_mut(
                                    &raw mut *actor as *mut physx_sys::PxRigidBody,
                                    2.0,
                                );
                            }
                            let vel = to_px_vec3(def.linear_velocity);
                            actor.set_linear_velocity(&vel, true);
                            let ptr: *mut PxRigidDynamic = &raw mut *actor;
                            self.scene.add_dynamic_actor(actor);
                            let mut body = DynamicActor::new(ptr, obj.cuboid.position, obj.cuboid.rotation, bounds_half, volume);
                            body.respawn_interval = def.respawn_interval;
                            body.respawn_sunk = def.respawn_sunk;
                            self.dynamic.insert(obj.id.clone(), body);
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

    /// THE HULL OF AN OBJECT'S MODEL, cooked once for each model file and
    /// scale however many objects use it: its index in `hull_meshes`, the
    /// centre and half extents of the box round it in the object's frame, and
    /// its volume. The model's points are taken as the renderer draws them
    /// (scaled, then turned by the mesh's own offset) and `PhysX` keeps at most
    /// 32 of the hull's corners. `None`, with a warning, where there is no
    /// model or it will not cook.
    fn hull_for(&mut self, obj: &GameObject) -> Option<(usize, Vec3, Vec3, f32)> {
        let mesh = obj.mesh.as_ref()?;
        let key = format!("{}|{:?}|{:?}", mesh.path, mesh.scale.to_array(), mesh.rotation_offset.to_array());
        if let Some(found) = self.hulls.get(&key) {
            return Some(*found);
        }
        let path = self.game_dir.join(&mesh.path);
        let read = gltf::Gltf::open(&path).ok().and_then(|g| {
            let buffers = gltf::import_buffers(&g.document, path.parent(), g.blob.clone()).ok()?;
            Some((g.document, buffers))
        });
        let Some((doc, buffers)) = read else {
            log::warn!("rigid_physics: hull of '{}': '{}' could not be read; it is its box", obj.id, path.display());
            return None;
        };
        let mut points: Vec<PxVec3> = Vec::new();
        for (mesh_index, node) in collect_terrain_instances(&doc, None) {
            let Some(m) = doc.meshes().nth(mesh_index) else { continue };
            let (local, _) = read_mesh_geometry(&m, &buffers);
            points.extend(local.iter().map(|p| {
                let p = mesh.rotation_offset * (mesh.scale * node.transform_point3(Vec3::new(p.x(), p.y(), p.z())));
                PxVec3::new(p.x, p.y, p.z)
            }));
        }
        if points.len() < 4 {
            log::warn!("rigid_physics: hull of '{}': '{}' has no geometry; it is its box", obj.id, path.display());
            return None;
        }
        let params = PxCookingParams::new(&self.foundation)?;
        let mut desc = PxConvexMeshDesc::new();
        desc.obj.points.count = points.len() as u32;
        desc.obj.points.stride = std::mem::size_of::<PxVec3>() as u32;
        desc.obj.points.data = points.as_ptr().cast::<std::ffi::c_void>();
        desc.obj.flags = physx_sys::PxConvexFlags::ComputeConvex;
        desc.obj.vertexLimit = 32;
        let ConvexMeshCookingResult::Success(hull) = create_convex_mesh(&mut self.foundation, &params, &desc) else {
            log::warn!("rigid_physics: hull of '{}' did not cook; it is its box", obj.id);
            return None;
        };
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in hull.get_vertices() {
            let p = Vec3::new(p.x(), p.y(), p.z());
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let mut volume = 0.0f32;
        unsafe {
            let mut inertia: physx_sys::PxMat33 = std::mem::zeroed();
            let mut centre: physx_sys::PxVec3 = std::mem::zeroed();
            physx_sys::PxConvexMesh_getMassInformation(hull.as_ptr(), &raw mut volume, &raw mut inertia, &raw mut centre);
        }
        self.hull_meshes.push(hull);
        let found = (self.hull_meshes.len() - 1, (lo + hi) * 0.5, (hi - lo) * 0.5, volume.max(1e-9));
        self.hulls.insert(key, found);
        Some(found)
    }

    pub(crate) fn spawn_slider_joint(&mut self, obj: &GameObject, def: &SliderJointDef) {
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
                parent.ptr.cast::<physx_sys::PxRigidActor>(),
                &raw const frame,
                child.ptr.cast::<physx_sys::PxRigidActor>(),
                &raw const frame,
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
                physx_sys::PxJointLinearLimitPair_new_1(0.0, def.travel.max(0.001), &raw const hard_spring);
            physx_sys::PxD6Joint_setLinearLimit_mut(joint, physx_sys::PxD6Axis::X, &raw const limit);

            let side_spring = physx_sys::PxSpring_new(4000.0, 120.0);
            let side_limit = physx_sys::PxJointLinearLimitPair_new_1(-0.0005, 0.0005, &raw const side_spring);
            physx_sys::PxD6Joint_setLinearLimit_mut(joint, physx_sys::PxD6Axis::Y, &raw const side_limit);
            physx_sys::PxD6Joint_setLinearLimit_mut(joint, physx_sys::PxD6Axis::Z, &raw const side_limit);

            let drive = physx_sys::PxD6JointDrive_new_1(
                def.spring_stiffness,
                def.spring_damping,
                1.0e6,
                false,
            );
            physx_sys::PxD6Joint_setDrive_mut(joint, physx_sys::PxD6Drive::X, &raw const drive);
            let rest_pose = to_raw_transform(PxTransform::default());
            physx_sys::PxD6Joint_setDrivePosition_mut(joint, &raw const rest_pose, true);
        }
    }

    pub(crate) fn spawn_scatter_trunks(&mut self, scene: &crate::scene::Scene, ground: &dyn crate::scatter::Ground) {
        if scene.scatter.is_empty() {
            return;
        }
        let Some(mut material) = self.foundation.create_material(0.8, 0.8, 0.0, ()) else {
            log::warn!("rigid_physics: failed to create scatter trunk material");
            return;
        };

        let upright = to_px_transform(Vec3::ZERO, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let mut trunks = 0usize;
        for layer in &scene.scatter {
            let solid: Vec<bool> = layer.prototypes.iter().map(|p| scatter_is_solid(&p.mesh)).collect();
            if !solid.iter().any(|s| *s) {
                continue;
            }
            for inst in crate::scatter::resolve(layer, ground) {
                if !solid[inst.prototype] {
                    continue;
                }
                let radius = (TRUNK_RADIUS * inst.scale).clamp(TRUNK_RADIUS_MIN, TRUNK_RADIUS_MAX);
                let geo = PxCapsuleGeometry::new(radius, TRUNK_HALF_HEIGHT);

                let centre = inst.position + Vec3::Y * (TRUNK_HALF_HEIGHT - TRUNK_SINK);
                let transform = to_px_transform(centre, Quat::IDENTITY);
                match self.foundation.create_rigid_static(transform, &geo, material.as_mut(), upright, ()) {
                    Some(actor) => {
                        self.scene.add_static_actor(actor);
                        trunks += 1;
                    }
                    None => log::warn!("rigid_physics: failed to create a scatter trunk actor"),
                }
            }
        }
        self.materials.push(material);
        if trunks > 0 {
            log::info!("rigid_physics: {trunks} scatter trunk collider(s)");
        }
    }

    /// Static collider for the scene's sculpted ground.
    ///
    /// A `PhysX` HEIGHT FIELD where the ground is a grid of heights, which every
    /// scene's is today (`spawn_scene_height_field`): its memory and its build
    /// time follow the sample count, at four bytes and some tens of
    /// nanoseconds each. The triangle mesh it replaces
    /// (`spawn_scene_terrain_mesh`) took about 600 bytes of process memory a
    /// sample and a BVH to cook -- 2.6 GB and over 3 s at 2049 samples a side,
    /// on the server and again on the headset (docs/scale-audit-2026-10-13.md,
    /// L3; the numbers at each size are on [`TerrainCollider::TriangleMesh`]).
    /// The mesh stays for a source that is not a grid, for a grid `PhysX` will
    /// not take, and by choice ([`TerrainCollider`]).
    ///
    /// A failure warns and leaves the scene without ground rather than aborting
    /// the load: a level that opens with nothing to stand on is diagnosable, and
    /// a runtime that refuses to start is not.
    pub(crate) fn spawn_scene_terrain(&mut self, source: &dyn crate::terrain::TerrainSource) {
        if self.terrain_collider == TerrainCollider::HeightField {
            if let Some(field) = source.as_heightfield() {
                if self.spawn_scene_height_field(field) {
                    return;
                }
                log::warn!("rigid_physics: scene terrain not taken as a height field; cooking it as a triangle mesh");
            }
        }
        self.spawn_scene_terrain_mesh(source);
    }

    /// The ground as a `PhysX` height field. `false`, with nothing added to the
    /// scene, if `PhysX` will not take it.
    ///
    /// THE SAME SURFACE AS `patch(bounds, 1)`, which is what is drawn and what
    /// the triangle mesh was cooked from, by the mesher's own answers rather
    /// than a second copy of its rules:
    ///
    /// - HEIGHTS: the sixteen-bit samples themselves, moved to `PhysX`'s signed
    ///   range, with the height of a step as the scale -- no rounding, so a
    ///   vertex is where the mesher puts it to within float arithmetic
    ///   (hundredths of a millimetre on a level some hundreds of metres tall).
    /// - HOLES: a cell the mesher leaves out (`cell_is_open`, and the cells
    ///   past `meshed_cells`) has `PhysX`'s hole material on both its triangles.
    /// - DIAGONALS: each cell's tessellation flag is the mesher's choice for
    ///   it (`cell_splits_through_first_sample`), so a ray lands on the
    ///   triangle that is drawn, not on the other half of the quad.
    /// - THE CAVES' GROUND: the triangles a cave bake put back in its cells
    ///   are not on the grid, so they are cooked as they are, as triangle
    ///   meshes of their own beside the field, a few thousand triangles each
    ///   (`ground_patch_batches`). Their outline vertices are the grid's own
    ///   to the bit, so the two meet where the one mesh's triangles did.
    ///
    /// `PhysX` lays a height field's rows along x and its columns along z, and
    /// stores it a row at a time: the transpose of the terrain's own order.
    ///
    /// COST: one pass over the samples and four bytes each, twice while it is
    /// built (this array, then `PhysX`'s copy) and once after. The patch meshes
    /// follow the caves' triangles, not the level's size. A ray straight down
    /// reads one cell. A ray along the ground walks the cells under it, where
    /// a mesh's tree stepped over them: a ray the whole length of a level
    /// 2049 samples a side, inside the ground's height range and meeting
    /// nothing, took 6.5 microseconds here against 0.2 -- and one above the
    /// highest ground, or stopped by the first hill, is as cheap as it was.
    fn spawn_scene_height_field(&mut self, field: &crate::terrain::Heightfield) -> bool {
        const MIN_HEIGHT_SCALE: f32 = 0.0001 / 65535.0;
        const MIN_CELL: f32 = 1e-8;

        let [nx, nz] = field.resolution();
        let [dx, dz] = field.cell_size();
        let [lo, hi] = field.height_range();
        let origin = field.origin();
        let step = (hi - lo) / f32::from(u16::MAX);
        if !(step >= 0.0 && step.is_finite() && dx >= MIN_CELL && dz >= MIN_CELL && dx.is_finite() && dz.is_finite() && origin.is_finite()) {
            return false;
        }
        let level = step < MIN_HEIGHT_SCALE;
        let height_scale = step.max(MIN_HEIGHT_SCALE);
        let base = if level { origin.y + lo } else { origin.y + lo + 32768.0 * height_scale };

        let raw = field.raw_samples();
        let holed = field.holes().is_some();
        let [meshed_x, meshed_z] = field.meshed_cells();
        let solid = HeightFieldMaterial::new(0);
        let mut samples: Vec<HeightFieldSample> = Vec::with_capacity(nx as usize * nz as usize);
        let (mut open_cells, mut cells) = (0usize, 0usize);
        for ix in 0..nx {
            for iz in 0..nz {
                let height = if level { 0 } else { (i32::from(raw[(iz * nx + ix) as usize]) - 32768) as i16 };
                let is_cell = ix + 1 < nx && iz + 1 < nz;
                let open = is_cell && (ix >= meshed_x || iz >= meshed_z || (holed && field.cell_is_open(ix, iz)));
                let material = if open { HeightFieldMaterial::HOLE } else { solid };
                let through_first = is_cell && !open && field.cell_splits_through_first_sample(ix, iz);
                samples.push(HeightFieldSample::new(height, material, material, through_first));
                cells += usize::from(is_cell);
                open_cells += usize::from(open);
            }
        }

        let mut desc = PxHeightFieldDesc::new();
        desc.obj.nbRows = nx;
        desc.obj.nbColumns = nz;
        desc.obj.format = physx_sys::PxHeightFieldFormat::S16Tm;
        desc.obj.samples.stride = std::mem::size_of::<HeightFieldSample>() as u32;
        desc.obj.samples.data = samples.as_ptr().cast::<std::ffi::c_void>();
        if !desc.is_valid() {
            return false;
        }
        let Some(mut height_field) = create_height_field(&mut self.foundation, &desc) else {
            return false;
        };
        drop(samples);

        let Some(mut material) = self.foundation.create_material(0.8, 0.8, 0.0, ()) else {
            return false;
        };

        let geo = PxHeightFieldGeometry::new(height_field.as_mut(), MeshGeometryFlags::empty(), height_scale, dx, dz);
        if !geo.is_valid() {
            return false;
        }
        let pose = to_px_transform(Vec3::new(origin.x, base, origin.z), Quat::IDENTITY);
        let Some(ground) = self.foundation.create_rigid_static(pose, &geo, material.as_mut(), PxTransform::default(), ()) else {
            return false;
        };

        let mut patch_meshes = Vec::new();
        let mut patch_actors = Vec::new();
        let mut patch_triangles = 0usize;
        if let Some(extra) = field.ground_patch() {
            for (points, indices) in ground_patch_batches(extra, crate::terrain::TerrainSource::bounds(field)) {
                let Some(mut mesh) = cook_triangle_mesh(&mut self.foundation, &points, &indices) else {
                    return false;
                };
                let scale_px = PxVec3::new(1.0, 1.0, 1.0);
                let rot_px = PxQuat::new(0.0, 0.0, 0.0, 1.0);
                let mesh_scale = unsafe { physx_sys::PxMeshScale_new_3(scale_px.as_ptr(), rot_px.as_ptr()) };
                let geo = PxTriangleMeshGeometry::new(mesh.as_mut(), &mesh_scale, MeshGeometryFlags::empty());
                let Some(actor) = self.foundation.create_rigid_static(
                    to_px_transform(Vec3::ZERO, Quat::IDENTITY),
                    &geo,
                    material.as_mut(),
                    PxTransform::default(),
                    (),
                ) else {
                    return false;
                };
                patch_triangles += indices.len() / 3;
                patch_meshes.push(mesh);
                patch_actors.push(actor);
            }
        }

        self.scene.add_static_actor(ground);
        let patch_count = patch_meshes.len();
        for actor in patch_actors {
            self.scene.add_static_actor(actor);
        }
        self.terrain_height_fields.push(height_field);
        self.terrain_meshes.extend(patch_meshes);
        self.materials.push(material);
        log::info!(
            "rigid_physics: scene terrain is a height field -- {nx} x {nz} samples, {open_cells} of {cells} cells open, {patch_triangles} patch triangles beside it in {patch_count} meshes"
        );
        true
    }

    /// The ground as one triangle mesh of `patch(bounds, 1)`: every sample's
    /// vertex and two triangles a cell, cooked into a BVH. Its memory and its
    /// cooking time follow the sample count at hundreds of bytes and most of a
    /// microsecond each -- see [`TerrainCollider::TriangleMesh`].
    /// It shares its cooking code with the glTF terrain colliders.
    pub(crate) fn spawn_scene_terrain_mesh(&mut self, source: &dyn crate::terrain::TerrainSource) {
        let patch = source.patch(source.bounds(), 1);
        if patch.positions.is_empty() || patch.indices.is_empty() {
            log::warn!("rigid_physics: scene terrain produced no geometry");
            return;
        }

        let Some(mut material) = self.foundation.create_material(0.8, 0.8, 0.0, ()) else {
            log::warn!("rigid_physics: failed to create scene terrain material");
            return;
        };

        let points: Vec<PxVec3> = patch
            .positions
            .iter()
            .map(|p| PxVec3::new(p.x, p.y, p.z))
            .collect();

        let Some(owned) = cook_triangle_mesh(&mut self.foundation, &points, &patch.indices) else {
            log::warn!("rigid_physics: failed to cook scene terrain mesh");
            return;
        };
        self.terrain_meshes.push(owned);
        let mesh_idx = self.terrain_meshes.len() - 1;

        let scale_px = PxVec3::new(1.0, 1.0, 1.0);
        let rot_px = PxQuat::new(0.0, 0.0, 0.0, 1.0);
        let mesh_scale = unsafe { physx_sys::PxMeshScale_new_3(scale_px.as_ptr(), rot_px.as_ptr()) };

        let geo = PxTriangleMeshGeometry::new(
            self.terrain_meshes[mesh_idx].as_mut(),
            &mesh_scale,
            MeshGeometryFlags::empty(),
        );

        let transform = to_px_transform(Vec3::ZERO, Quat::IDENTITY);
        if let Some(actor) = self.foundation.create_rigid_static(
            transform,
            &geo,
            material.as_mut(),
            PxTransform::default(),
            (),
        ) { self.scene.add_static_actor(actor) } else {
            log::warn!("rigid_physics: failed to create scene terrain actor");
            return;
        }
        self.materials.push(material);
        log::info!(
            "rigid_physics: scene terrain cooked -- {} vertices, {} triangles",
            patch.positions.len(),
            patch.indices.len() / 3
        );
    }

    pub(crate) fn spawn_brush_collider(&mut self, obj: &GameObject, def: &BrushDef) {
        if self.statics.contains_key(&obj.id)
            || self.dynamic.contains_key(&obj.id)
            || self.kinematic.contains_key(&obj.id)
        {
            log::debug!("rigid_physics: brush '{}' already has an authored collider", obj.id);
            return;
        }
        let (positions, indices) = brush_collision_triangles(def);
        if indices.is_empty() {
            log::warn!("rigid_physics: brush '{}' has no surface to collide with", obj.id);
            return;
        }
        let Some(mut material) = self.foundation.create_material(0.8, 0.8, 0.0, ()) else {
            log::warn!("rigid_physics: failed to create material for brush '{}'", obj.id);
            return;
        };
        let points: Vec<PxVec3> = positions.iter().map(|p| PxVec3::new(p[0], p[1], p[2])).collect();
        let Some(owned) = cook_triangle_mesh(&mut self.foundation, &points, &indices) else {
            log::warn!("rigid_physics: failed to cook brush '{}'", obj.id);
            return;
        };
        self.note_floor(positions.iter().fold(f32::INFINITY, |low, p| low.min(p[1])));
        self.terrain_meshes.push(owned);
        let mesh_idx = self.terrain_meshes.len() - 1;

        let scale_px = PxVec3::new(1.0, 1.0, 1.0);
        let rot_px = PxQuat::new(0.0, 0.0, 0.0, 1.0);
        let mesh_scale = unsafe { physx_sys::PxMeshScale_new_3(scale_px.as_ptr(), rot_px.as_ptr()) };
        let geo = PxTriangleMeshGeometry::new(
            self.terrain_meshes[mesh_idx].as_mut(),
            &mesh_scale,
            MeshGeometryFlags::empty(),
        );
        let transform = to_px_transform(Vec3::ZERO, Quat::IDENTITY);
        match self.foundation.create_rigid_static(
            transform,
            &geo,
            material.as_mut(),
            PxTransform::default(),
            (),
        ) {
            Some(mut actor) => {
                let ptr: *mut PxRigidStatic = &raw mut *actor;
                self.scene.add_static_actor(actor);
                self.statics.insert(obj.id.clone(), ptr);
            }
            None => {
                log::warn!("rigid_physics: failed to create static actor for brush '{}'", obj.id);
                return;
            }
        }
        self.materials.push(material);
        log::info!(
            "rigid_physics: brush '{}' is solid -- {} triangles",
            obj.id,
            indices.len() / 3
        );
    }

    pub(crate) fn spawn_walk_area(
        &mut self,
        obj: &GameObject,
        def: &crate::walk_area::WalkAreaDef,
        terrain: Option<&dyn crate::terrain::TerrainSource>,
    ) {

        const MAX_CELLS: usize = 128 * 128;
        let c = &obj.cuboid;
        let mesh = crate::walk_area::build(c.position, c.half_size.x, c.half_size.z, def, MAX_CELLS, &|x, z| {
            terrain.and_then(|t| t.height_at(x, z))
        });
        let (points, indices) = crate::walk_area::physics_triangles(&mesh);
        if points.is_empty() || indices.is_empty() {
            log::warn!("rigid_physics: walk_area on '{}' produced no geometry", obj.id);
            return;
        }

        let Some(mut material) = self.foundation.create_material(0.8, 0.8, 0.0, ()) else {
            log::warn!("rigid_physics: failed to create walk_area material for '{}'", obj.id);
            return;
        };

        let px_points: Vec<PxVec3> = points.iter().map(|p| PxVec3::new(p.x, p.y, p.z)).collect();
        let Some(owned) = cook_triangle_mesh(&mut self.foundation, &px_points, &indices) else {
            log::warn!("rigid_physics: walk_area on '{}' failed to cook", obj.id);
            return;
        };
        self.terrain_meshes.push(owned);
        let mesh_idx = self.terrain_meshes.len() - 1;

        let scale_px = PxVec3::new(1.0, 1.0, 1.0);
        let rot_px = PxQuat::new(0.0, 0.0, 0.0, 1.0);
        let mesh_scale = unsafe { physx_sys::PxMeshScale_new_3(scale_px.as_ptr(), rot_px.as_ptr()) };
        let geo = PxTriangleMeshGeometry::new(
            self.terrain_meshes[mesh_idx].as_mut(),
            &mesh_scale,
            MeshGeometryFlags::empty(),
        );
        let transform = to_px_transform(Vec3::ZERO, Quat::IDENTITY);
        if let Some(actor) = self.foundation.create_rigid_static(transform, &geo, material.as_mut(), PxTransform::default(), ()) { self.scene.add_static_actor(actor) } else {
            log::warn!("rigid_physics: walk_area on '{}' failed to create static actor", obj.id);
            return;
        }
        self.materials.push(material);
        log::info!(
            "rigid_physics: walk_area '{}' cooked -- {} vertices, {} triangles ({} fence quads)",
            obj.id,
            points.len(),
            indices.len() / 3,
            mesh.fence.len()
        );
    }

    pub(crate) fn spawn_terrain_colliders(&mut self, obj: &GameObject, def: &TerrainColliderDef, game_dir: &Path) {
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

        let mut cooked: HashMap<usize, (usize, Vec3, Vec3)> = HashMap::new();
        let mut spawned = 0u32;

        for (mesh_index, node_mat) in instances {
            let (mesh_idx_in_pool, local_min, local_max) = if let Some(&found) = cooked.get(&mesh_index) {
                found
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
                if let Some(owned) = cook_triangle_mesh(&mut self.foundation, &points, &tri_indices) {
                    self.terrain_meshes.push(owned);
                    let i = self.terrain_meshes.len() - 1;
                    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                    for p in &points {
                        let p = Vec3::new(p.x(), p.y(), p.z());
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                    cooked.insert(mesh_index, (i, lo, hi));
                    (i, lo, hi)
                } else {
                    log::warn!(
                        "rigid_physics: terrain_collider on '{}' failed to cook mesh (index {mesh_index})",
                        obj.id
                    );
                    continue;
                }
            };

            let world = object_mat * node_mat;
            let (scale, rotation, translation) = world.to_scale_rotation_translation();

            let lowest = (0..8)
                .map(|k| {
                    let pick = |bit: u32, lo: f32, hi: f32| if k & bit == 0 { lo } else { hi };
                    let corner = Vec3::new(pick(1, local_min.x, local_max.x), pick(2, local_min.y, local_max.y), pick(4, local_min.z, local_max.z));
                    world.transform_point3(corner).y
                })
                .fold(f32::INFINITY, f32::min);
            self.note_floor(lowest);

            let scale_px = PxVec3::new(scale.x, scale.y, scale.z);
            let rot_px = PxQuat::new(0.0, 0.0, 0.0, 1.0);
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
}

const TRUNK_RADIUS: f32 = 0.7;
const TRUNK_RADIUS_MIN: f32 = 0.12;
const TRUNK_RADIUS_MAX: f32 = 1.2;

const TRUNK_HALF_HEIGHT: f32 = 2.0;

const TRUNK_SINK: f32 = 0.5;

pub(crate) fn scatter_is_solid(mesh: &str) -> bool {
    let m = mesh.replace('\\', "/");
    m.contains("/trees/") || m.contains("/rocks/") || m == "builtin:tree" || m == "builtin:rock"
}
