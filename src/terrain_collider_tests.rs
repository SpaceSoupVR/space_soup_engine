//! THE GROUND'S COLLIDER AGAINST THE GROUND THAT IS DRAWN.
//!
//! Physics holds a sculpted ground as a PhysX height field
//! (`PhysicsWorld::spawn_scene_height_field`) where it used to cook the
//! mesher's own triangles. That is only the same ground if every cell is
//! split along the diagonal the mesher draws, every hole is the mesher's hole,
//! and the triangles a cave put back are still there -- none of which a ball
//! landing on a plateau can tell. So this casts a ray down on every part of a
//! ground with relief, holes and a patch, and compares each hit with the
//! triangle of `patch(bounds, 1)` under it: the height to a millimetre, and
//! the normal, which names the triangle even where two halves of a quad agree
//! on a height.
//!
//! The same sweep is run on the triangle mesh first. It passes there by
//! construction, which is what makes a failure on the height field mean the
//! height field is wrong rather than the reference.

use std::collections::HashMap;
use std::path::Path;

use glam::{DVec2, Vec3};

use crate::rigid_physics::{PhysicsWorld, TerrainCollider};
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::Scene;
use crate::terrain::{GroundPatch, Heightfield, HoleMask, TerrainPatch, TerrainSource};

fn mix(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^ (v >> 16)
}

/// The next of a fixed run of numbers in 0..1.
fn next(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (mix(*state) >> 8) as f32 / (1u32 << 24) as f32
}

/// 49 x 41 samples -- not square, and half a metre apart one way and 0.7 m
/// the other, so rows taken for columns cannot pass -- with hills a few
/// metres high and a roughness on every sample that twists nearly every quad,
/// one way or the other. Its origin and size are ones whose far edge the
/// mesher's arithmetic falls short of along x (see
/// `Heightfield::meshed_cells`), so the last column of cells is not drawn.
const NX: u32 = 49;
const NZ: u32 = 41;

fn relief(height_range: [f32; 2]) -> Heightfield {
    let mut samples = Vec::with_capacity((NX * NZ) as usize);
    for iz in 0..NZ {
        for ix in 0..NX {
            let (x, z) = (ix as f32, iz as f32);
            let hills = 0.5 + 0.22 * (x * 0.31).sin() * (z * 0.23).cos() + 0.16 * ((x + z) * 0.17).sin();
            let rough = (mix(ix * 7919 + iz * 104_729 + 17) & 0xffff) as f32 / 65535.0 - 0.5;
            samples.push(((hills + rough * 0.05).clamp(0.0, 1.0) * 65535.0) as u16);
        }
    }
    Heightfield::new(samples, [NX, NZ], [24.0, 28.0], height_range, Vec3::new(9.1, 2.1, 11.9)).unwrap()
}

/// The cells of the fixture with no ground, at the cells' own resolution: a
/// 3 x 3 block a cave half fills again (`cave_patch`), a 3 x 6 opening, one
/// cell alone, and a cell in each of two corners.
fn cell_holes() -> HoleMask {
    let mut mask = HoleMask::solid([NX - 1, NZ - 1]);
    for iz in 15..18 {
        for ix in 20..23 {
            mask.set_open(ix, iz, true);
        }
    }
    for iz in 30..36 {
        for ix in 5..8 {
            mask.set_open(ix, iz, true);
        }
    }
    mask.set_open(40, 3, true);
    mask.set_open(0, 0, true);
    mask.set_open(NX - 2, NZ - 2, true);
    mask
}

/// GROUND A CAVE PUT BACK in the 3 x 3 block: a fan from a vertex of its own
/// in the middle of the block, 0.8 m under it, out to the block's outline at
/// the grid's own samples -- with three of the twelve triangles left out, so
/// part of the block is still an opening. Wound as the grid is.
fn cave_patch(bare: &Heightfield) -> GroundPatch {
    let grid = bare.patch(bare.bounds(), 1);
    let stride = bare.meshed_cells()[0] + 1;
    let at = |ix: u32, iz: u32| grid.positions[(iz * stride + ix) as usize];
    // Round the outline: along +z, then +x, then back.
    let ring: [(u32, u32); 12] =
        [(20, 15), (20, 16), (20, 17), (20, 18), (21, 18), (22, 18), (23, 18), (23, 17), (23, 16), (23, 15), (22, 15), (21, 15)];
    let mut positions: Vec<Vec3> = ring.iter().map(|&(ix, iz)| at(ix, iz)).collect();
    let middle = positions.iter().copied().sum::<Vec3>() / 12.0 - Vec3::Y * 0.8;
    positions.push(middle);
    let mut indices = Vec::new();
    for k in 0..12u32 {
        if (3..6).contains(&k) {
            continue;
        }
        indices.extend([k, (k + 1) % 12, 12]);
    }
    let mut normals = vec![Vec3::ZERO; 12];
    normals.push(Vec3::Y);
    GroundPatch { positions, normals, indices }
}

fn relief_with_holes_and_a_patch() -> Heightfield {
    let bare = relief([-6.0, 18.0]);
    let patch = cave_patch(&bare);
    bare.with_holes(Some(cell_holes())).with_ground_patch(Some(patch))
}

/// The triangles of the drawn ground, found by the cell they lie over.
struct Drawn {
    patch: TerrainPatch,
    origin: DVec2,
    cell: DVec2,
    cells: [usize; 2],
    bins: Vec<Vec<u32>>,
}

impl Drawn {
    fn of(field: &Heightfield) -> Self {
        let patch = field.patch(field.bounds(), 1);
        let [dx, dz] = field.cell_size();
        let cells = [(NX - 1) as usize, (NZ - 1) as usize];
        let (origin, cell) = (DVec2::new(field.origin().x as f64, field.origin().z as f64), DVec2::new(dx as f64, dz as f64));
        let mut bins = vec![Vec::new(); cells[0] * cells[1]];
        for (t, tri) in patch.indices.chunks_exact(3).enumerate() {
            let p = [0, 1, 2].map(|k| patch.positions[tri[k] as usize]);
            let lo = p[0].min(p[1]).min(p[2]);
            let hi = p[0].max(p[1]).max(p[2]);
            let span = |lo: f32, hi: f32, o: f64, c: f64, n: usize| {
                let a = (((lo as f64 - o) / c).floor() - 1.0).max(0.0) as usize;
                let b = ((((hi as f64 - o) / c).floor() + 1.0).max(0.0) as usize).min(n - 1);
                a..=b
            };
            for iz in span(lo.z, hi.z, origin.y, cell.y, cells[1]) {
                for ix in span(lo.x, hi.x, origin.x, cell.x, cells[0]) {
                    bins[iz * cells[0] + ix].push(t as u32);
                }
            }
        }
        Self { patch, origin, cell, cells, bins }
    }

    fn corners(&self, t: u32) -> [Vec3; 3] {
        let tri = &self.patch.indices[t as usize * 3..t as usize * 3 + 3];
        [0, 1, 2].map(|k| self.patch.positions[tri[k] as usize])
    }

    /// What a ray straight down at (`x`, `z`) should meet.
    fn under(&self, x: f32, z: f32) -> Under {
        /// How far inside a triangle, as a share of it, a point has to be to
        /// be asked about; and how far outside every one to be called open.
        const EDGE: f64 = 0.02;
        let p = DVec2::new(x as f64, z as f64);
        let cx = ((p.x - self.origin.x) / self.cell.x).floor();
        let cz = ((p.y - self.origin.y) / self.cell.y).floor();
        if cx < 0.0 || cz < 0.0 || cx as usize >= self.cells[0] || cz as usize >= self.cells[1] {
            return Under::Edge;
        }
        let (mut best, mut near) = (None::<(f32, Vec3)>, false);
        for &t in &self.bins[cz as usize * self.cells[0] + cx as usize] {
            let c = self.corners(t);
            let q = c.map(|v| DVec2::new(v.x as f64, v.z as f64));
            let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
            if area.abs() < 1e-12 {
                continue;
            }
            let w = [(q[1] - p).perp_dot(q[2] - p) / area, (q[2] - p).perp_dot(q[0] - p) / area, (q[0] - p).perp_dot(q[1] - p) / area];
            if w.iter().all(|w| *w >= EDGE) {
                let y = (w[0] * c[0].y as f64 + w[1] * c[1].y as f64 + w[2] * c[2].y as f64) as f32;
                let n = (c[1] - c[0]).cross(c[2] - c[0]).normalize();
                if best.is_none_or(|(by, _)| y > by) {
                    best = Some((y, n));
                }
            } else if w.iter().all(|w| *w >= -EDGE) {
                near = true;
            }
        }
        match best {
            Some((y, n)) => Under::Ground(y, n),
            None if near => Under::Edge,
            None => Under::Open,
        }
    }
}

enum Under {
    /// A triangle, well inside it: its height there, and its normal.
    Ground(f32, Vec3),
    /// No triangle anywhere near: a hole, or past the last drawn cell.
    Open,
    /// On an edge, where a hit and a miss are both right; not asked.
    Edge,
}

#[derive(Debug, Default)]
struct Swept {
    hits: usize,
    open: usize,
    edge: usize,
    /// Cells split along the diagonal b-c, and along a-d.
    splits: [usize; 2],
}

/// Cast down on sixteen jittered points a cell and compare with the drawn
/// ground; then up from under it, where nothing may be hit.
fn sweep(field: &Heightfield, kind: TerrainCollider, expect_a_height_field: bool) -> Swept {
    let mut physics = PhysicsWorld::new();
    physics.set_terrain_collider(kind);
    physics.spawn_scene_terrain(field);
    assert_eq!(
        physics.terrain_height_fields.len(),
        usize::from(expect_a_height_field),
        "{kind:?}: the ground was not built as this test means to test it"
    );

    let drawn = Drawn::of(field);
    let bounds = field.bounds();
    let [dx, dz] = field.cell_size();
    let grid = relief_grid(field);
    // By the higher and the lower end, whichever way round the range is written.
    let (high, low) = (bounds.max.y.max(bounds.min.y), bounds.max.y.min(bounds.min.y));
    let (top, reach) = (high + 3.0, high - low + 10.0);
    let mut out = Swept::default();
    let mut state = 0x5eed_u32;
    // A cell with a twist of more than this between its two possible splits
    // has to have been asked at a point that tells them apart.
    const TWIST: f32 = 0.05;
    const TELLS: f32 = 0.005;
    let mut told: HashMap<(u32, u32), bool> = HashMap::new();

    for iz in 0..NZ - 1 {
        for ix in 0..NX - 1 {
            let [ha, hb, hc, hd] = grid(ix, iz);
            for k in 0..16u32 {
                let (tx, tz) = (((k % 4) as f32 + next(&mut state)) / 4.0, ((k / 4) as f32 + next(&mut state)) / 4.0);
                let (x, z) = (bounds.min.x + (ix as f32 + tx) * dx, bounds.min.z + (iz as f32 + tz) * dz);
                let hit = physics.raycast_down(Vec3::new(x, top, z), reach);
                match drawn.under(x, z) {
                    Under::Ground(y, n) => {
                        let Some((at, normal)) = hit else {
                            panic!("{kind:?}: no ground under ({x}, {z}) in cell ({ix}, {iz}), where the drawn ground is at {y}");
                        };
                        assert!(
                            (at.y - y).abs() < 1e-3,
                            "{kind:?}: the ray at ({x}, {z}) in cell ({ix}, {iz}) met y = {}, and the drawn triangle there is at {y} -- {} mm apart",
                            at.y,
                            (at.y - y) * 1000.0
                        );
                        assert!(
                            normal.dot(n) > 0.9995,
                            "{kind:?}: the ray at ({x}, {z}) in cell ({ix}, {iz}) met a face turned {normal:?}, and the drawn triangle there is turned {n:?}"
                        );
                        out.hits += 1;
                        // The two heights this point could have had: under
                        // the split b-c, and under a-d.
                        let bc = if tx + tz <= 1.0 { ha + (hb - ha) * tx + (hc - ha) * tz } else { hd + (hc - hd) * (1.0 - tx) + (hb - hd) * (1.0 - tz) };
                        let ad = if tx >= tz { ha + (hb - ha) * tx + (hd - hb) * tz } else { ha + (hc - ha) * tz + (hd - hc) * tx };
                        if (bc - ad).abs() > TELLS && ((y - bc).abs() < 1e-3 || (y - ad).abs() < 1e-3) {
                            told.insert((ix, iz), true);
                        }
                    }
                    Under::Open => {
                        assert!(hit.is_none(), "{kind:?}: the ray at ({x}, {z}) in open cell ({ix}, {iz}) met {hit:?}");
                        out.open += 1;
                    }
                    Under::Edge => out.edge += 1,
                }
            }
            let through_first = field.cell_splits_through_first_sample(ix, iz);
            let is_drawn = !field.cell_is_open(ix, iz) && ix < field.meshed_cells()[0] && iz < field.meshed_cells()[1];
            if is_drawn {
                out.splits[usize::from(through_first)] += 1;
                if (ha + hd - hb - hc).abs() > TWIST {
                    assert!(
                        told.contains_key(&(ix, iz)),
                        "cell ({ix}, {iz}) is twisted by {} m and no point asked in it could tell its two splits apart",
                        (ha + hd - hb - hc).abs()
                    );
                }
            }
        }
    }

    // Past the field on every side: nothing.
    for k in 0..200 {
        let t = k as f32 / 200.0;
        let (w, d) = (bounds.max.x - bounds.min.x, bounds.max.z - bounds.min.z);
        for (x, z) in [
            (bounds.min.x - 0.3, bounds.min.z + t * d),
            (bounds.max.x + 0.3, bounds.min.z + t * d),
            (bounds.min.x + t * w, bounds.min.z - 0.3),
            (bounds.min.x + t * w, bounds.max.z + 0.3),
        ] {
            assert!(physics.raycast_down(Vec3::new(x, top, z), reach).is_none(), "{kind:?}: ground past the field's edge at ({x}, {z})");
        }
    }

    // FROM UNDERNEATH: the ground is one-sided, as the mesh's back faces are.
    // A cave's ceiling is the cave's own rock; a ray inside it that met the
    // terrain's underside would put a floor over the player's head.
    for _ in 0..400 {
        let (x, z) = (bounds.min.x + next(&mut state) * (bounds.max.x - bounds.min.x), bounds.min.z + next(&mut state) * (bounds.max.z - bounds.min.z));
        let hit = physics.raycast(Vec3::new(x, low - 3.0, z), Vec3::Y, reach);
        assert!(hit.is_none(), "{kind:?}: a ray going up from under the ground at ({x}, {z}) met {hit:?}");
    }
    out
}

/// The four corner heights of a cell -- `a` at its own sample, `b` along x,
/// `c` along z, `d` opposite -- by the arithmetic the mesher places them with.
fn relief_grid(field: &Heightfield) -> impl Fn(u32, u32) -> [f32; 4] + '_ {
    let (raw, [nx, _], [lo, hi], base) = (field.raw_samples(), field.resolution(), field.height_range(), field.origin().y);
    move |ix, iz| {
        let h = |ix: u32, iz: u32| base + lo + raw[(iz * nx + ix) as usize] as f32 / u16::MAX as f32 * (hi - lo);
        [h(ix, iz), h(ix + 1, iz), h(ix, iz + 1), h(ix + 1, iz + 1)]
    }
}

#[test]
fn a_ray_lands_on_the_drawn_triangle_everywhere_on_a_ground_with_relief_holes_and_a_patch() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let field = relief_with_holes_and_a_patch();
    assert_eq!(field.meshed_cells(), [NX - 2, NZ - 1], "the fixture is meant to leave its last column of cells undrawn");

    // The reference against the collider it was cooked into, first.
    let mesh = sweep(&field, TerrainCollider::TriangleMesh, false);
    let grid = sweep(&field, TerrainCollider::HeightField, true);
    eprintln!("triangle mesh: {mesh:?}\nheight field:  {grid:?}");

    for swept in [&mesh, &grid] {
        assert!(swept.hits > 24_000, "too few rays met ground to mean anything: {swept:?}");
        assert!(swept.open > 600, "too few rays went through holes to mean anything: {swept:?}");
        assert!(swept.edge * 5 < swept.hits, "too many rays were on edges and not asked: {swept:?}");
        // Both diagonals, in number: a sweep of one of them proves half.
        assert!(swept.splits[0] > 300 && swept.splits[1] > 300, "the fixture does not use both diagonals: {swept:?}");
    }
    assert_eq!((mesh.hits, mesh.open, mesh.edge), (grid.hits, grid.open, grid.edge), "the two colliders were asked the same rays");
}

/// A hole mask coarser than the cells -- one of its cells is four by four of
/// the ground's -- opens the cells the mesher opens for it.
#[test]
fn a_hole_mask_coarser_than_the_cells_opens_the_same_cells() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut mask = HoleMask::solid([12, 10]);
    mask.set_open(3, 4, true);
    mask.set_open(11, 9, true);
    mask.set_open(0, 9, true);
    let field = relief([-6.0, 18.0]).with_holes(Some(mask));
    let grid = sweep(&field, TerrainCollider::HeightField, true);
    assert!(grid.open > 400 && grid.hits > 24_000, "{grid:?}");
}

/// A ground whose height range has no height -- every sample at one level --
/// is still a height field, at that level.
#[test]
fn a_level_ground_is_a_height_field_at_its_level() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let field = relief([3.0, 3.0]).with_holes(Some(cell_holes()));
    let grid = sweep(&field, TerrainCollider::HeightField, true);
    assert!(grid.hits > 24_000 && grid.open > 600, "{grid:?}");
}

/// A height range written upside down is not a height field PhysX can hold:
/// the ground is cooked as triangles, and is still the drawn ground.
#[test]
fn a_range_written_upside_down_falls_back_to_the_triangle_mesh() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let field = relief([18.0, -6.0]).with_holes(Some(cell_holes()));
    let mesh = sweep(&field, TerrainCollider::HeightField, false);
    assert!(mesh.hits > 24_000 && mesh.open > 600, "{mesh:?}");
}

/// Rolling ground a metre a cell -- slopes a box does not slide on -- with a
/// 3 m square opening in the middle of it.
fn rolling_ground_with_an_opening() -> Heightfield {
    let n = 33u32;
    let mut samples = Vec::new();
    for iz in 0..n {
        for ix in 0..n {
            let h = 0.5 + 0.1 * (ix as f32 * 0.2).sin() * (iz as f32 * 0.17).cos();
            samples.push((h * 65535.0) as u16);
        }
    }
    let mut mask = HoleMask::solid([n - 1, n - 1]);
    for iz in 14..17 {
        for ix in 14..17 {
            mask.set_open(ix, iz, true);
        }
    }
    Heightfield::new(samples, [n, n], [32.0, 32.0], [0.0, 8.0], Vec3::new(-16.0, -2.0, -16.0)).unwrap().with_holes(Some(mask))
}

/// BODIES, not only rays: a box dropped on the ground comes to rest on it at
/// the same height whichever collider holds it up, and one dropped over the
/// opening falls through it.
#[test]
fn a_body_rests_on_the_height_field_where_it_rests_on_the_mesh_and_falls_through_a_hole() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let field = rolling_ground_with_an_opening();
    // Over solid ground, four places; and over the middle of the opening.
    let solid = [(-9.3f32, -7.4f32), (6.6, -11.2), (10.2, 9.7), (-4.1, 12.3)];
    let hole = (-0.5f32, -0.5f32);
    const HALF: f32 = 0.2;

    let run = |kind: TerrainCollider| -> (Vec<f32>, f32) {
        let mut objects = String::new();
        for (i, (x, z)) in solid.iter().chain([&hole]).enumerate() {
            let y = field.height_at(*x, *z).unwrap() + 1.0;
            objects += &format!(
                r#"{}{{ "id": "box{i}", "cuboid": {{ "position": [{x}, {y}, {z}], "half_size": [{HALF}, {HALF}, {HALF}] }},
                     "rigid_body": {{ "mode": "Dynamic", "shape": "Box", "mass": 1.0 }} }}"#,
                if i == 0 { "" } else { "," }
            );
        }
        let mut scene: Scene = serde_json::from_str(&format!(r#"{{"name":"t","objects":[{objects}]}}"#)).unwrap();
        let mut physics = PhysicsWorld::new();
        physics.set_terrain_collider(kind);
        physics.rebuild(&scene, Path::new("."));
        physics.spawn_scene_terrain(&field);
        assert_eq!(physics.terrain_height_fields.len(), usize::from(kind == TerrainCollider::HeightField));
        physics.note_floor(field.lowest_height());
        let rigs = HashMap::new();
        let mut lowest_over_hole = f32::MAX;
        for _ in 0..180 {
            physics.step(1.0 / 60.0, &mut scene, &rigs);
            lowest_over_hole = lowest_over_hole.min(scene.find_object("box4").unwrap().cuboid.position.y);
        }
        ((0..4).map(|i| scene.find_object(&format!("box{i}")).unwrap().cuboid.position.y).collect(), lowest_over_hole)
    };
    let (on_mesh, through_mesh) = run(TerrainCollider::TriangleMesh);
    let (on_grid, through_grid) = run(TerrainCollider::HeightField);
    eprintln!("resting heights on the mesh {on_mesh:?}, on the height field {on_grid:?}");

    let ground_at_hole = field.height_at(hole.0, hole.1).unwrap();
    assert!(through_mesh < ground_at_hole - 1.0, "mesh: the box over the opening got no lower than {through_mesh}; the ground round it is at {ground_at_hole}");
    assert!(through_grid < ground_at_hole - 1.0, "height field: the box over the opening got no lower than {through_grid}; the ground round it is at {ground_at_hole}");
    for i in 0..4 {
        let ground = field.height_at(solid[i].0, solid[i].1).unwrap();
        assert!(
            (on_grid[i] - (ground + HALF)).abs() < 0.08,
            "height field: box {i} came to rest at {}, over ground at {ground}",
            on_grid[i]
        );
        assert!(
            (on_grid[i] - on_mesh[i]).abs() < 0.005,
            "box {i} rests at {} on the height field and at {} on the mesh",
            on_grid[i],
            on_mesh[i]
        );
    }
}

/// A REAL LEVEL, both ways: the shipped test_room's ground (`GAME_DIR`, or the
/// game beside this crate) with its cave's holes and the ground its bake put
/// back, built as the triangle mesh it was and then as the height field, and
/// asked the same rays. Each has to meet the same thing: nothing, or the same
/// point to a millimetre on a face turned the same way.
///
/// The rays: straight down on a jittered lattice over the whole field; and,
/// for the patch -- where the two colliders are least alike -- one at every
/// triangle of it, HEAD ON, from 30 cm off its face. Head on because much of
/// a cave mouth's ground is close to upright: a ray straight down on such a
/// face grazes it, and whether a grazing ray meets a sliver or slips past is
/// decided by the last bit of the arithmetic, differently in any two cooked
/// meshes of the same triangles (3 of 31,503 such rays on the 625 stress
/// level, each a face the other collider also had).
///
/// Even head on, a ray or two in ten thousand passes within a hundredth of
/// a millimetre of the EDGE of a face that stands in front of the one it was
/// aimed at -- at the join of the grid and the patch the ground steps by a
/// few millimetres, in what is drawn as in both colliders -- and is given
/// the nearer face by one collider and the one behind it by the other.
/// Those are counted, held to a ray in five thousand and to two centimetres,
/// and printed; a hit against a miss, or anything further apart, fails.
/// (Measured beside this test: a ray straight down at a hole's rim is met up
/// to 3 micrometres past the rim by the mesh and not at all past it by the
/// height field. Neither is padded.)
///
/// Skips where there is no such level.
#[test]
fn a_real_levels_ground_meets_every_ray_where_its_triangle_mesh_did() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let game = std::env::var("GAME_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../game").to_string());
    let game = Path::new(&game);
    let name = std::env::var("SCENE").unwrap_or_else(|_| "test_room".into());
    let Ok(mut scene) = Scene::load(&crate::manifest::Manifest::scene_path(game, &name)) else {
        eprintln!("skipped: no {name}");
        return;
    };
    let Some(def) = scene.terrain.clone() else {
        eprintln!("skipped: {name} has no terrain");
        return;
    };
    let Ok(ground) = crate::terrain::load(&def, game) else {
        eprintln!("skipped: {name}'s terrain does not load");
        return;
    };
    // The ground alone: nothing else to be met.
    scene.objects.clear();
    scene.water.clear();

    let b = ground.bounds();
    let (top, reach) = (b.max.y + 5.0, b.max.y - b.min.y + 60.0);
    // Each ray: where from, which way, how far.
    let mut rays: Vec<(Vec3, Vec3, f32)> = Vec::new();
    let mut state = 0xca7e_u32;
    const SIDE: u32 = 300;
    for k in 0..SIDE * SIDE {
        let (u, v) = (((k % SIDE) as f32 + next(&mut state)) / SIDE as f32, ((k / SIDE) as f32 + next(&mut state)) / SIDE as f32);
        rays.push((Vec3::new(b.min.x + u * (b.max.x - b.min.x), top, b.min.z + v * (b.max.z - b.min.z)), Vec3::NEG_Y, reach));
    }
    let lattice = rays.len();
    if let Some(extra) = ground.as_heightfield().and_then(|f| f.ground_patch()) {
        // No more than some tens of thousands, however many caves there are.
        let every = (extra.indices.len() / 3 / 40_000).max(1);
        for tri in extra.triangles_in(b).step_by(every) {
            let c = [0, 1, 2].map(|k| extra.positions[tri[k] as usize]);
            let Some(n) = (c[1] - c[0]).cross(c[2] - c[0]).try_normalize() else { continue };
            // Well inside it: a fifth of the way in from every edge at least.
            let (u, v) = (0.2 + 0.4 * next(&mut state), 0.2 + 0.4 * next(&mut state));
            let (u, v) = if u + v > 0.8 { (0.6 - u * 0.5, 0.6 - v * 0.5) } else { (u, v) };
            let p = c[0] + (c[1] - c[0]) * u + (c[2] - c[0]) * v;
            rays.push((p + n * 0.3, -n, 0.6));
        }
    }

    let ask = |kind: TerrainCollider| -> Vec<Option<(Vec3, Vec3)>> {
        let mut physics = PhysicsWorld::new();
        physics.set_terrain_collider(kind);
        physics.rebuild(&scene, game);
        assert_eq!(physics.terrain_height_fields.len(), usize::from(kind == TerrainCollider::HeightField));
        rays.iter().map(|&(from, dir, reach)| physics.raycast(from, dir, reach)).collect()
    };
    let on_mesh = ask(TerrainCollider::TriangleMesh);
    let on_grid = ask(TerrainCollider::HeightField);

    // `shared`: the same point on another face. A ray that arrives exactly on
    // an edge two faces share -- the patch's vertices sit on the grid's own
    // lines, so some of the lattice's do -- is given either face, and either
    // is right; a cell split the wrong way would move the point across the
    // whole cell instead.
    // `past_an_edge`: the nearer of two faces by one collider and the one
    // behind it by the other, no more than two centimetres apart.
    let (mut met, mut open, mut shared, mut past_an_edge, mut furthest, mut differ) = (0usize, 0usize, 0usize, 0usize, 0f32, Vec::new());
    let mut patch_faces_met = 0usize;
    for (i, (mesh, grid)) in on_mesh.iter().zip(&on_grid).enumerate() {
        match (mesh, grid) {
            (None, None) => open += 1,
            (Some((m, mn)), Some((g, gn))) if m.distance(*g) < 1e-3 => {
                if mn.dot(*gn) > 0.9995 {
                    met += 1;
                    patch_faces_met += usize::from(i >= lattice);
                } else {
                    shared += 1;
                }
            }
            (Some((m, _)), Some((g, _))) if m.distance(*g) < 0.02 => {
                past_an_edge += 1;
                furthest = furthest.max(m.distance(*g));
            }
            _ => differ.push((rays[i], *mesh, *grid)),
        }
    }
    eprintln!(
        "{name}: {} rays ({lattice} down on a lattice, {} head on at the patch's triangles): {met} met the same face at the same point, {shared} the same point on an edge two faces share, {past_an_edge} one of two faces a ray passed the edge of (up to {:.1} mm apart), {open} met nothing on either, {} differ",
        rays.len(),
        rays.len() - lattice,
        furthest * 1000.0,
        differ.len()
    );
    assert!(met > lattice / 2, "too few rays met ground to mean anything");
    assert!(patch_faces_met * 10 >= (rays.len() - lattice) * 9, "too few of the patch's triangles were met to mean anything: {patch_faces_met}");
    assert!(differ.is_empty(), "the height field and the triangle mesh disagree at {} rays; the first: {:?}", differ.len(), &differ[..differ.len().min(5)]);
    assert!(shared * 2000 <= rays.len(), "{shared} rays met another face at the same point: more than edges account for");
    assert!(past_an_edge * 5000 <= rays.len(), "{past_an_edge} rays met a different face nearby: more than edges account for");
}

/// The caves' ground is cooked in batches of whole connected pieces: every
/// triangle is in one batch, on the vertices it had, and no piece is split.
#[test]
fn the_caves_ground_is_batched_in_whole_connected_pieces() {
    use crate::rigid_physics_spawn::ground_patch_batches;

    // Five strips of ground, each its own piece: 2 x 1500 triangles on a
    // ribbon of 1501 x 2 vertices, three metres apart in z. The third is
    // stored in two parts with two other strips between them, as a file may.
    let ribbon = |piece: u32, from: u32, to: u32, base: u32, positions: &mut Vec<Vec3>, indices: &mut Vec<u32>| {
        for i in from..=to {
            for side in 0..2u32 {
                positions.push(Vec3::new(i as f32 * 0.1, piece as f32, piece as f32 * 3.0 + side as f32));
            }
        }
        for i in 0..to - from {
            let a = base + i * 2;
            indices.extend([a, a + 1, a + 2, a + 2, a + 1, a + 3]);
        }
    };
    let (mut positions, mut indices) = (Vec::new(), Vec::new());
    let mut starts = Vec::new();
    for piece in 0..5u32 {
        starts.push(positions.len() as u32);
        ribbon(piece, 0, 1500, positions.len() as u32, &mut positions, &mut indices);
    }
    // Piece 2 again, further along, joined to its first half by one triangle
    // that reaches back to it.
    let more = positions.len() as u32;
    ribbon(2, 1501, 2000, more, &mut positions, &mut indices);
    indices.extend([starts[2] + 1500 * 2, starts[2] + 1500 * 2 + 1, more]);
    let normals = vec![Vec3::Y; positions.len()];
    let patch = GroundPatch { positions, normals, indices };
    let everywhere = crate::physics::Aabb { min: Vec3::splat(-1e6), max: Vec3::splat(1e6) };

    let batches = ground_patch_batches(&patch, everywhere);
    // 3000 triangles a piece, and 3999 for the third (its second half is 499
    // steps, and the one triangle that joins them): closed past 8192.
    assert_eq!(batches.iter().map(|(_, i)| i.len() / 3).collect::<Vec<_>>(), vec![3000 + 3000 + 3999, 3000 + 3000]);
    assert_eq!(batches.iter().map(|(_, i)| i.len()).sum::<usize>(), patch.indices.len());
    let mut seen: Vec<Vec<u32>> = Vec::new();
    for (points, indices) in &batches {
        assert!(indices.iter().all(|i| (*i as usize) < points.len()));
        // A piece is known by its height here; a batch holds whole pieces.
        let mut pieces: Vec<u32> = points.iter().map(|p| p.y() as u32).collect();
        pieces.sort_unstable();
        pieces.dedup();
        seen.push(pieces);
        // No vertex twice: each is the patch's own, once.
        let mut at: Vec<(u32, u32, u32)> = points.iter().map(|p| (p.x().to_bits(), p.y().to_bits(), p.z().to_bits())).collect();
        at.sort_unstable();
        at.dedup();
        assert_eq!(at.len(), points.len());
    }
    assert_eq!(seen, vec![vec![0, 1, 2], vec![3, 4]], "a piece was split between two meshes");
    // Every triangle, with its corners where they were.
    let mut before: Vec<[(u32, u32, u32); 3]> =
        patch.indices.chunks_exact(3).map(|t| [0, 1, 2].map(|k| { let p = patch.positions[t[k] as usize]; (p.x.to_bits(), p.y.to_bits(), p.z.to_bits()) })).collect();
    let mut after: Vec<[(u32, u32, u32); 3]> = batches
        .iter()
        .flat_map(|(points, indices)| indices.chunks_exact(3).map(move |t| [0, 1, 2].map(|k| { let p = &points[t[k] as usize]; (p.x().to_bits(), p.y().to_bits(), p.z().to_bits()) })))
        .collect();
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(before, after);
}

/// Megabytes this process holds, as `ps` reports them.
fn rss_mb() -> f32 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().expect("ps");
    String::from_utf8_lossy(&out.stdout).trim().parse::<f32>().unwrap_or(0.0) / 1024.0
}

/// WHAT A LEVEL'S PHYSICS COSTS TO BUILD, printed: the time `rebuild` takes
/// and the memory the process holds before and after it, then a hundred rays
/// straight down at the ground and a hundred long ones along it.
///
/// `GAME_DIR=<game> SCENE=<name> cargo test --release -- --ignored --nocapture print_what_the_physics_rebuild_costs`
///
/// `SS_TERRAIN_COLLIDER=mesh` builds the ground as the triangle mesh it was;
/// `NO_TERRAIN=1` leaves the ground out, for what everything else costs, and
/// `NO_WATER=1` the water, whose footprints are measured against the ground
/// and so come and go with it. One run a process: the memory a second run
/// reports is the first one's too.
#[test]
#[ignore = "a measurement: needs GAME_DIR and SCENE"]
fn print_what_the_physics_rebuild_costs() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(game) = std::env::var("GAME_DIR") else {
        eprintln!("skipped: no GAME_DIR");
        return;
    };
    let game = Path::new(&game);
    let name = std::env::var("SCENE").unwrap_or_else(|_| "test_room".into());
    let mut scene = Scene::load(&crate::manifest::Manifest::scene_path(game, &name)).expect("the scene loads");
    scene.resolve_world_transforms();
    if std::env::var("NO_TERRAIN").is_ok() {
        scene.terrain = None;
    }
    if std::env::var("NO_WATER").is_ok() {
        scene.water.clear();
    }
    let samples = scene.terrain.as_ref().map(|t| match &t.kind {
        crate::terrain::TerrainKind::Heightfield { resolution, .. } => *resolution,
    });

    let mut physics = PhysicsWorld::new();
    let before = rss_mb();
    let started = std::time::Instant::now();
    physics.rebuild(&scene, game);
    let rebuild_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = rss_mb();

    // The rays: over the ground's own extent, measured after the memory is.
    let mut rays = String::new();
    if let Some(def) = &scene.terrain {
        let ground = crate::terrain::load(def, game).expect("the terrain loads");
        let b = ground.bounds();
        let (w, d) = (b.max.x - b.min.x, b.max.z - b.min.z);
        let reach = b.max.y - b.min.y + 20.0;
        let lattice: Vec<(f32, f32)> = (0..100).map(|k| (b.min.x + ((k % 10) as f32 + 0.37) / 10.0 * w, b.min.z + ((k / 10) as f32 + 0.61) / 10.0 * d)).collect();
        const ROUNDS: u32 = 200;
        let (mut down_hits, mut long_hits, mut over_hits) = (0u32, 0u32, 0u32);
        let started = std::time::Instant::now();
        for _ in 0..ROUNDS {
            for &(x, z) in &lattice {
                down_hits += u32::from(physics.raycast_down(Vec3::new(x, b.max.y + 10.0, z), reach).is_some());
            }
        }
        let down_ms = started.elapsed().as_secs_f64() * 1000.0 / ROUNDS as f64;
        // Which of them met nothing, as a map: x across, z down.
        let mut missed = String::new();
        if down_hits < 100 * ROUNDS {
            missed = "\n  rays down that met no ground (x across, z down):".to_string();
            for row in lattice.chunks(10) {
                missed += "\n    ";
                missed.extend(row.iter().map(|&(x, z)| if physics.raycast_down(Vec3::new(x, b.max.y + 10.0, z), reach).is_some() { '#' } else { '.' }));
            }
        }
        // Along the ground, a metre and a half over it, the length of the
        // level: a shot, or a line of sight.
        let along = Vec3::new(1.0, -0.004, 0.31).normalize();
        let started = std::time::Instant::now();
        for _ in 0..ROUNDS {
            for &(_, z) in &lattice {
                let x = b.min.x + 0.5;
                let y = ground.height_at(x, z).unwrap_or(b.max.y) + 1.5;
                long_hits += u32::from(physics.raycast(Vec3::new(x, y, z), along, w * 1.5).is_some());
            }
        }
        let long_ms = started.elapsed().as_secs_f64() * 1000.0 / ROUNDS as f64;
        // Across the whole level, level, at nine tenths of the ground's
        // height range: over nearly all of it, so a height field walks every
        // cell under the ray where a mesh's tree steps over them.
        let across = Vec3::new(1.0, 0.0, 0.31).normalize();
        let high = b.min.y + (b.max.y - b.min.y) * 0.9;
        let started = std::time::Instant::now();
        for _ in 0..ROUNDS {
            for &(_, z) in &lattice {
                over_hits += u32::from(physics.raycast(Vec3::new(b.min.x + 0.5, high, z), across, w * 1.5).is_some());
            }
        }
        let over_ms = started.elapsed().as_secs_f64() * 1000.0 / ROUNDS as f64;
        rays = format!(
            "; 100 rays down {down_ms:.4} ms ({} hit), 100 rays along the ground {long_ms:.4} ms ({} hit), 100 rays across the level over it {over_ms:.4} ms ({} hit){missed}",
            down_hits / ROUNDS,
            long_hits / ROUNDS,
            over_hits / ROUNDS
        );
    }
    println!(
        "physics rebuild: scene {name} terrain {samples:?} as {:?}: {rebuild_ms:.1} ms; process {before:.0} MB before, {after:.0} MB after (+{:.0} MB){rays}",
        physics.terrain_collider(),
        after - before,
    );
}
