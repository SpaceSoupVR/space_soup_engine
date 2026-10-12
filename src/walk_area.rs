//! Walkable mats: a floor draped over the ground, with a fence at its edge.
//!
//! A `walk_area` object's cuboid gives the footprint (position + half_size in
//! x/z); the geometry here samples the scene's terrain across that rectangle
//! and produces a grid mesh that follows the ground -- up hills, down dips --
//! lifted a few centimetres so it sits ON the surface instead of z-fighting
//! inside it. Around the rectangle's edge an invisible fence wall rises, so
//! the player can walk everywhere on the mat and nowhere off it.
//!
//! Both consumers build from the same function so they cannot disagree:
//! `rigid_physics` cooks [`physics_triangles`] into a PhysX triangle mesh
//! (the same `cook_triangle_mesh` + static actor path the sculpted terrain
//! uses), and SSStudio's editor draws the grid so the author can SEE the mat
//! bend over the terrain before anyone walks on it.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The authorable part: everything else about the mat comes from the
/// object's own cuboid (footprint) and the scene's terrain (heights).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkAreaDef {
    /// Height (m) of the invisible wall around the mat's edge. 0 = no fence.
    #[serde(default = "default_fence_height")]
    pub fence_height: f32,
    /// How far (m) the floor sits above the sampled ground. Keeps the
    /// collider from being buried by sampling error on steep slopes.
    #[serde(default = "default_lift")]
    pub lift: f32,
    /// A level floor at the object's own height, ignoring the terrain: for a
    /// room or tent that sits under or apart from the ground.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flat: bool,
    /// Hand-pulled points on the mat, `[x, z, dy, radius]`: the floor is
    /// raised (or sunk) by `dy` metres at (x, z), easing back to the ground
    /// over `radius` metres. This is how the author reshapes the grid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bumps: Vec<[f32; 4]>,
}

/// How much of a bump's `dy` applies `dist` metres from its centre.
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

/// The draped mat as geometry: a height-following floor grid plus the fence
/// quads standing on its boundary.
pub struct WalkAreaMesh {
    /// Floor grid vertices, row-major: `(nx + 1) * (nz + 1)`, x fastest.
    pub floor: Vec<Vec3>,
    /// Floor triangles into `floor`, wound so their normals point up.
    pub floor_indices: Vec<u32>,
    /// Cells along x and z.
    pub nx: usize,
    pub nz: usize,
    /// Fence wall quads `[bottom0, bottom1, top1, top0]`, one per boundary
    /// grid edge. Empty when `fence_height` is 0.
    pub fence: Vec<[Vec3; 4]>,
}

/// Drapes the mat over the ground.
///
/// `height_at` answers in world space; `None` (no terrain there, or a scene
/// with no terrain at all) falls back to the mat's own `center.y`, so a flat
/// room gets a flat mat at the height the author placed it.
///
/// `max_cells` caps `nx * nz`: physics can afford a fine grid, a per-frame
/// editor preview wants a coarser one, and both still sample the same
/// heights where their grid points coincide.
pub fn build(
    center: Vec3,
    half_x: f32,
    half_z: f32,
    def: &WalkAreaDef,
    max_cells: usize,
    height_at: &dyn Fn(f32, f32) -> Option<f32>,
) -> WalkAreaMesh {
    let (ex, ez) = ((half_x * 2.0).max(0.1), (half_z * 2.0).max(0.1));
    // ~1 m cells, coarsened together until the cap holds.
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
            // Wound so the face normal points up (+y): PhysX raycasts skip a
            // triangle mesh's back faces, and the ground probe comes from above.
            floor_indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let mut fence = Vec::new();
    if def.fence_height > 1e-3 {
        let at = |ix: usize, iz: usize| floor[iz * (nx + 1) + ix];
        let mut wall = |p0: Vec3, p1: Vec3| {
            // The wall hugs the ground: below the lower end (so a dip can't
            // open a gap under it) and `fence_height` above the higher one.
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
            wall(at(ix, 0), at(ix + 1, 0)); // near edge (z = min)
            wall(at(ix, nz), at(ix + 1, nz)); // far edge
        }
        for iz in 0..nz {
            wall(at(0, iz), at(0, iz + 1)); // left edge (x = min)
            wall(at(nx, iz), at(nx, iz + 1)); // right edge
        }
    }

    WalkAreaMesh { floor, floor_indices, nx, nz, fence }
}

/// Everything as one triangle soup for the physics cooker: the floor as
/// built, and each fence quad twice -- once per winding -- because PhysX
/// raycasts ignore back faces and the player meets the fence from inside.
pub fn physics_triangles(mesh: &WalkAreaMesh) -> (Vec<Vec3>, Vec<u32>) {
    let mut points = mesh.floor.clone();
    let mut indices = mesh.floor_indices.clone();
    for quad in &mesh.fence {
        let base = points.len() as u32;
        points.extend_from_slice(quad);
        // Front and back of each quad as two triangle pairs.
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
        // A ramp: y = x / 2.
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
            let floor_hi = quad[0].y.max(quad[1].y) + 1.0; // bottom is 1 m under
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
