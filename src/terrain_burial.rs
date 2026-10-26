//! TERRAIN UNDER STRUCTURES: the drawn ground's treatment where a building's
//! floor covers it -- lowered out of the floor's depth, or not drawn at all.
//! The headset's terrain (`space_soup_app::terrain_render`) and the studio's
//! both use it, so the two draw the same ground under the same rooms.
//! Physics never sees it: the terrain stays the ground you collide with.

use glam::Vec3;

use crate::brush::{contains_point, BrushSolid};

/// How far below the terrain surface to look for a structure that buries it.
///
/// BELOW, not above. `BrushDef::evaluate` yields a building's SOLID pieces --
/// slab, walls, ceiling -- and not the carved room, so a point just above the
/// ground inside a room is open air and would find nothing. Five centimetres
/// down lands inside a floor slab resting on the ground, and inside nothing at
/// all under a structure raised off it, like a bridge.
pub const BURIED_PROBE_DEPTH: f32 = 0.05;

/// How far a buried terrain vertex is lowered, in the RENDERED copy only.
///
/// A floor laid on flattened ground is coplanar with it: `test_room` measures
/// the terrain 0.1 mm under the room floor, well inside depth precision, so the
/// two z-fight and grass flickers through the floor. It only showed with SSR
/// off, because the SSR eye pass redraws every brush over the finished scene
/// and paints the fight over.
///
/// Physics is untouched: the terrain stays the ground you collide with.
pub const BURIED_DROP: f32 = 0.02;

/// How finely a candidate triangle is sampled for coverage: a barycentric grid
/// of `(n + 1)(n + 2) / 2` points (28 at 6). At the 0.86 m terrain spacing that
/// is a sample every ~14 cm, finer than any wall.
pub const COVERAGE_SAMPLES: u32 = 6;

/// What `bury_under_structures` changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Burial {
    pub hidden_triangles: usize,
    pub lowered_vertices: usize,
}

/// Render-only treatment of terrain covered by structure floors.
///
/// A triangle whose three vertices are ALL buried, and which is covered across
/// its whole area, is dropped: drawing it is pure wasted fill and the source of
/// the z-fight. (Three buried corners alone are not enough at an inside corner;
/// see the note in the body.)
/// A triangle that crosses a wall line cannot be dropped without opening a gap
/// outside the wall, so it is kept and only its buried vertices are lowered by
/// `BURIED_DROP` -- enough to put the part under the room well below the floor,
/// while the dip outside is hidden by the wall's foot below ground.
pub fn bury_under_structures(
    positions: &mut [Vec3],
    indices: &[u32],
    solids: &[BrushSolid],
) -> (Vec<u32>, Burial) {
    let (kept, burial, _) = bury_indexed(positions, indices, &SolidIndex::new(solids));
    (kept, burial)
}

/// THE LEVEL'S BRUSH SOLIDS BY WHERE THEY STAND, so "is this point inside a
/// solid" asks the few solids over that spot and not every solid of the
/// level.
///
/// `bury_under_structures` asked every ground vertex against every solid:
/// samples x solids, both of which grow with the level's area -- 13 ms on
/// `test_room`, 210 ms at 625 samples a side, 2.5 s at 1,025, 39 s at 2,049
/// (`docs/scale-audit-2026-10-13.md`, L1). Nearly all of the ground is under
/// no building at all.
///
/// A grid over the solids' footprints (x, z), each cell listing the solids
/// whose box reaches it. A point outside the grid, or in an empty cell, is
/// answered without a plane test; in a cell, by the solids listed there whose
/// box holds it. THE SAME ANSWER as asking them all (a solid can only hold a
/// point its own box holds; the boxes are widened by more than the test's
/// tolerance; a solid with no box -- unbounded, degenerate -- is asked every
/// time, as before): `the_indexed_burial_is_the_burial_asked_of_every_solid`.
///
/// WHAT IT COSTS NOW: the ground's vertices, once each, plus the plane tests
/// under buildings -- linear in the level's samples.
pub struct SolidIndex<'a> {
    solids: &'a [BrushSolid],
    /// Each solid's box, widened; `None` for one without.
    boxes: Vec<Option<([f64; 3], [f64; 3])>>,
    /// Solids with no box: asked for every point.
    boxless: Vec<u32>,
    origin: [f64; 2],
    cell: f64,
    cells_x: usize,
    cells_z: usize,
    cells: Vec<Vec<u32>>,
    /// Plane-set tests made (`contains_point` calls): what the index is for.
    tests: std::cell::Cell<u64>,
}

impl<'a> SolidIndex<'a> {
    /// Wider than `contains_point`'s tolerance here (1e-4), so no solid is
    /// missed at its own surface.
    const SLACK: f64 = 1e-3;
    /// Past this a face's extent is the CSG's seed polygon showing through
    /// (`brush::HUGE`, 1e5, over root three for a tilted face): the solid is
    /// open that way and is asked for every point instead.
    const BOUNDED: f64 = 2.5e4;

    pub fn new(solids: &'a [BrushSolid]) -> Self {
        let boxes: Vec<Option<([f64; 3], [f64; 3])>> = solids
            .iter()
            .map(|s| {
                // A solid open on some side has faces that run to the size
                // the CSG seeds a face at (100 km): its "box" is not one.
                crate::brush::solid_bounds(s)
                    .filter(|(lo, hi)| lo.iter().chain(hi.iter()).all(|c| c.is_finite() && c.abs() < Self::BOUNDED))
                    .map(|(lo, hi)| (lo.map(|c| c - Self::SLACK), hi.map(|c| c + Self::SLACK)))
            })
            .collect();
        let boxless: Vec<u32> = boxes.iter().enumerate().filter(|(_, b)| b.is_none()).map(|(i, _)| i as u32).collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for (l, h) in boxes.iter().flatten() {
            lo = [lo[0].min(l[0]), lo[1].min(l[2])];
            hi = [hi[0].max(h[0]), hi[1].max(h[2])];
        }
        let boxed = boxes.iter().flatten().count();
        if boxed == 0 {
            return Self { solids, boxes, boxless, origin: [0.0; 2], cell: 1.0, cells_x: 0, cells_z: 0, cells: Vec::new(), tests: std::cell::Cell::default() };
        }
        // About as many cells as solids, and no cell under 4 m: a building's
        // solids share a few cells, an open level's cells are mostly empty.
        let side = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let per_axis = ((boxed as f64).sqrt().ceil() as usize * 2).clamp(1, 512);
        let cell = (side / per_axis as f64).max(4.0);
        let cells_x = (((hi[0] - lo[0]) / cell).floor() as usize + 1).max(1);
        let cells_z = (((hi[1] - lo[1]) / cell).floor() as usize + 1).max(1);
        let mut cells = vec![Vec::new(); cells_x * cells_z];
        for (i, b) in boxes.iter().enumerate() {
            let Some((l, h)) = b else { continue };
            let at = |v: f64, o: f64, n: usize| (((v - o) / cell).floor().max(0.0) as usize).min(n - 1);
            for cz in at(l[2], lo[1], cells_z)..=at(h[2], lo[1], cells_z) {
                for cx in at(l[0], lo[0], cells_x)..=at(h[0], lo[0], cells_x) {
                    cells[cz * cells_x + cx].push(i as u32);
                }
            }
        }
        Self { solids, boxes, boxless, origin: lo, cell, cells_x, cells_z, cells, tests: std::cell::Cell::default() }
    }

    /// Whether any solid holds `p`, within `tolerance`.
    pub fn holds(&self, p: [f64; 3], tolerance: f64) -> bool {
        let inside = |i: u32| {
            self.tests.set(self.tests.get() + 1);
            contains_point(&self.solids[i as usize], p, tolerance)
        };
        if self.boxless.iter().any(|&i| inside(i)) {
            return true;
        }
        let (fx, fz) = ((p[0] - self.origin[0]) / self.cell, (p[2] - self.origin[1]) / self.cell);
        if !(fx >= 0.0 && fz >= 0.0) || fx as usize >= self.cells_x || fz as usize >= self.cells_z {
            return false;
        }
        self.cells[fz as usize * self.cells_x + fx as usize].iter().any(|&i| {
            let (lo, hi) = self.boxes[i as usize].expect("only boxed solids are listed");
            (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k]) && inside(i)
        })
    }

    /// Solids asked for every point, and solid entries listed in the grid's
    /// cells: how the index laid the solids out.
    pub fn layout(&self) -> (usize, usize) {
        (self.boxless.len(), self.cells.iter().map(Vec::len).sum())
    }

    /// Plane-set tests made so far.
    pub fn tests(&self) -> u64 {
        self.tests.get()
    }
}

/// [`bury_under_structures`] through an index of the solids; also how many
/// plane-set tests it took.
pub fn bury_indexed(positions: &mut [Vec3], indices: &[u32], solids: &SolidIndex) -> (Vec<u32>, Burial, u64) {
    let before = solids.tests();
    let covered = |q: Vec3| solids.holds([f64::from(q.x), f64::from(q.y - BURIED_PROBE_DEPTH), f64::from(q.z)], 1e-4);
    let buried: Vec<bool> = positions.iter().map(|p| covered(*p)).collect();
    // COVERED ACROSS ITS WHOLE AREA, not just at its corners. Three buried
    // corners prove a triangle covered only under a CONVEX footprint. Where the
    // hallway meets a hall the footprint is an L, and a triangle with a corner
    // under each building spans the open ground in the inside corner between
    // them: hiding it opened a hole in the grass (headset, 2026-09-24). So a
    // triangle whose corners are all buried is also sampled across its area,
    // and kept -- lowered, like any triangle crossing a wall -- if any sample
    // is open ground. Only triangles already buried at every corner pay for it.
    let fully_covered = |a: Vec3, b: Vec3, c: Vec3| {
        (0..=COVERAGE_SAMPLES).all(|i| {
            (0..=(COVERAGE_SAMPLES - i)).all(|j| {
                let (u, v) = (i as f32 / COVERAGE_SAMPLES as f32, j as f32 / COVERAGE_SAMPLES as f32);
                covered(a + (b - a) * u + (c - a) * v)
            })
        })
    };
    let mut kept = Vec::with_capacity(indices.len());
    let mut burial = Burial::default();
    for tri in indices.as_chunks::<3>().0 {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| positions[i as usize]);
        if tri.iter().all(|&i| buried[i as usize]) && fully_covered(a, b, c) {
            burial.hidden_triangles += 1;
        } else {
            kept.extend_from_slice(tri);
        }
    }
    for (p, b) in positions.iter_mut().zip(&buried) {
        if *b {
            p.y -= BURIED_DROP;
            burial.lowered_vertices += 1;
        }
    }
    (kept, burial, solids.tests() - before)
}

/// Only the vertex half of [`bury_under_structures`]: every terrain vertex
/// under a structure lowered by `BURIED_DROP`, no triangle dropped -- for a
/// drawn copy whose triangles are listed elsewhere (a LOD's own index
/// lists). Returns how many were lowered.
pub fn lower_buried(positions: &mut [Vec3], solids: &SolidIndex) -> usize {
    let mut lowered = 0;
    for p in positions.iter_mut() {
        if solids.holds([f64::from(p.x), f64::from(p.y - BURIED_PROBE_DEPTH), f64::from(p.z)], 1e-4) {
            p.y -= BURIED_DROP;
            lowered += 1;
        }
    }
    lowered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::block_solid;

    #[test]
    fn lowering_alone_moves_the_same_vertices_the_burial_does() {
        let n = 12i32;
        let mut grid: Vec<Vec3> = (0..n * n).map(|i| Vec3::new((i % n - n / 2) as f32, 0.0, (i / n - n / 2) as f32)).collect();
        let indices: Vec<u32> = (0..n - 1)
            .flat_map(|z| (0..n - 1).flat_map(move |x| {
                let a = (z * n + x) as u32;
                [a, a + n as u32, a + 1, a + 1, a + n as u32, a + n as u32 + 1]
            }))
            .collect();
        let slab = block_solid([-3.0, -0.2, -3.0], [3.0, 0.05, 3.0], "floor");
        let mut by_burial = grid.clone();
        let (_, burial) = bury_under_structures(&mut by_burial, &indices, std::slice::from_ref(&slab));
        let lowered = lower_buried(&mut grid, &SolidIndex::new(std::slice::from_ref(&slab)));
        assert!(lowered > 0);
        assert_eq!(lowered, burial.lowered_vertices);
        assert_eq!(grid, by_burial);
    }
}
