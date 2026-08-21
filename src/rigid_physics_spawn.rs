use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use physx::prelude::*;

use physx::cooking::{
    create_triangle_mesh, PxCookingParams, PxTriangleMeshDesc, TriangleMeshCookingResult,
};
use physx::traits::Class;
use physx::triangle_mesh::TriangleMesh;

use crate::rigid_physics::{
    calculated_mass, to_px_transform, to_px_vec3, to_raw_transform, DynamicActor, PhysicsWorld,
    PxFoundation, PxRigidDynamic, PxRigidStatic, DEFAULT_DENSITY,
};
use crate::scene::{BodyMode, ColliderShape, GameObject, RigidBodyDef, SliderJointDef, TerrainColliderDef};

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
    pub(crate) fn spawn_actor(&mut self, obj: &GameObject, def: &RigidBodyDef) {
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
                    Some(mut actor) => {
                        // Take the pointer BEFORE handing ownership to the
                        // scene, exactly as the dynamic path does. The scene
                        // owns the actor from here; this map only borrows, so
                        // it can be dropped without releasing anything.
                        let ptr: *mut PxRigidStatic = &mut *actor as *mut PxRigidStatic;
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

    /// Static collider for the scene's sculpted ground.
    ///
    /// Cooked as a triangle mesh rather than a PhysX heightfield. PhysX 5.1 has
    /// PxHeightField and it would be considerably more compact, but the safe
    /// `physx` wrapper does not expose it -- reaching it means unsafe FFI
    /// against physx-sys, which is not worth doing before anything has measured
    /// terrain collision as a cost. The triangle path is already proven here by
    /// the glTF terrain colliders and shares its cooking code.
    ///
    /// A failure warns and leaves the scene without ground rather than aborting
    /// the load: a level that opens with nothing to stand on is diagnosable, and
    /// a runtime that refuses to start is not.
    pub(crate) fn spawn_scene_terrain(&mut self, def: &crate::terrain::TerrainDef, game_dir: &Path) {
        let source = match crate::terrain::load(def, game_dir) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("rigid_physics: scene terrain not loaded: {e}");
                return;
            }
        };

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

        // Already in world space -- patch() bakes the terrain origin into every
        // vertex -- so the actor sits at identity rather than carrying a second
        // transform that could disagree with the one the editor previewed.
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
            Some(actor) => self.scene.add_static_actor(actor),
            None => {
                log::warn!("rigid_physics: failed to create scene terrain actor");
                return;
            }
        }
        self.materials.push(material);
        log::info!(
            "rigid_physics: scene terrain cooked -- {} vertices, {} triangles",
            patch.positions.len(),
            patch.indices.len() / 3
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
}
