//! The compact heightfield: the open space above each walkable span, with links to the
//! neighbouring spans an agent can step to, then eroded by the agent's radius. A port of Recast's
//! `rcBuildCompactHeightfield` and `rcErodeWalkableArea` (zlib licence, Copyright (c) 2009-2010
//! Mikko Mononen; altered: written in Rust).

use super::heightfield::{Heightfield, DIR_X, DIR_Z};
use super::NULL_AREA;

/// No neighbour in that direction.
pub const NOT_CONNECTED: u8 = 0x3f;

#[derive(Debug, Clone, Copy, Default)]
pub struct CompactCell {
    pub index: u32,
    pub count: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompactSpan {
    /// Floor height (the solid span's top), cell heights.
    pub y: u16,
    pub reg: u16,
    /// Per direction (x-, z+, x+, z-), the index of the neighbour within its column, or
    /// [`NOT_CONNECTED`].
    pub con: [u8; 4],
    /// Open height above the floor, clamped to 255.
    pub h: u8,
}

pub struct CompactHeightfield {
    pub width: i32,
    pub height: i32,
    pub walkable_height: i32,
    pub walkable_climb: i32,
    pub border_size: i32,
    pub max_distance: u16,
    pub max_regions: u16,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    pub cells: Vec<CompactCell>,
    pub spans: Vec<CompactSpan>,
    pub dist: Vec<u16>,
    pub areas: Vec<u8>,
}

impl CompactHeightfield {
    /// Index of the span `dir` of span `i` in cell `(x, z)` is connected to. The caller has
    /// checked the connection exists.
    #[inline]
    pub fn neighbour(&self, x: i32, z: i32, dir: usize, con: u8) -> usize {
        let (nx, nz) = (x + DIR_X[dir], z + DIR_Z[dir]);
        self.cells[(nx + nz * self.width) as usize].index as usize + con as usize
    }

    pub fn build(walkable_height: i32, walkable_climb: i32, hf: &Heightfield) -> CompactHeightfield {
        let (w, h) = (hf.width, hf.height);
        let mut chf = CompactHeightfield {
            width: w,
            height: h,
            walkable_height,
            walkable_climb,
            border_size: 0,
            max_distance: 0,
            max_regions: 0,
            bmin: hf.bmin,
            bmax: [hf.bmax[0], hf.bmax[1] + walkable_height as f32 * hf.ch, hf.bmax[2]],
            cs: hf.cs,
            ch: hf.ch,
            cells: vec![CompactCell::default(); (w * h) as usize],
            spans: Vec::with_capacity(hf.walkable_span_count()),
            dist: Vec::new(),
            areas: Vec::with_capacity(hf.walkable_span_count()),
        };
        const MAX_HEIGHT: i32 = 0xffff;
        for (ci, column) in hf.columns.iter().enumerate() {
            if column.is_empty() {
                continue;
            }
            let cell = &mut chf.cells[ci];
            cell.index = chf.spans.len() as u32;
            cell.count = 0;
            for (si, s) in column.iter().enumerate() {
                if s.area == NULL_AREA {
                    continue;
                }
                let bot = i32::from(s.smax);
                let top = column.get(si + 1).map_or(MAX_HEIGHT, |n| i32::from(n.smin));
                chf.spans.push(CompactSpan {
                    y: bot.clamp(0, 0xffff) as u16,
                    reg: 0,
                    con: [NOT_CONNECTED; 4],
                    h: (top - bot).clamp(0, 0xff) as u8,
                });
                chf.areas.push(s.area);
                cell.count += 1;
            }
        }
        for z in 0..h {
            for x in 0..w {
                let cell = chf.cells[(x + z * w) as usize];
                for i in cell.index as usize..(cell.index as usize + cell.count as usize) {
                    for dir in 0..4 {
                        let (nx, nz) = (x + DIR_X[dir], z + DIR_Z[dir]);
                        if nx < 0 || nz < 0 || nx >= w || nz >= h {
                            continue;
                        }
                        let s = chf.spans[i];
                        let nc = chf.cells[(nx + nz * w) as usize];
                        for k in nc.index as usize..(nc.index as usize + nc.count as usize) {
                            let n = chf.spans[k];
                            let bot = i32::from(s.y).max(i32::from(n.y));
                            let top = (i32::from(s.y) + i32::from(s.h)).min(i32::from(n.y) + i32::from(n.h));
                            if top - bot >= walkable_height && (i32::from(n.y) - i32::from(s.y)).abs() <= walkable_climb {
                                let layer = k - nc.index as usize;
                                if layer < NOT_CONNECTED as usize {
                                    chf.spans[i].con[dir] = layer as u8;
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        chf
    }

    pub fn walkable_count(&self) -> usize {
        self.areas.iter().filter(|&&a| a != NULL_AREA).count()
    }
}

/// `rcErodeWalkableArea`: a two-pass chamfer distance (2 straight, 3 diagonal) from the edge of
/// the walkable area; spans nearer than the radius stop being walkable.
pub fn erode_walkable_area(chf: &mut CompactHeightfield, radius: i32) {
    let (w, h) = (chf.width, chf.height);
    let mut dist = vec![0xffu8; chf.spans.len()];
    for z in 0..h {
        for x in 0..w {
            let cell = chf.cells[(x + z * w) as usize];
            for i in cell.index as usize..(cell.index as usize + cell.count as usize) {
                if chf.areas[i] == NULL_AREA {
                    dist[i] = 0;
                    continue;
                }
                let s = chf.spans[i];
                let mut neighbours = 0;
                for dir in 0..4 {
                    let con = s.con[dir];
                    if con == NOT_CONNECTED {
                        break;
                    }
                    if chf.areas[chf.neighbour(x, z, dir, con)] == NULL_AREA {
                        break;
                    }
                    neighbours += 1;
                }
                if neighbours != 4 {
                    dist[i] = 0;
                }
            }
        }
    }

    chamfer(chf, &mut dist);
    let min = (radius * 2) as u8;
    for (area, d) in chf.areas.iter_mut().zip(&dist) {
        if *d < min {
            *area = NULL_AREA;
        }
    }
}

/// Recast's two-pass chamfer distance over the compact heightfield (2 per straight step, 3 per
/// diagonal), from the spans already at 0 in `dist`.
pub fn chamfer(chf: &CompactHeightfield, dist: &mut [u8]) {
    let (w, h) = (chf.width, chf.height);
    let relax = |chf: &CompactHeightfield, dist: &mut [u8], x: i32, z: i32, i: usize, dir: usize, diag: usize| {
        let s = chf.spans[i];
        if s.con[dir] == NOT_CONNECTED {
            return;
        }
        let a = chf.neighbour(x, z, dir, s.con[dir]);
        let nd = (i32::from(dist[a]) + 2).min(255) as u8;
        if nd < dist[i] {
            dist[i] = nd;
        }
        let aspan = chf.spans[a];
        if aspan.con[diag] != NOT_CONNECTED {
            let b = chf.neighbour(x + DIR_X[dir], z + DIR_Z[dir], diag, aspan.con[diag]);
            let nd = (i32::from(dist[b]) + 3).min(255) as u8;
            if nd < dist[i] {
                dist[i] = nd;
            }
        }
    };
    for z in 0..h {
        for x in 0..w {
            let cell = chf.cells[(x + z * w) as usize];
            for i in cell.index as usize..(cell.index as usize + cell.count as usize) {
                relax(chf, dist, x, z, i, 0, 3);
                relax(chf, dist, x, z, i, 3, 2);
            }
        }
    }
    for z in (0..h).rev() {
        for x in (0..w).rev() {
            let cell = chf.cells[(x + z * w) as usize];
            for i in cell.index as usize..(cell.index as usize + cell.count as usize) {
                relax(chf, dist, x, z, i, 2, 1);
                relax(chf, dist, x, z, i, 1, 0);
            }
        }
    }
}
