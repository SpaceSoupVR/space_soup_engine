use std::collections::HashMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};

use crate::scene::GameObject;

pub const ROOM_INSET: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionProxy {
    pub centre: Vec3,
    pub half_size: Vec3,
    pub rotation: Quat,

    pub room: usize,

    pub solid: bool,

    pub object: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProxyField {

    pub dims: [u32; 3],

    pub max_distance: f32,
    pub distances: Vec<u8>,
}

pub const FIELD_SAMPLES: u32 = 48;

const FIELD_REACH_SAMPLES: f32 = 4.0;

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

fn is_drawn_geometry(o: &GameObject) -> bool {
    !o.hidden
        && !o.is_trigger
        && o.reflection_probe.is_none()
        && o.spawn_point.is_none()
        && o.trigger_volume.is_none()

        && o.door.is_none()
        && (o.brush.is_some() || o.mesh.is_some())
}

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

fn obb_bounds(centre: Vec3, half: Vec3, rotation: Quat) -> (Vec3, Vec3) {
    let m = glam::Mat3::from_quat(rotation);
    let extent = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
    (centre - extent, centre + extent)
}

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

pub const DOOR_ROOM_REACH: f32 = 0.4;

pub fn door_proxies(game_dir: &Path, objects: &[GameObject], rooms: &[(Vec3, Vec3)]) -> Vec<ReflectionProxy> {
    let mut out = Vec::new();
    for (index, o) in objects.iter().enumerate() {
        let (Some(_), Some(mesh)) = (&o.door, &o.mesh) else { continue };
        if o.hidden {
            continue;
        }
        let Some((mlo, mhi)) = model_bounds(&game_dir.join(&mesh.path)) else {
            log::warn!("reflection proxies: could not read the bounds of door '{}'", o.id);
            continue;
        };
        let rotation = o.cuboid.rotation * mesh.rotation_offset;
        let centre = o.cuboid.position + rotation * (mesh.scale * (mlo + mhi) * 0.5);
        let half_size = (mesh.scale * (mhi - mlo) * 0.5).abs();
        for (room, (lo, hi)) in rooms.iter().enumerate() {
            let reach = Vec3::splat(DOOR_ROOM_REACH);
            if (lo.cmple(*hi)).all() && ((*lo - reach).cmple(centre) & centre.cmple(*hi + reach)).all() {
                out.push(ReflectionProxy { centre, half_size, rotation, room, solid: false, object: index });
            }
        }
    }
    out
}

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

pub const MAX_SHAPED_MODELS: usize = 8;

pub fn shaped_models(proxies: &[ReflectionProxy], objects: &[GameObject]) -> Vec<Option<u32>> {
    let mut keys: Vec<(String, [u32; 3])> = Vec::new();
    proxies
        .iter()
        .map(|p| {
            if p.solid {
                return None;
            }
            let mesh = objects.get(p.object)?.mesh.as_ref()?;
            let key = (mesh.path.clone(), mesh.scale.to_array().map(f32::to_bits));
            if let Some(i) = keys.iter().position(|k| *k == key) {
                return Some(i as u32);
            }
            if keys.len() >= MAX_SHAPED_MODELS {
                return None;
            }
            keys.push(key);
            Some(keys.len() as u32 - 1)
        })
        .collect()
}

pub fn portal_wall_extent(objects: &[GameObject], min: Vec3, max: Vec3, axis: usize) -> Option<(f32, f32)> {
    const RIM: f32 = 0.05;
    let mut out: Option<(f32, f32)> = None;
    for o in objects.iter().filter(|o| !o.hidden && o.brush.is_some()) {
        for solid in o.brush.as_ref().expect("filtered").evaluate() {
            let Some((lo, hi)) = crate::brush::solid_bounds(&solid) else { continue };
            let lo = Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32);
            let hi = Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32);

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

        assert!((point_triangle_distance(Vec3::new(0.2, 0.2, 0.5), &t) - 0.5).abs() < 1e-6);

        assert!((point_triangle_distance(Vec3::new(-1.0, -1.0, 0.0), &t) - 2f32.sqrt()).abs() < 1e-6);
        assert!((point_triangle_distance(Vec3::new(0.5, -2.0, 0.0), &t) - 2.0).abs() < 1e-6);
        assert!((point_triangle_distance(Vec3::new(1.0, 1.0, 0.0), &t) - 0.5f32.sqrt()).abs() < 1e-6);
        assert!(point_triangle_distance(Vec3::new(0.25, 0.25, 0.0), &t) < 1e-6);
    }

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

    #[test]
    fn shaped_models_are_numbered_per_model_and_scale_within_the_budget() {
        let mut objects = vec![object(r#"{"id":"pillar"}"#)];
        let mut proxies = vec![ReflectionProxy {
            centre: Vec3::ZERO,
            half_size: Vec3::ONE,
            rotation: Quat::IDENTITY,
            room: 0,
            solid: true,
            object: 0,
        }];
        let mut place = |json: &str| {
            objects.push(object(json));
            proxies.push(ReflectionProxy {
                centre: Vec3::ZERO,
                half_size: Vec3::ONE,
                rotation: Quat::IDENTITY,
                room: 0,
                solid: false,
                object: objects.len() - 1,
            });
        };
        place(r#"{"id":"sconce_a","mesh":{"path":"sconce.gltf"}}"#);
        place(r#"{"id":"lamp","mesh":{"path":"lamp.gltf"}}"#);
        place(r#"{"id":"sconce_b","mesh":{"path":"sconce.gltf"}}"#);
        place(r#"{"id":"big_sconce","mesh":{"path":"sconce.gltf","scale":[2.0,2.0,2.0]}}"#);
        for i in 0..MAX_SHAPED_MODELS {
            place(&format!(r#"{{"id":"prop_{i}","mesh":{{"path":"prop_{i}.gltf"}}}}"#));
        }
        let shaped = shaped_models(&proxies, &objects);
        assert_eq!(shaped[0], None, "a brush piece is its box");
        assert_eq!(shaped[1], Some(0));
        assert_eq!(shaped[2], Some(1));
        assert_eq!(shaped[3], Some(0), "a second sconce is the first one's model");
        assert_eq!(shaped[4], Some(2), "at another scale it is another field");
        let within = shaped[5..].iter().filter(|s| s.is_some()).count();
        assert_eq!(within, MAX_SHAPED_MODELS - 3, "{shaped:?}");
        assert_eq!(shaped.last().copied().flatten(), None, "past the budget: traced by its bounds");
    }

    const HALL: (Vec3, Vec3) = (Vec3::new(-2.7, 0.0, -15.7), Vec3::new(2.7, 3.1, 3.7));

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
