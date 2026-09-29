//! WHAT STANDS INSIDE A ROOM, as the reflection trace sees it.
//!
//! A reflected ray is traced exactly through the rooms' boxes and the doorways
//! between them (`probe_trace` in the renderer's lights block): a room's probe
//! box IS its interior, so its walls, floor and ceiling are the box's faces,
//! and a doorway is the carve its portal records. What the boxes cannot say is
//! what stands INSIDE a room -- a pillar, a hanging lamp -- and that is this:
//! one oriented box per thing, tagged with the room it stands in.
//!
//! WHY NOT THE PROBES' OWN DEPTH
//!
//! It was tried, and walked well where the photographs saw everything. They do
//! not: both of a hall's photographs are taken on its long axis, so a pillar on
//! that axis shows each of them only its front or its back. Its SIDES were seen
//! by nothing, space nobody saw past is indistinguishable from solid, and rays
//! aimed at those sides walked straight through the pillar to the lamp beyond
//! it (headset, 2026-09-26). The geometry has to come from the geometry.
//!
//! Brushes contribute each convex piece's bounds, which for the axis-aligned
//! pieces a level is built from is the piece itself. Meshes contribute their
//! model's bounds, read from the glTF's own accessor min/max under its node
//! transforms, placed with exactly the pose the renderer draws them with. A
//! room's own shell -- its walls, floor and ceiling -- lies outside its box and
//! is never included, so nothing here duplicates what the box already is.
//!
//! Plain data: `space_soup` (the renderer) cannot depend on this crate, so the
//! app converts these into its `ProbeProxy`.

use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};

use crate::scene::GameObject;

/// How far into a room's box something must reach to stand in that room.
///
/// A room's walls touch its box exactly, and float rounding would otherwise put
/// every wall piece "inside" the room it bounds.
pub const ROOM_INSET: f32 = 0.02;

/// One box standing in one room, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionProxy {
    pub centre: Vec3,
    pub half_size: Vec3,
    pub rotation: Quat,
    /// Index into the `rooms` the proxies were built against.
    pub room: usize,
    /// A brush piece IS its box. A model's box is only its bounds -- a hanging
    /// lamp's is mostly air around a cord -- so the trace asks the room's
    /// photographs where inside it the object actually is, or, better, its
    /// [`ProxyField`].
    pub solid: bool,
    /// Index into the `objects` the proxies were built from: which object this
    /// is, so a model's [`ProxyField`] can be built from its mesh.
    pub object: usize,
}

/// A STANDING MODEL'S SHAPE for the reflection trace: the distance from each
/// point of its proxy's box to the nearest of its triangles.
///
/// A model's proxy box is only its bounds, and the trace used to ask the
/// room's photographs where inside it the model was -- four samples against
/// 256-pixel depth. A hanging lamp came out as a ragged block: the dark "L" in
/// the ceiling's reflection of the front door (offline, 2026-09-28). With this
/// the trace walks the model's own surface to within a sample, the way mesh
/// distance fields are traced in Unreal's Lumen and distance-field shadows.
///
/// UNSIGNED: a lamp's shade is an open cone and its cage is wire, so there is
/// no inside to sign. The trace stops half a sample short of the surface, a
/// shell a few millimetres thick.
#[derive(Clone, Debug, PartialEq)]
pub struct ProxyField {
    /// Samples along x, y, z of the proxy's box (its own frame), x fastest.
    /// Sample (i, j, k) stands at the centre of its cell of the box.
    pub dims: [u32; 3],
    /// Metres a byte of 255 stands for. Nearer than this the field is exact;
    /// past it, it says only "at least this far".
    pub max_distance: f32,
    pub distances: Vec<u8>,
}

/// Samples along a field's longest side: 48 puts a hanging lamp's 1.4 m at
/// 3 cm and a wall sconce's 34 cm at 7 mm.
pub const FIELD_SAMPLES: u32 = 48;

/// How many samples out from the surface a field is exact. The trace steps
/// by the distance it reads, so this caps its stride.
const FIELD_REACH_SAMPLES: f32 = 4.0;

/// The field of the model at `path`, placed as an object places it: scaled by
/// `scale`, over the box [`reflection_proxies`] gives it -- the model's bounds,
/// centred on the box. `None` when the model cannot be read.
pub fn model_field(path: &Path, scale: Vec3, samples: u32) -> Option<ProxyField> {
    let (mlo, mhi) = model_bounds(path)?;
    let parts = crate::mesh_lightmap::load_mesh_parts(path)?;
    let mid = (mlo + mhi) * 0.5;
    let half = (scale * (mhi - mlo) * 0.5).abs();
    let longest = half.max_element().max(1e-4);
    let dims = [0usize, 1, 2].map(|a| ((samples.max(1) as f32 * half[a] / longest).ceil() as u32).max(1));
    let cell = Vec3::new(
        2.0 * half.x / dims[0] as f32,
        2.0 * half.y / dims[1] as f32,
        2.0 * half.z / dims[2] as f32,
    );
    let max_distance = FIELD_REACH_SAMPLES * 2.0 * longest / samples.max(1) as f32;
    let n = (dims[0] * dims[1] * dims[2]) as usize;
    let mut dist = vec![max_distance; n];
    // Each triangle updates only the samples within reach of it.
    for part in &parts {
        let local = |i: u32| scale * (Vec3::from(part.positions[i as usize]) - mid);
        for tri in part.indices.chunks_exact(3) {
            let t = [local(tri[0]), local(tri[1]), local(tri[2])];
            let lo = t[0].min(t[1]).min(t[2]) - Vec3::splat(max_distance);
            let hi = t[0].max(t[1]).max(t[2]) + Vec3::splat(max_distance);
            let range = |a: usize| {
                let f = |v: f32| (v + half[a]) / cell[a].max(1e-9) - 0.5;
                let first = f(lo[a]).floor().max(0.0) as u32;
                let last = (f(hi[a]).ceil().max(0.0) as u32).min(dims[a] - 1);
                first..=last
            };
            for k in range(2) {
                for j in range(1) {
                    for i in range(0) {
                        let p = Vec3::new(
                            -half.x + (i as f32 + 0.5) * cell.x,
                            -half.y + (j as f32 + 0.5) * cell.y,
                            -half.z + (k as f32 + 0.5) * cell.z,
                        );
                        let d = point_triangle_distance(p, &t);
                        let slot = &mut dist[((k * dims[1] + j) * dims[0] + i) as usize];
                        if d < *slot {
                            *slot = d;
                        }
                    }
                }
            }
        }
    }
    let distances = dist.iter().map(|d| ((d / max_distance).clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    Some(ProxyField { dims, max_distance, distances })
}

/// Distance from `p` to the triangle `t` (Ericson, *Real-Time Collision
/// Detection* 5.1.5: the closest point by Voronoi region).
fn point_triangle_distance(p: Vec3, t: &[Vec3; 3]) -> f32 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return p.distance(a);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return p.distance(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return p.distance(a + ab * v);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return p.distance(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return p.distance(a + ac * w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return p.distance(b + (c - b) * w);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    p.distance(a + ab * v + ac * w)
}

/// Whether an object is drawn geometry a reflection could show.
///
/// The same rule the baker's `occlusion::occludes_light` applies: markers --
/// probes, spawn points, triggers -- have cuboids that are editor handles, and
/// hidden objects are not drawn at all.
fn is_drawn_geometry(o: &GameObject) -> bool {
    !o.hidden
        && !o.is_trigger
        && o.reflection_probe.is_none()
        && o.spawn_point.is_none()
        && o.trigger_volume.is_none()
        && (o.brush.is_some() || o.mesh.is_some())
}

/// The model-space bounds of a glTF's default scene, from each primitive's
/// POSITION accessor min/max (which the format requires) under its node's
/// world transform. No vertex buffer is read.
pub fn model_bounds(path: &Path) -> Option<(Vec3, Vec3)> {
    let gltf = gltf::Gltf::open(path).ok()?;
    let doc = &gltf.document;
    let scene = doc.default_scene().or_else(|| doc.scenes().next())?;
    let mut acc: Option<(Vec3, Vec3)> = None;
    fn walk(node: gltf::Node, parent: Mat4, acc: &mut Option<(Vec3, Vec3)>) {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                let b = prim.bounding_box();
                let (lo, hi) = (Vec3::from(b.min), Vec3::from(b.max));
                for i in 0..8 {
                    let corner = Vec3::new(
                        if i & 1 == 0 { lo.x } else { hi.x },
                        if i & 2 == 0 { lo.y } else { hi.y },
                        if i & 4 == 0 { lo.z } else { hi.z },
                    );
                    let p = world.transform_point3(corner);
                    *acc = Some(match *acc {
                        None => (p, p),
                        Some((a, b)) => (a.min(p), b.max(p)),
                    });
                }
            }
        }
        for child in node.children() {
            walk(child, world, acc);
        }
    }
    for node in scene.nodes() {
        walk(node, Mat4::IDENTITY, &mut acc);
    }
    acc
}

/// The world-space bounds of an oriented box.
fn obb_bounds(centre: Vec3, half: Vec3, rotation: Quat) -> (Vec3, Vec3) {
    let m = glam::Mat3::from_quat(rotation);
    let extent = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
    (centre - extent, centre + extent)
}

/// The room `lo..hi` stands in, as an index into `rooms`: the TIGHTEST box it
/// reaches into. Rooms nest -- a hall inside the outdoor volume, an alcove
/// inside a hall -- and a pillar in the hall stands in the hall, not in the
/// sky. An empty box (min above max) is a room nothing stands in.
fn room_reached(lo: Vec3, hi: Vec3, rooms: &[(Vec3, Vec3)]) -> Option<usize> {
    rooms
        .iter()
        .enumerate()
        .filter(|(_, (rlo, rhi))| {
            let a = *rlo + Vec3::splat(ROOM_INSET);
            let b = *rhi - Vec3::splat(ROOM_INSET);
            lo.cmplt(b).all() && hi.cmpgt(a).all()
        })
        .min_by(|(_, (alo, ahi)), (_, (blo, bhi))| {
            let va = (*ahi - *alo).max(Vec3::ZERO).element_product();
            let vb = (*bhi - *blo).max(Vec3::ZERO).element_product();
            va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

/// What stands in each of `rooms` (their probe boxes, in the caller's room
/// order), for the reflection trace. Mesh paths resolve against `game_dir`; a
/// model that cannot be read contributes nothing and is logged.
pub fn reflection_proxies(game_dir: &Path, objects: &[GameObject], rooms: &[(Vec3, Vec3)]) -> Vec<ReflectionProxy> {
    let mut out = Vec::new();
    let mut bounds_cache: HashMap<String, Option<(Vec3, Vec3)>> = HashMap::new();
    for (index, o) in objects.iter().enumerate().filter(|(_, o)| is_drawn_geometry(o)) {
        if let Some(brush) = &o.brush {
            for solid in brush.evaluate() {
                let Some((lo, hi)) = crate::brush::solid_bounds(&solid) else { continue };
                let lo = Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32);
                let hi = Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32);
                if let Some(room) = room_reached(lo, hi, rooms) {
                    out.push(ReflectionProxy {
                        centre: (lo + hi) * 0.5,
                        half_size: (hi - lo) * 0.5,
                        rotation: Quat::IDENTITY,
                        room,
                        solid: true,
                        object: index,
                    });
                }
            }
        } else if let Some(mesh) = &o.mesh {
            let bounds = *bounds_cache.entry(mesh.path.clone()).or_insert_with(|| {
                let b = model_bounds(&game_dir.join(&mesh.path));
                if b.is_none() {
                    log::warn!("reflection proxies: could not read the bounds of '{}'", mesh.path);
                }
                b
            });
            let Some((mlo, mhi)) = bounds else { continue };
            // The pose the renderer draws the model with: see
            // `quest_app::scene_meshes` (position, rotation * offset, scale).
            let rotation = o.cuboid.rotation * mesh.rotation_offset;
            let centre = o.cuboid.position + rotation * (mesh.scale * (mlo + mhi) * 0.5);
            let half_size = (mesh.scale * (mhi - mlo) * 0.5).abs();
            let (lo, hi) = obb_bounds(centre, half_size, rotation);
            if let Some(room) = room_reached(lo, hi, rooms) {
                out.push(ReflectionProxy { centre, half_size, rotation, room, solid: false, object: index });
            }
        }
    }
    out
}

/// HOW DEEP A DOORWAY'S JAMBS ARE along its axis: the extent of the wall the
/// opening is cut through, from the brush pieces around its rim.
///
/// A portal records its CARVE, which reaches past the wall on both sides (a
/// front door's carve runs 3.69..4.4 through a wall 3.7..4.0). Between two
/// rooms the next room's box bounds the jambs anyway; out of the rooms there
/// is no next box, and treating the whole carve as jambs would report hits on
/// sides that end 40 cm earlier. `None` when no piece touches the rim.
pub fn portal_wall_extent(objects: &[GameObject], min: Vec3, max: Vec3, axis: usize) -> Option<(f32, f32)> {
    const RIM: f32 = 0.05;
    let mut out: Option<(f32, f32)> = None;
    for o in objects.iter().filter(|o| !o.hidden && o.brush.is_some()) {
        for solid in o.brush.as_ref().expect("filtered").evaluate() {
            let Some((lo, hi)) = crate::brush::solid_bounds(&solid) else { continue };
            let lo = Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32);
            let hi = Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32);
            // Touching the opening's rim across the axis, overlapping it along.
            let touches = (0..3).all(|i| {
                if i == axis {
                    lo[i] < max[i] && hi[i] > min[i]
                } else {
                    lo[i] <= max[i] + RIM && hi[i] >= min[i] - RIM
                }
            });
            if !touches {
                continue;
            }
            let (a, b) = (lo[axis].max(min[axis]), hi[axis].min(max[axis]));
            if b <= a {
                continue;
            }
            out = Some(match out {
                None => (a, b),
                Some((x, y)) => (x.min(a), y.max(b)),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_points_distance_to_a_triangle_is_to_its_nearest_feature() {
        let t = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        // Over the face.
        assert!((point_triangle_distance(Vec3::new(0.2, 0.2, 0.5), &t) - 0.5).abs() < 1e-6);
        // Off a corner, an edge, and in the plane.
        assert!((point_triangle_distance(Vec3::new(-1.0, -1.0, 0.0), &t) - 2f32.sqrt()).abs() < 1e-6);
        assert!((point_triangle_distance(Vec3::new(0.5, -2.0, 0.0), &t) - 2.0).abs() < 1e-6);
        assert!((point_triangle_distance(Vec3::new(1.0, 1.0, 0.0), &t) - 0.5f32.sqrt()).abs() < 1e-6);
        assert!(point_triangle_distance(Vec3::new(0.25, 0.25, 0.0), &t) < 1e-6);
    }

    /// A REAL FIXTURE: the wall sconce test_room hangs in its hallway. The field
    /// spans the model's box at the object's scale, reaches zero on its
    /// surface and is largest in the empty corners of its bounds.
    #[test]
    fn a_fixtures_field_finds_its_surface() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../game/models/lights/industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        if !path.is_file() {
            eprintln!("skipping: {} is not installed", path.display());
            return;
        }
        let f = model_field(&path, Vec3::ONE, FIELD_SAMPLES).expect("the sconce's field");
        let (lo, hi) = model_bounds(&path).unwrap();
        let half = (hi - lo) * 0.5;
        assert_eq!(f.dims.iter().max().copied(), Some(FIELD_SAMPLES), "{:?}", f.dims);
        for a in 0..3 {
            let expect = (FIELD_SAMPLES as f32 * half[a] / half.max_element()).ceil() as u32;
            assert_eq!(f.dims[a], expect.max(1), "axis {a}: {:?} for half {half:?}", f.dims);
        }
        assert_eq!(f.distances.len() as u32, f.dims[0] * f.dims[1] * f.dims[2]);
        let near = f.distances.iter().filter(|&&d| d < 32).count();
        let far = f.distances.iter().filter(|&&d| d == 255).count();
        assert!(near > 0, "no sample lies within an eighth of the reach of the surface");
        assert!(far > 0, "every sample is near a surface -- the field says nothing");
        eprintln!("sconce field {:?}, {:.1} mm reach, {near} near / {far} far", f.dims, f.max_distance * 1000.0);
    }

    fn object(json: &str) -> GameObject {
        serde_json::from_str(json).expect("the test object parses")
    }

    const HALL: (Vec3, Vec3) = (Vec3::new(-2.7, 0.0, -15.7), Vec3::new(2.7, 3.1, 3.7));

    /// A room's shell lies outside its box and must never be handed to the
    /// trace as something standing in the room -- it would stop every ray at
    /// the first wall piece's bounds, which for a wall with a doorway cut in
    /// it spans the doorway.
    #[test]
    fn a_rooms_own_walls_are_not_proxies_but_a_pillar_is() {
        let shell = object(
            r#"{"id":"shell","brush":{"solids":[{"faces":[
                {"plane":[1,0,0,3.0],"material":"m"},{"plane":[-1,0,0,3.0],"material":"m"},
                {"plane":[0,1,0,3.4],"material":"m"},{"plane":[0,-1,0,0.3],"material":"m"},
                {"plane":[0,0,1,4.0],"material":"m"},{"plane":[0,0,-1,16.0],"material":"m"}]}],
              "subtract":[{"faces":[
                {"plane":[1,0,0,2.7],"material":"m"},{"plane":[-1,0,0,2.7],"material":"m"},
                {"plane":[0,1,0,3.1],"material":"m"},{"plane":[0,-1,0,0.0],"material":"m"},
                {"plane":[0,0,1,3.7],"material":"m"},{"plane":[0,0,-1,15.7],"material":"m"}]}]}}"#,
        );
        let pillar = object(
            r#"{"id":"pillar","brush":{"solids":[{"faces":[
                {"plane":[1,0,0,0.45],"material":"m"},{"plane":[-1,0,0,0.45],"material":"m"},
                {"plane":[0,1,0,3.1],"material":"m"},{"plane":[0,-1,0,0.0],"material":"m"},
                {"plane":[0,0,1,-6.55],"material":"m"},{"plane":[0,0,-1,7.45],"material":"m"}]}]}}"#,
        );
        let p = reflection_proxies(Path::new("."), &[shell, pillar], &[HALL]);
        assert_eq!(p.len(), 1, "only the pillar stands in the hall: {p:?}");
        assert!((p[0].centre - Vec3::new(0.0, 1.55, -7.0)).length() < 1e-4);
        assert!((p[0].half_size - Vec3::new(0.45, 1.55, 0.45)).length() < 1e-4);
        assert_eq!(p[0].room, 0);
    }

    /// Markers are editor handles, and hidden objects are not drawn.
    #[test]
    fn markers_and_hidden_objects_are_not_proxies() {
        let hidden = object(
            r#"{"id":"h","hidden":true,"brush":{"solids":[{"faces":[
                {"plane":[1,0,0,0.5],"material":"m"},{"plane":[-1,0,0,0.5],"material":"m"},
                {"plane":[0,1,0,1.0],"material":"m"},{"plane":[0,-1,0,0.0],"material":"m"},
                {"plane":[0,0,1,-5.0],"material":"m"},{"plane":[0,0,-1,6.0],"material":"m"}]}]}}"#,
        );
        let probe = object(r#"{"id":"p","cuboid":{"position":[0,1.5,-6],"half_size":[2,1,2]},"reflection_probe":{}}"#);
        assert!(reflection_proxies(Path::new("."), &[hidden, probe], &[HALL]).is_empty());
    }

    /// Rooms nest. A pillar inside the hall is inside the outdoor volume too,
    /// and stands in the hall.
    #[test]
    fn a_proxy_stands_in_the_tightest_room() {
        let pillar = object(
            r#"{"id":"pillar","brush":{"solids":[{"faces":[
                {"plane":[1,0,0,0.45],"material":"m"},{"plane":[-1,0,0,0.45],"material":"m"},
                {"plane":[0,1,0,3.1],"material":"m"},{"plane":[0,-1,0,0.0],"material":"m"},
                {"plane":[0,0,1,-6.55],"material":"m"},{"plane":[0,0,-1,7.45],"material":"m"}]}]}}"#,
        );
        let outdoors = (Vec3::splat(-30.0), Vec3::splat(30.0));
        let p = reflection_proxies(Path::new("."), &[pillar], &[outdoors, HALL]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].room, 1, "the pillar was filed under the enclosing volume");
    }

    /// The front door's carve runs 3.69..4.4 through a wall 3.7..4.0: the
    /// jambs are the wall's depth, not the carve's.
    #[test]
    fn a_doorways_jambs_are_as_deep_as_its_wall() {
        let shell = object(
            r#"{"id":"shell","brush":{"solids":[{"faces":[
                {"plane":[1,0,0,3.0],"material":"m"},{"plane":[-1,0,0,3.0],"material":"m"},
                {"plane":[0,1,0,3.4],"material":"m"},{"plane":[0,-1,0,0.3],"material":"m"},
                {"plane":[0,0,1,4.0],"material":"m"},{"plane":[0,0,-1,16.0],"material":"m"}]}],
              "subtract":[{"faces":[
                {"plane":[1,0,0,2.7],"material":"m"},{"plane":[-1,0,0,2.7],"material":"m"},
                {"plane":[0,1,0,3.1],"material":"m"},{"plane":[0,-1,0,0.0],"material":"m"},
                {"plane":[0,0,1,3.7],"material":"m"},{"plane":[0,0,-1,15.7],"material":"m"}]},
              {"faces":[
                {"plane":[1,0,0,0.8],"material":"m"},{"plane":[-1,0,0,0.8],"material":"m"},
                {"plane":[0,1,0,2.2],"material":"m"},{"plane":[0,-1,0,0.0],"material":"m"},
                {"plane":[0,0,1,4.4],"material":"m"},{"plane":[0,0,-1,-3.69],"material":"m"}]}]}}"#,
        );
        let (a, b) = portal_wall_extent(&[shell], Vec3::new(-0.8, 0.0, 3.69), Vec3::new(0.8, 2.2, 4.4), 2).expect("a wall");
        assert!((a - 3.69).abs() < 1e-4 && (b - 4.0).abs() < 1e-4, "jambs {a}..{b}");
    }

    #[test]
    fn a_rotated_box_reaches_as_far_as_its_corners() {
        let (lo, hi) = obb_bounds(Vec3::ZERO, Vec3::new(1.0, 0.1, 0.1), Quat::from_rotation_y(std::f32::consts::FRAC_PI_4));
        let r = std::f32::consts::FRAC_1_SQRT_2 * 1.1;
        assert!((hi.x - r).abs() < 1e-5 && (lo.z + r).abs() < 1e-5, "{lo} {hi}");
    }
}
