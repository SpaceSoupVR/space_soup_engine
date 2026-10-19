//! WHAT A NAVMESH IS BAKED FROM: every static collider in the scene, read back from PhysX as
//! world-space triangles (`navigation`). What a player collides with is what a creature walks
//! on, whatever made it -- a cuboid, a brush, a model's mesh collider, the terrain.
//!
//! Only STATIC actors, and only their simulation shapes: a door swings (its doorway must stay
//! walkable), a prop gets thrown, and a trigger volume stops nothing. Triangles keep the winding
//! the baker reads a floor by ((b - a) x (c - a) points out of the surface); a mesh mirrored by a
//! negative scale has its winding flipped back. A sphere or capsule is taken as its box (an
//! obstacle either way), and an infinite plane is left out with a warning.

use glam::{Mat3, Quat, Vec3};
use physx::traits::Class;

use crate::rigid_physics::PhysicsWorld;

/// World-space triangles: what [`PhysicsWorld::static_collision_triangles`] returns.
#[derive(Debug, Clone, Default)]
pub struct CollisionTriangles {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// Shapes left out (an infinite plane, a kind with no triangles), by kind.
    pub skipped: Vec<String>,
    /// How many shapes of each kind went in ("Box", "Trianglemesh", "Heightfield", ...).
    pub shapes: std::collections::BTreeMap<String, usize>,
}

impl CollisionTriangles {
    fn push(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let k = self.vertices.len() as u32;
        self.vertices.extend([a.to_array(), b.to_array(), c.to_array()]);
        self.triangles.push([k, k + 1, k + 2]);
    }

    /// A triangle turned to face away from `inside` (a closed shape's middle).
    fn push_out(&mut self, a: Vec3, b: Vec3, c: Vec3, inside: Vec3) {
        if (b - a).cross(c - a).dot(a - inside) < 0.0 {
            self.push(a, c, b);
        } else {
            self.push(a, b, c);
        }
    }

    /// A box's twelve triangles, facing out: `to_world` places its corners.
    fn push_box(&mut self, half: Vec3, to_world: impl Fn(Vec3) -> Vec3) {
        let c = |x: f32, y: f32, z: f32| to_world(Vec3::new(x * half.x, y * half.y, z * half.z));
        let middle = to_world(Vec3::ZERO);
        // Each face as two triangles of its four corners.
        for axis in 0..3 {
            for side in [-1.0f32, 1.0] {
                let corner = |u: f32, v: f32| match axis {
                    0 => c(side, u, v),
                    1 => c(u, side, v),
                    _ => c(u, v, side),
                };
                let (p, q, r, s) = (corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0));
                self.push_out(p, q, r, middle);
                self.push_out(p, r, s, middle);
            }
        }
    }
}

fn pose(t: &physx_sys::PxTransform) -> (Vec3, Quat) {
    (Vec3::new(t.p.x, t.p.y, t.p.z), Quat::from_xyzw(t.q.x, t.q.y, t.q.z, t.q.w).normalize())
}

fn vec(v: &physx_sys::PxVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// A mesh's scale (possibly about a rotated frame) as one matrix: PhysX's own `toMat33`.
fn mesh_scale(s: &physx_sys::PxMeshScale) -> Mat3 {
    let m = unsafe { physx_sys::PxMeshScale_toMat33(s) };
    Mat3::from_cols(vec(&m.column0), vec(&m.column1), vec(&m.column2))
}

/// PhysX's hole material on a height field triangle (`PxHeightFieldMaterial::eHOLE`).
const HOLE: u8 = 127;

impl PhysicsWorld {
    /// Every static collider's triangles, world space: what a navmesh is baked from.
    pub fn static_collision_triangles(&self) -> CollisionTriangles {
        let mut out = CollisionTriangles::default();
        let scene = self.scene.as_ptr();
        let kinds = physx_sys::PxActorTypeFlags::RigidStatic;
        let n = unsafe { physx_sys::PxScene_getNbActors(scene, kinds) };
        let mut actors: Vec<*mut physx_sys::PxActor> = vec![std::ptr::null_mut(); n as usize];
        let got = unsafe { physx_sys::PxScene_getActors(scene, kinds, actors.as_mut_ptr(), n, 0) };
        actors.truncate(got as usize);
        for actor in actors {
            let actor = actor as *const physx_sys::PxRigidActor;
            let (ap, ar) = pose(&unsafe { physx_sys::PxRigidActor_getGlobalPose(actor) });
            let count = unsafe { physx_sys::PxRigidActor_getNbShapes(actor) };
            let mut shapes: Vec<*mut physx_sys::PxShape> = vec![std::ptr::null_mut(); count as usize];
            let got = unsafe { physx_sys::PxRigidActor_getShapes(actor, shapes.as_mut_ptr(), count, 0) };
            shapes.truncate(got as usize);
            for shape in shapes {
                let flags = unsafe { physx_sys::PxShape_getFlags(shape) };
                if !flags.contains(physx_sys::PxShapeFlags::SimulationShape) {
                    continue;
                }
                let (lp, lr) = pose(&unsafe { physx_sys::PxShape_getLocalPose(shape) });
                let (p, r) = (ap + ar * lp, ar * lr);
                let world = |v: Vec3| p + r * v;
                let geometry = unsafe { physx_sys::PxShape_getGeometry(shape) };
                unsafe { self.add_shape(&mut out, geometry, world) };
            }
        }
        out
    }

    /// One shape's triangles, placed by `world`.
    unsafe fn add_shape(&self, out: &mut CollisionTriangles, geometry: *const physx_sys::PxGeometry, world: impl Fn(Vec3) -> Vec3) {
        use physx_sys::PxGeometryType as G;
        let kind = physx_sys::PxGeometry_getType(geometry);
        if !matches!(kind, G::Plane | G::Particlesystem | G::Tetrahedronmesh | G::Hairsystem | G::Custom) {
            *out.shapes.entry(format!("{kind:?}")).or_default() += 1;
        }
        match kind {
            G::Box => {
                let g = &*(geometry as *const physx_sys::PxBoxGeometry);
                out.push_box(vec(&g.halfExtents), &world);
            }
            // Obstacles, never floors: their box is enough. A capsule lies along its +X.
            G::Sphere => {
                let g = &*(geometry as *const physx_sys::PxSphereGeometry);
                out.push_box(Vec3::splat(g.radius), &world);
            }
            G::Capsule => {
                let g = &*(geometry as *const physx_sys::PxCapsuleGeometry);
                out.push_box(Vec3::new(g.halfHeight + g.radius, g.radius, g.radius), &world);
            }
            G::Trianglemesh => {
                let g = &*(geometry as *const physx_sys::PxTriangleMeshGeometry);
                let mesh = g.triangleMesh as *const physx_sys::PxTriangleMesh;
                let scale = mesh_scale(&g.scale);
                // A mirroring scale turns every triangle inside out: turned back.
                let mirrored = scale.determinant() < 0.0;
                let nv = physx_sys::PxTriangleMesh_getNbVertices(mesh) as usize;
                let verts = std::slice::from_raw_parts(physx_sys::PxTriangleMesh_getVertices(mesh), nv);
                let base = out.vertices.len() as u32;
                out.vertices.extend(verts.iter().map(|v| world(scale * vec(v)).to_array()));
                let nt = physx_sys::PxTriangleMesh_getNbTriangles(mesh) as usize;
                let small = physx_sys::PxTriangleMesh_getTriangleMeshFlags(mesh).contains(physx_sys::PxTriangleMeshFlags::E16BitIndices);
                let raw = physx_sys::PxTriangleMesh_getTriangles(mesh);
                for t in 0..nt {
                    let [a, b, c] = if small {
                        let i = std::slice::from_raw_parts(raw as *const u16, nt * 3);
                        [i[t * 3] as u32, i[t * 3 + 1] as u32, i[t * 3 + 2] as u32]
                    } else {
                        let i = std::slice::from_raw_parts(raw as *const u32, nt * 3);
                        [i[t * 3], i[t * 3 + 1], i[t * 3 + 2]]
                    };
                    out.triangles.push(if mirrored { [base + a, base + c, base + b] } else { [base + a, base + b, base + c] });
                }
            }
            G::Convexmesh => {
                let g = &*(geometry as *const physx_sys::PxConvexMeshGeometry);
                let mesh = g.convexMesh as *const physx_sys::PxConvexMesh;
                let scale = mesh_scale(&g.scale);
                let nv = physx_sys::PxConvexMesh_getNbVertices(mesh) as usize;
                let verts: Vec<Vec3> = std::slice::from_raw_parts(physx_sys::PxConvexMesh_getVertices(mesh), nv).iter().map(|v| world(scale * vec(v))).collect();
                let index = physx_sys::PxConvexMesh_getIndexBuffer(mesh);
                let middle = verts.iter().copied().sum::<Vec3>() / nv.max(1) as f32;
                for k in 0..physx_sys::PxConvexMesh_getNbPolygons(mesh) {
                    let mut poly: physx_sys::PxHullPolygon = std::mem::zeroed();
                    if !physx_sys::PxConvexMesh_getPolygonData(mesh, k, &mut poly) {
                        continue;
                    }
                    let corners: Vec<Vec3> = (0..poly.mNbVerts as usize).map(|i| verts[*index.add(poly.mIndexBase as usize + i) as usize]).collect();
                    // A fan over each face, facing out of the hull.
                    for i in 1..corners.len().saturating_sub(1) {
                        out.push_out(corners[0], corners[i], corners[i + 1], middle);
                    }
                }
            }
            G::Heightfield => {
                let g = &*(geometry as *const physx_sys::PxHeightFieldGeometry);
                let field = g.heightField as *const physx_sys::PxHeightField;
                let (rows, cols) = (physx_sys::PxHeightField_getNbRows(field), physx_sys::PxHeightField_getNbColumns(field));
                // Rows along x, columns along z (`spawn_scene_height_field`).
                let at = |r: u32, c: u32| {
                    let s = physx_sys::PxHeightField_getSample(field, r, c) as *const u8;
                    let height = i16::from_le_bytes([*s, *s.add(1)]);
                    world(Vec3::new(r as f32 * g.rowScale, height as f32 * g.heightScale, c as f32 * g.columnScale))
                };
                let base = out.vertices.len() as u32;
                for r in 0..rows {
                    for c in 0..cols {
                        out.vertices.push(at(r, c).to_array());
                    }
                }
                let id = |r: u32, c: u32| base + r * cols + c;
                for r in 0..rows.saturating_sub(1) {
                    for c in 0..cols.saturating_sub(1) {
                        let s = physx_sys::PxHeightField_getSample(field, r, c);
                        let bytes = s as *const u8;
                        let (m0, m1) = (*bytes.add(2) & 0x7f, *bytes.add(3) & 0x7f);
                        let (v0, v1, v2, v3) = (id(r, c), id(r, c + 1), id(r + 1, c), id(r + 1, c + 1));
                        // Up-facing either way; split through the first sample when flagged.
                        let (t0, t1) = if physx_sys::PxHeightFieldSample_tessFlag(s) != 0 { ([v0, v1, v3], [v0, v3, v2]) } else { ([v0, v1, v2], [v1, v3, v2]) };
                        if m0 != HOLE {
                            out.triangles.push(t0);
                        }
                        if m1 != HOLE {
                            out.triangles.push(t1);
                        }
                    }
                }
            }
            G::Plane => out.skipped.push("plane".into()),
            other => out.skipped.push(format!("{other:?}")),
        }
    }
}
