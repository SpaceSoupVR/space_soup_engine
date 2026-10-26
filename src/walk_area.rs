use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkAreaDef {

    #[serde(default = "default_fence_height")]
    pub fence_height: f32,

    #[serde(default = "default_lift")]
    pub lift: f32,

    #[serde(default, skip_serializing_if = "is_false")]
    pub flat: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bumps: Vec<[f32; 4]>,
}

#[must_use]
pub fn bump_falloff(dist: f32, radius: f32) -> f32 {
    if radius <= 1e-4 || dist >= radius {
        return 0.0;
    }
    let t = 1.0 - dist / radius;
    t * t * (3.0 - 2.0 * t)
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_fence_height() -> f32 {
    2.5
}
fn default_lift() -> f32 {
    0.05
}

impl Default for WalkAreaDef {
    fn default() -> Self {
        Self { fence_height: default_fence_height(), lift: default_lift(), flat: false, bumps: Vec::new() }
    }
}

pub struct WalkAreaMesh {

    pub floor: Vec<Vec3>,

    pub floor_indices: Vec<u32>,

    pub nx: usize,
    pub nz: usize,

    pub fence: Vec<[Vec3; 4]>,
}

pub fn build(
    center: Vec3,
    half_x: f32,
    half_z: f32,
    def: &WalkAreaDef,
    max_cells: usize,
    height_at: &dyn Fn(f32, f32) -> Option<f32>,
) -> WalkAreaMesh {
    let (ex, ez) = ((half_x * 2.0).max(0.1), (half_z * 2.0).max(0.1));

    let mut nx = (ex.ceil() as usize).max(1);
    let mut nz = (ez.ceil() as usize).max(1);
    while nx * nz > max_cells.max(1) {
        if nx >= nz {
            nx = (nx / 2).max(1);
        } else {
            nz = (nz / 2).max(1);
        }
        if nx == 1 && nz == 1 {
            break;
        }
    }
    let (x0, z0) = (center.x - half_x, center.z - half_z);
    let (sx, sz) = (ex / nx as f32, ez / nz as f32);

    let mut floor = Vec::with_capacity((nx + 1) * (nz + 1));
    for iz in 0..=nz {
        for ix in 0..=nx {
            let (x, z) = (x0 + ix as f32 * sx, z0 + iz as f32 * sz);
            let pulled: f32 = def
                .bumps
                .iter()
                .map(|b| b[2] * bump_falloff(((x - b[0]).powi(2) + (z - b[1]).powi(2)).sqrt(), b[3]))
                .sum();
            let ground = if def.flat { None } else { height_at(x, z) };
            let y = ground.unwrap_or(center.y) + def.lift + pulled;
            floor.push(Vec3::new(x, y, z));
        }
    }

    let stride = (nx + 1) as u32;
    let mut floor_indices = Vec::with_capacity(nx * nz * 6);
    for iz in 0..nz as u32 {
        for ix in 0..nx as u32 {
            let a = iz * stride + ix;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;

            floor_indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let mut fence = Vec::new();
    if def.fence_height > 1e-3 {
        let at = |ix: usize, iz: usize| floor[iz * (nx + 1) + ix];
        let mut wall = |p0: Vec3, p1: Vec3| {

            let bottom = p0.y.min(p1.y) - 1.0;
            let top = p0.y.max(p1.y) + def.fence_height;
            fence.push([
                Vec3::new(p0.x, bottom, p0.z),
                Vec3::new(p1.x, bottom, p1.z),
                Vec3::new(p1.x, top, p1.z),
                Vec3::new(p0.x, top, p0.z),
            ]);
        };
        for ix in 0..nx {
            wall(at(ix, 0), at(ix + 1, 0));
            wall(at(ix, nz), at(ix + 1, nz));
        }
        for iz in 0..nz {
            wall(at(0, iz), at(0, iz + 1));
            wall(at(nx, iz), at(nx, iz + 1));
        }
    }

    WalkAreaMesh { floor, floor_indices, nx, nz, fence }
}

#[must_use]
pub fn physics_triangles(mesh: &WalkAreaMesh) -> (Vec<Vec3>, Vec<u32>) {
    let mut points = mesh.floor.clone();
    let mut indices = mesh.floor_indices.clone();
    for quad in &mesh.fence {
        let base = points.len() as u32;
        points.extend_from_slice(quad);

        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    (points, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> WalkAreaDef {
        WalkAreaDef::default()
    }

    #[test]
    fn a_flat_scene_gets_a_flat_mat_at_the_objects_height() {
        let m = build(Vec3::new(0.0, 1.5, 0.0), 4.0, 4.0, &def(), 16 * 16, &|_, _| None);
        assert!(!m.floor.is_empty());
        for v in &m.floor {
            assert!((v.y - (1.5 + def().lift)).abs() < 1e-5);
        }
    }

    #[test]
    fn the_floor_follows_the_ground_up_a_hill() {

        let m = build(Vec3::ZERO, 8.0, 2.0, &def(), 64 * 64, &|x, _| Some(x * 0.5));
        let (min_y, max_y) = m
            .floor
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.y), hi.max(v.y)));
        assert!(max_y - min_y > 7.0, "mat should climb the ramp, rose {}", max_y - min_y);
        for v in &m.floor {
            assert!((v.y - (v.x * 0.5 + def().lift)).abs() < 1e-4);
        }
    }

    #[test]
    fn the_fence_rings_the_whole_boundary_and_rises_above_it() {
        let m = build(Vec3::ZERO, 3.0, 2.0, &def(), 1024, &|x, _| Some(x * 0.3));
        assert_eq!(m.fence.len(), 2 * m.nx + 2 * m.nz);
        for quad in &m.fence {
            let floor_hi = quad[0].y.max(quad[1].y) + 1.0;
            assert!(quad[2].y >= floor_hi + def().fence_height - 1e-4);
        }
    }

    #[test]
    fn fence_height_zero_means_no_fence() {
        let d = WalkAreaDef { fence_height: 0.0, lift: 0.05, flat: false, bumps: Vec::new() };
        let m = build(Vec3::ZERO, 3.0, 3.0, &d, 1024, &|_, _| None);
        assert!(m.fence.is_empty());
    }

    #[test]
    fn the_preview_cap_coarsens_the_grid_without_losing_the_footprint() {
        let m = build(Vec3::ZERO, 100.0, 100.0, &def(), 2048, &|_, _| None);
        assert!(m.nx * m.nz <= 2048);
        let first = m.floor.first().unwrap();
        let last = m.floor.last().unwrap();
        assert!((first.x + 100.0).abs() < 1e-3 && (last.x - 100.0).abs() < 1e-3);
        assert!((first.z + 100.0).abs() < 1e-3 && (last.z - 100.0).abs() < 1e-3);
    }

    #[test]
    fn physics_soup_doubles_the_fence_faces() {
        let m = build(Vec3::ZERO, 2.0, 2.0, &def(), 1024, &|_, _| None);
        let (points, indices) = physics_triangles(&m);
        assert_eq!(points.len(), m.floor.len() + m.fence.len() * 4);
        assert_eq!(indices.len(), m.floor_indices.len() + m.fence.len() * 12);
        assert!(indices.iter().all(|&i| (i as usize) < points.len()));
    }
}
