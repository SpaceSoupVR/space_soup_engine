//! Partitioning the walkable spans into regions, each of which becomes one simple contour. A port
//! of Recast's `rcBuildRegionsMonotone`, `rcBuildLayerRegions` and the merge/filter passes they
//! use, from `RecastRegion.cpp` (zlib licence, Copyright (c) 2009-2010 Mikko Mononen; altered:
//! written in Rust). Watershed partitioning is not ported (deferred by the design research).
//!
//! The merge passes are order-sensitive; they are kept statement for statement so the region ids
//! come out identical to Recast's, which the contour and polygon stages then depend on.

use super::compact::{CompactHeightfield, NOT_CONNECTED};
use super::heightfield::{DIR_X, DIR_Z};
use super::NULL_AREA;

/// Region ids with this bit are tile-border regions, never merged or kept.
pub const BORDER_REG: u16 = 0x8000;
const NULL_NEI: u16 = 0xffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Partition {
    Monotone,
    Layers,
}

#[derive(Clone)]
struct Region {
    span_count: i32,
    id: u16,
    area_type: u8,
    remap: bool,
    visited: bool,
    overlap: bool,
    connects_to_border: bool,
    ymin: u16,
    ymax: u16,
    connections: Vec<i32>,
    floors: Vec<i32>,
}

impl Region {
    fn new(id: u16) -> Region {
        Region {
            span_count: 0,
            id,
            area_type: 0,
            remap: false,
            visited: false,
            overlap: false,
            connects_to_border: false,
            ymin: 0xffff,
            ymax: 0,
            connections: Vec::new(),
            floors: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct SweepSpan {
    id: u16,
    ns: u16,
    nei: u16,
}

/// Builds regions in place (`chf.spans[i].reg`, `chf.max_regions`).
pub fn build_regions(chf: &mut CompactHeightfield, partition: Partition, border_size: i32, min_region_area: i32, merge_region_area: i32) {
    let (w, h) = (chf.width, chf.height);
    let mut id: u16 = 1;
    let mut src = vec![0u16; chf.spans.len()];

    if border_size > 0 {
        let bw = w.min(border_size);
        let bh = h.min(border_size);
        for (x0, x1, z0, z1) in [(0, bw, 0, h), (w - bw, w, 0, h), (0, w, 0, bh), (0, w, h - bh, h)] {
            paint_rect(chf, &mut src, x0, x1, z0, z1, id | BORDER_REG);
            id += 1;
        }
    }
    chf.border_size = border_size;

    let mut prev: Vec<i32> = vec![0; 256];
    let mut sweeps: Vec<SweepSpan> = Vec::new();
    for z in border_size..h - border_size {
        prev.resize(id as usize + 1, 0);
        for p in prev.iter_mut().take(id as usize) {
            *p = 0;
        }
        let mut rid: u16 = 1;
        for x in border_size..w - border_size {
            let cell = chf.cells[(x + z * w) as usize];
            for i in cell.index as usize..(cell.index + cell.count as u32) as usize {
                if chf.areas[i] == NULL_AREA {
                    continue;
                }
                let s = chf.spans[i];
                let mut previd = 0u16;
                if s.con[0] != NOT_CONNECTED {
                    let ai = chf.neighbour(x, z, 0, s.con[0]);
                    if src[ai] & BORDER_REG == 0 && chf.areas[i] == chf.areas[ai] {
                        previd = src[ai];
                    }
                }
                if previd == 0 {
                    previd = rid;
                    rid += 1;
                    if sweeps.len() <= previd as usize {
                        sweeps.resize(previd as usize + 1, SweepSpan::default());
                    }
                    sweeps[previd as usize].ns = 0;
                    sweeps[previd as usize].nei = 0;
                }
                if s.con[3] != NOT_CONNECTED {
                    let ai = chf.neighbour(x, z, 3, s.con[3]);
                    if src[ai] != 0 && src[ai] & BORDER_REG == 0 && chf.areas[i] == chf.areas[ai] {
                        let nr = src[ai];
                        let sw = &mut sweeps[previd as usize];
                        if sw.nei == 0 || sw.nei == nr {
                            sw.nei = nr;
                            sw.ns += 1;
                            prev[nr as usize] += 1;
                        } else {
                            sw.nei = NULL_NEI;
                        }
                    }
                }
                src[i] = previd;
            }
        }
        for sw in sweeps.iter_mut().take(rid as usize).skip(1) {
            if sw.nei != NULL_NEI && sw.nei != 0 && prev[sw.nei as usize] == sw.ns as i32 {
                sw.id = sw.nei;
            } else {
                sw.id = id;
                id += 1;
            }
        }
        for x in border_size..w - border_size {
            let cell = chf.cells[(x + z * w) as usize];
            for i in cell.index as usize..(cell.index + cell.count as u32) as usize {
                if src[i] > 0 && src[i] < rid {
                    src[i] = sweeps[src[i] as usize].id;
                }
            }
        }
    }

    let mut max_regions = id;
    match partition {
        Partition::Monotone => merge_and_filter_regions(chf, &mut src, min_region_area, merge_region_area, &mut max_regions),
        Partition::Layers => merge_and_filter_layer_regions(chf, &mut src, min_region_area, &mut max_regions),
    }
    chf.max_regions = max_regions;
    for (s, r) in chf.spans.iter_mut().zip(&src) {
        s.reg = *r;
    }
}

fn paint_rect(chf: &CompactHeightfield, src: &mut [u16], x0: i32, x1: i32, z0: i32, z1: i32, reg: u16) {
    for z in z0..z1 {
        for x in x0..x1 {
            let c = chf.cells[(x + z * chf.width) as usize];
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                if chf.areas[i] != NULL_AREA {
                    src[i] = reg;
                }
            }
        }
    }
}

fn remove_adjacent_duplicates(v: &mut Vec<i32>) {
    let mut i = 0;
    while i < v.len() && v.len() > 1 {
        let ni = (i + 1) % v.len();
        if v[i] == v[ni] {
            v.remove(i);
        } else {
            i += 1;
        }
    }
}

fn replace_neighbour(reg: &mut Region, old: u16, new: u16) {
    let mut changed = false;
    for c in reg.connections.iter_mut() {
        if *c == old as i32 {
            *c = new as i32;
            changed = true;
        }
    }
    for f in reg.floors.iter_mut() {
        if *f == old as i32 {
            *f = new as i32;
        }
    }
    if changed {
        remove_adjacent_duplicates(&mut reg.connections);
    }
}

fn can_merge(a: &Region, b: &Region) -> bool {
    if a.area_type != b.area_type {
        return false;
    }
    if a.connections.iter().filter(|&&c| c == b.id as i32).count() > 1 {
        return false;
    }
    !a.floors.contains(&(b.id as i32))
}

fn add_unique(v: &mut Vec<i32>, n: i32) {
    if !v.contains(&n) {
        v.push(n);
    }
}

/// `mergeRegions(rega = regions[a], regb = regions[b])`.
fn merge_regions(regions: &mut [Region], a: usize, b: usize) -> bool {
    let (aid, bid) = (regions[a].id, regions[b].id);
    let acon = regions[a].connections.clone();
    let bcon = regions[b].connections.clone();
    let Some(insa) = acon.iter().position(|&c| c == bid as i32) else { return false };
    let Some(insb) = bcon.iter().position(|&c| c == aid as i32) else { return false };
    let mut merged = Vec::with_capacity(acon.len() + bcon.len());
    for i in 0..acon.len() - 1 {
        merged.push(acon[(insa + 1 + i) % acon.len()]);
    }
    for i in 0..bcon.len() - 1 {
        merged.push(bcon[(insb + 1 + i) % bcon.len()]);
    }
    remove_adjacent_duplicates(&mut merged);
    regions[a].connections = merged;
    let bfloors = regions[b].floors.clone();
    for f in bfloors {
        add_unique(&mut regions[a].floors, f);
    }
    regions[a].span_count += regions[b].span_count;
    regions[b].span_count = 0;
    regions[b].connections.clear();
    true
}

fn is_solid_edge(chf: &CompactHeightfield, src: &[u16], x: i32, z: i32, i: usize, dir: usize) -> bool {
    let s = chf.spans[i];
    let mut r = 0;
    if s.con[dir] != NOT_CONNECTED {
        r = src[chf.neighbour(x, z, dir, s.con[dir])];
    }
    r != src[i]
}

fn walk_contour(chf: &CompactHeightfield, src: &[u16], mut x: i32, mut z: i32, mut i: usize, mut dir: usize, cont: &mut Vec<i32>) {
    let (start_dir, start_i) = (dir, i);
    let ss = chf.spans[i];
    let mut cur_reg = 0u16;
    if ss.con[dir] != NOT_CONNECTED {
        cur_reg = src[chf.neighbour(x, z, dir, ss.con[dir])];
    }
    cont.push(cur_reg as i32);
    let mut iter = 0;
    loop {
        iter += 1;
        if iter >= 40000 {
            break;
        }
        let s = chf.spans[i];
        if is_solid_edge(chf, src, x, z, i, dir) {
            let mut r = 0u16;
            if s.con[dir] != NOT_CONNECTED {
                r = src[chf.neighbour(x, z, dir, s.con[dir])];
            }
            if r != cur_reg {
                cur_reg = r;
                cont.push(cur_reg as i32);
            }
            dir = (dir + 1) & 3;
        } else {
            if s.con[dir] == NOT_CONNECTED {
                return;
            }
            let ni = chf.neighbour(x, z, dir, s.con[dir]);
            x += DIR_X[dir];
            z += DIR_Z[dir];
            i = ni;
            dir = (dir + 3) & 3;
        }
        if start_i == i && start_dir == dir {
            break;
        }
    }
    if cont.len() > 1 {
        let mut j = 0;
        while j < cont.len() {
            let nj = (j + 1) % cont.len();
            if cont[j] == cont[nj] {
                cont.remove(j);
            } else {
                j += 1;
            }
        }
    }
}

fn compress_ids(regions: &mut [Region], src: &mut [u16], max_region_id: &mut u16) {
    for r in regions.iter_mut() {
        r.remap = r.id != 0 && r.id & BORDER_REG == 0;
    }
    let mut gen: u16 = 0;
    for i in 0..regions.len() {
        if !regions[i].remap {
            continue;
        }
        let old = regions[i].id;
        gen += 1;
        for r in regions[i..].iter_mut() {
            if r.id == old {
                r.id = gen;
                r.remap = false;
            }
        }
    }
    *max_region_id = gen;
    for r in src.iter_mut() {
        if *r & BORDER_REG == 0 {
            *r = regions[*r as usize].id;
        }
    }
}

/// `mergeAndFilterRegions` (monotone).
fn merge_and_filter_regions(chf: &CompactHeightfield, src: &mut [u16], min_region_area: i32, merge_region_size: i32, max_region_id: &mut u16) {
    let (w, h) = (chf.width, chf.height);
    let nreg = *max_region_id as usize + 1;
    let mut regions: Vec<Region> = (0..nreg).map(|i| Region::new(i as u16)).collect();

    for z in 0..h {
        for x in 0..w {
            let c = chf.cells[(x + z * w) as usize];
            let (first, end) = (c.index as usize, (c.index + c.count as u32) as usize);
            for i in first..end {
                let r = src[i];
                if r == 0 || r as usize >= nreg {
                    continue;
                }
                let reg = &mut regions[r as usize];
                reg.span_count += 1;
                for (j, &floor) in src.iter().enumerate().take(end).skip(first) {
                    if i == j || floor == 0 || floor as usize >= nreg {
                        continue;
                    }
                    if floor == r {
                        reg.overlap = true;
                    }
                    add_unique(&mut reg.floors, floor as i32);
                }
                if !reg.connections.is_empty() {
                    continue;
                }
                reg.area_type = chf.areas[i];
                if let Some(ndir) = (0..4).find(|&d| is_solid_edge(chf, src, x, z, i, d)) {
                    let mut cont = Vec::new();
                    walk_contour(chf, src, x, z, i, ndir, &mut cont);
                    regions[r as usize].connections = cont;
                }
            }
        }
    }

    // Remove too small regions (counting whole connected groups).
    let mut stack: Vec<i32> = Vec::new();
    let mut trace: Vec<i32> = Vec::new();
    for i in 0..nreg {
        {
            let reg = &regions[i];
            if reg.id == 0 || reg.id & BORDER_REG != 0 || reg.span_count == 0 || reg.visited {
                continue;
            }
        }
        let mut connects_to_border = false;
        let mut span_count = 0;
        stack.clear();
        trace.clear();
        regions[i].visited = true;
        stack.push(i as i32);
        while let Some(ri) = stack.pop() {
            span_count += regions[ri as usize].span_count;
            trace.push(ri);
            for j in 0..regions[ri as usize].connections.len() {
                let c = regions[ri as usize].connections[j];
                if c & BORDER_REG as i32 != 0 {
                    connects_to_border = true;
                    continue;
                }
                let nei = &mut regions[c as usize];
                if nei.visited || nei.id == 0 || nei.id & BORDER_REG != 0 {
                    continue;
                }
                stack.push(nei.id as i32);
                nei.visited = true;
            }
        }
        if span_count < min_region_area && !connects_to_border {
            for &t in &trace {
                regions[t as usize].span_count = 0;
                regions[t as usize].id = 0;
            }
        }
    }

    // Merge too small regions into neighbours.
    loop {
        let mut merge_count = 0;
        for i in 0..nreg {
            let reg = &regions[i];
            if reg.id == 0 || reg.id & BORDER_REG != 0 || reg.overlap || reg.span_count == 0 {
                continue;
            }
            if reg.span_count > merge_region_size && reg.connections.contains(&0) {
                continue;
            }
            let mut smallest = 0xfffffff;
            let mut merge_id = reg.id;
            for &c in &reg.connections {
                if c & BORDER_REG as i32 != 0 {
                    continue;
                }
                let m = &regions[c as usize];
                if m.id == 0 || m.id & BORDER_REG != 0 || m.overlap {
                    continue;
                }
                if m.span_count < smallest && can_merge(reg, m) && can_merge(m, reg) {
                    smallest = m.span_count;
                    merge_id = m.id;
                }
            }
            if merge_id != reg.id {
                let old = reg.id;
                if merge_regions(&mut regions, merge_id as usize, i) {
                    for r in regions.iter_mut() {
                        if r.id == 0 || r.id & BORDER_REG != 0 {
                            continue;
                        }
                        if r.id == old {
                            r.id = merge_id;
                        }
                        replace_neighbour(r, old, merge_id);
                    }
                    merge_count += 1;
                }
            }
        }
        if merge_count == 0 {
            break;
        }
    }

    compress_ids(&mut regions, src, max_region_id);
}

/// `mergeAndFilterLayerRegions`.
fn merge_and_filter_layer_regions(chf: &CompactHeightfield, src: &mut [u16], min_region_area: i32, max_region_id: &mut u16) {
    let (w, h) = (chf.width, chf.height);
    let nreg = *max_region_id as usize + 1;
    let mut regions: Vec<Region> = (0..nreg).map(|i| Region::new(i as u16)).collect();

    let mut lregs: Vec<i32> = Vec::new();
    for z in 0..h {
        for x in 0..w {
            let c = chf.cells[(x + z * w) as usize];
            lregs.clear();
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                let s = chf.spans[i];
                let ri = src[i];
                if ri == 0 || ri as usize >= nreg {
                    continue;
                }
                let reg = &mut regions[ri as usize];
                reg.span_count += 1;
                reg.area_type = chf.areas[i];
                reg.ymin = reg.ymin.min(s.y);
                reg.ymax = reg.ymax.max(s.y);
                lregs.push(ri as i32);
                for dir in 0..4 {
                    if s.con[dir] != NOT_CONNECTED {
                        let rai = src[chf.neighbour(x, z, dir, s.con[dir])];
                        if rai > 0 && (rai as usize) < nreg && rai != ri {
                            add_unique(&mut reg.connections, rai as i32);
                        }
                        if rai & BORDER_REG != 0 {
                            reg.connects_to_border = true;
                        }
                    }
                }
            }
            for i in 0..lregs.len().saturating_sub(1) {
                for j in i + 1..lregs.len() {
                    if lregs[i] != lregs[j] {
                        add_unique(&mut regions[lregs[i] as usize].floors, lregs[j]);
                        add_unique(&mut regions[lregs[j] as usize].floors, lregs[i]);
                    }
                }
            }
        }
    }

    let mut layer_id: u16 = 1;
    for r in regions.iter_mut() {
        r.id = 0;
    }
    let mut stack: std::collections::VecDeque<i32> = std::collections::VecDeque::new();
    for i in 1..nreg {
        if regions[i].id != 0 {
            continue;
        }
        regions[i].id = layer_id;
        stack.clear();
        stack.push_back(i as i32);
        while let Some(ri) = stack.pop_front() {
            let ncons = regions[ri as usize].connections.len();
            for j in 0..ncons {
                let nei = regions[ri as usize].connections[j];
                if regions[nei as usize].id != 0 {
                    continue;
                }
                if regions[ri as usize].area_type != regions[nei as usize].area_type {
                    continue;
                }
                if regions[i].floors.contains(&nei) {
                    continue;
                }
                stack.push_back(nei);
                regions[nei as usize].id = layer_id;
                let nfloors = regions[nei as usize].floors.clone();
                for f in nfloors {
                    add_unique(&mut regions[i].floors, f);
                }
                let (ymin, ymax, sc, ctb) = {
                    let n = &regions[nei as usize];
                    (n.ymin, n.ymax, n.span_count, n.connects_to_border)
                };
                let root = &mut regions[i];
                root.ymin = root.ymin.min(ymin);
                root.ymax = root.ymax.max(ymax);
                root.span_count += sc;
                root.connects_to_border = root.connects_to_border || ctb;
                regions[nei as usize].span_count = 0;
            }
        }
        layer_id += 1;
    }

    for i in 0..nreg {
        let r = &regions[i];
        if r.span_count > 0 && r.span_count < min_region_area && !r.connects_to_border {
            let id = r.id;
            for other in regions.iter_mut() {
                if other.id == id {
                    other.id = 0;
                }
            }
        }
    }

    compress_ids(&mut regions, src, max_region_id);
}
