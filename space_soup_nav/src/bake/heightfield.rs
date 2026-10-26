//! Voxelisation: triangles to a column-per-cell heightfield of solid spans, then the three span
//! filters. A port of Recast's `RecastRasterization.cpp`, `rcMarkWalkableTriangles` and
//! `RecastFilter.cpp` (zlib licence, Copyright (c) 2009-2010 Mikko Mononen; altered: written in
//! Rust, spans kept in a sorted `Vec` per column instead of pooled linked lists).
//!
//! Float operations are written in the same order as the C++ so the same spans come out; that
//! is what `tests/parity.rs` checks against the golden files.

use super::{Config, NULL_AREA, WALKABLE_AREA};

/// Spans' heights are stored in 13 bits in Recast; kept as the same limit.
pub const SPAN_MAX_HEIGHT: i32 = (1 << 13) - 1;
/// What Recast treats as "no ceiling" above the top span of a column.
const MAX_HEIGHTFIELD_HEIGHT: i32 = 0xffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Bottom and top of the solid, in cell heights from `bmin.y` (top exclusive).
    pub smin: u16,
    pub smax: u16,
    pub area: u8,
}

pub struct Heightfield {
    pub width: i32,
    pub height: i32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    /// One column per cell, `x + z * width`, spans sorted bottom to top and never overlapping.
    pub columns: Vec<Vec<Span>>,
}

pub const DIR_X: [i32; 4] = [-1, 0, 1, 0];
pub const DIR_Z: [i32; 4] = [0, 1, 0, -1];

impl Heightfield {
    pub fn new(cfg: &Config) -> Heightfield {
        Heightfield {
            width: cfg.width,
            height: cfg.height,
            bmin: cfg.bmin,
            bmax: cfg.bmax,
            cs: cfg.cs,
            ch: cfg.ch,
            columns: vec![Vec::new(); (cfg.width * cfg.height) as usize],
        }
    }

    pub fn span_count(&self) -> usize {
        self.columns.iter().map(Vec::len).sum()
    }

    pub fn walkable_span_count(&self) -> usize {
        self.columns.iter().flatten().filter(|s| s.area != NULL_AREA).count()
    }

    /// Recast's `addSpan`: inserts, merging with every span it overlaps or touches. Where the
    /// merged tops are within `merge_threshold` the higher area id wins.
    pub fn add_span(&mut self, x: i32, z: i32, smin: u16, smax: u16, area: u8, merge_threshold: i32) {
        let column = &mut self.columns[(x + z * self.width) as usize];
        let mut new = Span { smin, smax, area };
        let mut i = 0;
        while i < column.len() {
            let cur = column[i];
            if cur.smin > new.smax {
                break;
            }
            if cur.smax < new.smin {
                i += 1;
                continue;
            }
            new.smin = new.smin.min(cur.smin);
            new.smax = new.smax.max(cur.smax);
            if (new.smax as i32 - cur.smax as i32).abs() <= merge_threshold {
                new.area = new.area.max(cur.area);
            }
            column.remove(i);
        }
        column.insert(i, new);
    }
}

/// `rcMarkWalkableTriangles`: a triangle is walkable when its normal is within the slope of up.
/// Winding matters: the normal is `(b - a) x (c - a)`.
pub fn mark_walkable_triangles(slope_deg: f32, verts: &[[f32; 3]], tris: &[[u32; 3]]) -> Vec<u8> {
    let threshold = libm::cosf(slope_deg / 180.0 * std::f32::consts::PI);
    tris.iter()
        .map(|t| {
            let (v0, v1, v2) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
            let e0 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
            let e1 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
            let n = [e0[1] * e1[2] - e0[2] * e1[1], e0[2] * e1[0] - e0[0] * e1[2], e0[0] * e1[1] - e0[1] * e1[0]];
            let d = 1.0 / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if n[1] * d > threshold { WALKABLE_AREA } else { NULL_AREA }
        })
        .collect()
}

/// `dividePoly`: splits a convex polygon (at most 12 vertices) by the plane `v[axis] = offset`.
/// Returns the counts written to `out1` (the side below `offset`) and `out2` (above).
fn divide_poly(input: &[[f32; 3]], out_near: &mut [[f32; 3]; 12], out_far: &mut [[f32; 3]; 12], offset: f32, axis: usize) -> (usize, usize) {
    let n = input.len();
    if n == 0 {
        return (0, 0);
    }
    let mut delta = [0.0f32; 12];
    for (d, v) in delta.iter_mut().zip(input) {
        *d = offset - v[axis];
    }
    let (mut m, mut k) = (0usize, 0usize);
    let mut b = n - 1;
    for a in 0..n {
        let same_side = (delta[a] >= 0.0) == (delta[b] >= 0.0);
        if !same_side {
            let s = delta[b] / (delta[b] - delta[a]);
            let (va, vb) = (input[a], input[b]);
            let p = [vb[0] + (va[0] - vb[0]) * s, vb[1] + (va[1] - vb[1]) * s, vb[2] + (va[2] - vb[2]) * s];
            out_near[m] = p;
            out_far[k] = p;
            m += 1;
            k += 1;
            if delta[a] > 0.0 {
                out_near[m] = input[a];
                m += 1;
            } else if delta[a] < 0.0 {
                out_far[k] = input[a];
                k += 1;
            }
        } else {
            let mut to_second = true;
            if delta[a] >= 0.0 {
                out_near[m] = input[a];
                m += 1;
                to_second = delta[a] == 0.0;
            }
            if to_second {
                out_far[k] = input[a];
                k += 1;
            }
        }
        b = a;
    }
    (m, k)
}

/// `rasterizeTri`: clips the triangle into every cell it touches and adds the span it covers.
fn rasterize_triangle(hf: &mut Heightfield, v: [[f32; 3]; 3], area: u8, ics: f32, ich: f32, merge_threshold: i32) {
    let (bmin, bmax) = (hf.bmin, hf.bmax);
    let mut tmin = v[0];
    let mut tmax = v[0];
    for p in &v[1..] {
        for i in 0..3 {
            tmin[i] = tmin[i].min(p[i]);
            tmax[i] = tmax[i].max(p[i]);
        }
    }
    let overlaps = (0..3).all(|i| tmin[i] <= bmax[i] && tmax[i] >= bmin[i]);
    if !overlaps {
        return;
    }
    let (w, h) = (hf.width, hf.height);
    let by = bmax[1] - bmin[1];
    let z0 = (((tmin[2] - bmin[2]) * ics) as i32).clamp(-1, h - 1);
    let z1 = (((tmax[2] - bmin[2]) * ics) as i32).clamp(0, h - 1);

    let mut input = [[0.0f32; 3]; 12];
    input[..3].copy_from_slice(&v);
    let mut n_in = 3;
    let mut row = [[0.0f32; 3]; 12];
    let mut rest = [[0.0f32; 3]; 12];
    let mut cell = [[0.0f32; 3]; 12];
    let mut row_rest = [[0.0f32; 3]; 12];
    for z in z0..=z1 {
        let cz = bmin[2] + z as f32 * hf.cs;
        let (n_row, n_rest) = divide_poly(&input[..n_in], &mut row, &mut rest, cz + hf.cs, 2);
        std::mem::swap(&mut input, &mut rest);
        n_in = n_rest;
        if n_row < 3 || z < 0 {
            continue;
        }
        let mut min_x = row[0][0];
        let mut max_x = row[0][0];
        for p in &row[1..n_row] {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
        }
        let x0 = ((min_x - bmin[0]) * ics) as i32;
        let x1 = ((max_x - bmin[0]) * ics) as i32;
        if x1 < 0 || x0 >= w {
            continue;
        }
        let x0 = x0.clamp(-1, w - 1);
        let x1 = x1.clamp(0, w - 1);
        let mut n_row = n_row;
        for x in x0..=x1 {
            let cx = bmin[0] + x as f32 * hf.cs;
            let (n_cell, n_row_rest) = divide_poly(&row[..n_row], &mut cell, &mut row_rest, cx + hf.cs, 0);
            std::mem::swap(&mut row, &mut row_rest);
            n_row = n_row_rest;
            if n_cell < 3 || x < 0 {
                continue;
            }
            let mut smin = cell[0][1];
            let mut smax = cell[0][1];
            for p in &cell[1..n_cell] {
                smin = smin.min(p[1]);
                smax = smax.max(p[1]);
            }
            smin -= bmin[1];
            smax -= bmin[1];
            if smax < 0.0 || smin > by {
                continue;
            }
            if smin < 0.0 {
                smin = 0.0;
            }
            if smax > by {
                smax = by;
            }
            let lo = ((smin * ich).floor() as i32).clamp(0, SPAN_MAX_HEIGHT);
            let hi = ((smax * ich).ceil() as i32).clamp(lo + 1, SPAN_MAX_HEIGHT);
            hf.add_span(x, z, lo as u16, hi as u16, area, merge_threshold);
        }
    }
}

/// `rcRasterizeTriangles` (indexed).
pub fn rasterize_triangles(hf: &mut Heightfield, verts: &[[f32; 3]], tris: &[[u32; 3]], areas: &[u8], merge_threshold: i32) {
    let ics = 1.0 / hf.cs;
    let ich = 1.0 / hf.ch;
    for (t, &area) in tris.iter().zip(areas) {
        let v = [verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]];
        rasterize_triangle(hf, v, area, ics, ich, merge_threshold);
    }
}

/// `rcFilterLowHangingWalkableObstacles`: an unwalkable span just above a walkable one (within
/// the climb) becomes walkable, so kerbs and stair nosings can be stepped onto.
pub fn filter_low_hanging_obstacles(hf: &mut Heightfield, walkable_climb: i32) {
    for column in &mut hf.columns {
        let mut prev_walkable = false;
        let mut prev_area = NULL_AREA;
        let mut prev_smax = 0i32;
        for s in column.iter_mut() {
            let walkable = s.area != NULL_AREA;
            if !walkable && prev_walkable && s.smax as i32 - prev_smax <= walkable_climb {
                s.area = prev_area;
            }
            prev_walkable = walkable;
            prev_area = s.area;
            prev_smax = s.smax as i32;
        }
    }
}

/// `rcFilterLedgeSpans`: a walkable span next to a drop deeper than the climb, or whose
/// reachable neighbours differ in height by more than `max_spread`, becomes unwalkable. Recast
/// uses the climb as `max_spread`, which at coarse cells rejects ordinary stairs (two neighbouring
/// risers together exceed one climb); see [`super::Fixes::slope_aware_ledges`].
pub fn filter_ledge_spans(hf: &mut Heightfield, walkable_height: i32, walkable_climb: i32, max_spread: i32) {
    let (w, h) = (hf.width, hf.height);
    for z in 0..h {
        for x in 0..w {
            let ci = (x + z * w) as usize;
            for si in 0..hf.columns[ci].len() {
                let s = hf.columns[ci][si];
                if s.area == NULL_AREA {
                    continue;
                }
                let floor = s.smax as i32;
                let ceiling = hf.columns[ci].get(si + 1).map_or(MAX_HEIGHTFIELD_HEIGHT, |n| n.smin as i32);
                let mut lowest_diff = MAX_HEIGHTFIELD_HEIGHT;
                let mut lowest_floor = floor;
                let mut highest_floor = floor;
                for dir in 0..4 {
                    let (nx, nz) = (x + DIR_X[dir], z + DIR_Z[dir]);
                    if nx < 0 || nz < 0 || nx >= w || nz >= h {
                        lowest_diff = -walkable_climb - 1;
                        break;
                    }
                    let ncol = &hf.columns[(nx + nz * w) as usize];
                    let first_ceiling = ncol.first().map_or(MAX_HEIGHTFIELD_HEIGHT, |n| n.smin as i32);
                    if ceiling.min(first_ceiling) - floor >= walkable_height {
                        lowest_diff = -walkable_climb - 1;
                        break;
                    }
                    for (ni, n) in ncol.iter().enumerate() {
                        let nfloor = n.smax as i32;
                        let nceiling = ncol.get(ni + 1).map_or(MAX_HEIGHTFIELD_HEIGHT, |m| m.smin as i32);
                        if ceiling.min(nceiling) - floor.max(nfloor) < walkable_height {
                            continue;
                        }
                        let diff = nfloor - floor;
                        lowest_diff = lowest_diff.min(diff);
                        if diff.abs() <= walkable_climb {
                            lowest_floor = lowest_floor.min(nfloor);
                            highest_floor = highest_floor.max(nfloor);
                        } else if diff < -walkable_climb {
                            break;
                        }
                    }
                }
                if lowest_diff < -walkable_climb || highest_floor - lowest_floor > max_spread {
                    hf.columns[ci][si].area = NULL_AREA;
                }
            }
        }
    }
}

/// `rcFilterWalkableLowHeightSpans`: a span without headroom for the agent becomes unwalkable.
pub fn filter_low_height_spans(hf: &mut Heightfield, walkable_height: i32) {
    for column in &mut hf.columns {
        for i in 0..column.len() {
            let ceiling = column.get(i + 1).map_or(MAX_HEIGHTFIELD_HEIGHT, |n| n.smin as i32);
            if ceiling - (column[i].smax as i32) < walkable_height {
                column[i].area = NULL_AREA;
            }
        }
    }
}
