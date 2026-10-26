//! Tracing each region's outline and simplifying it to a polygon. A port of Recast's
//! `RecastContour.cpp` (zlib licence, Copyright (c) 2009-2010 Mikko Mononen; altered: written in
//! Rust).
//!
//! Two places cannot be bit-for-bit C++: `qsort` is unstable (ties keep input order here), and
//! hole merging tests a diagonal against `diags[i]` (the hole's index) where `diags[j]` is meant.
//! That bug is kept so the output matches Recast, with never-written entries reading as 0 where
//! C++ reads stale memory.

use super::compact::{CompactHeightfield, NOT_CONNECTED};
use super::heightfield::{DIR_X, DIR_Z};
use super::region::BORDER_REG;

pub const BORDER_VERTEX: i32 = 0x10000;
pub const AREA_BORDER: i32 = 0x20000;
pub const CONTOUR_REG_MASK: i32 = 0xffff;
pub const TESS_WALL_EDGES: i32 = 1;
pub const TESS_AREA_EDGES: i32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    /// Simplified vertices: x, y, z in cells, then flags and the neighbouring region.
    pub verts: Vec<[i32; 4]>,
    /// The traced outline before simplification.
    pub rverts: Vec<[i32; 4]>,
    pub reg: u16,
    pub area: u8,
}

pub struct ContourSet {
    pub conts: Vec<Contour>,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    pub width: i32,
    pub height: i32,
    pub border_size: i32,
    pub max_error: f32,
}

fn corner_height(chf: &CompactHeightfield, x: i32, z: i32, i: usize, dir: usize, is_border_vertex: &mut bool) -> i32 {
    let s = chf.spans[i];
    let mut ch = s.y as i32;
    let dirp = (dir + 1) & 3;
    let mut regs = [0u32; 4];
    let code = |k: usize| chf.spans[k].reg as u32 | ((chf.areas[k] as u32) << 16);
    regs[0] = code(i);
    if s.con[dir] != NOT_CONNECTED {
        let ai = chf.neighbour(x, z, dir, s.con[dir]);
        let a = chf.spans[ai];
        ch = ch.max(a.y as i32);
        regs[1] = code(ai);
        if a.con[dirp] != NOT_CONNECTED {
            let ai2 = chf.neighbour(x + DIR_X[dir], z + DIR_Z[dir], dirp, a.con[dirp]);
            ch = ch.max(chf.spans[ai2].y as i32);
            regs[2] = code(ai2);
        }
    }
    if s.con[dirp] != NOT_CONNECTED {
        let ai = chf.neighbour(x, z, dirp, s.con[dirp]);
        let a = chf.spans[ai];
        ch = ch.max(a.y as i32);
        regs[3] = code(ai);
        if a.con[dir] != NOT_CONNECTED {
            let ai2 = chf.neighbour(x + DIR_X[dirp], z + DIR_Z[dirp], dir, a.con[dir]);
            ch = ch.max(chf.spans[ai2].y as i32);
            regs[2] = code(ai2);
        }
    }
    let border = BORDER_REG as u32;
    for j in 0..4 {
        let (a, b, c, d) = (j, (j + 1) & 3, (j + 2) & 3, (j + 3) & 3);
        let two_same_exts = (regs[a] & regs[b] & border) != 0 && regs[a] == regs[b];
        let two_ints = ((regs[c] | regs[d]) & border) == 0;
        let ints_same_area = (regs[c] >> 16) == (regs[d] >> 16);
        let no_zeros = regs[a] != 0 && regs[b] != 0 && regs[c] != 0 && regs[d] != 0;
        if two_same_exts && two_ints && ints_same_area && no_zeros {
            *is_border_vertex = true;
            break;
        }
    }
    ch
}

fn walk_contour(chf: &CompactHeightfield, mut x: i32, mut z: i32, mut i: usize, flags: &mut [u8], points: &mut Vec<[i32; 4]>) {
    let mut dir = 0usize;
    while flags[i] & (1 << dir) == 0 {
        dir += 1;
    }
    let (start_dir, start_i) = (dir, i);
    let area = chf.areas[i];
    let mut iter = 0;
    loop {
        iter += 1;
        if iter >= 40000 {
            break;
        }
        if flags[i] & (1 << dir) != 0 {
            let mut is_border_vertex = false;
            let mut is_area_border = false;
            let mut px = x;
            let py = corner_height(chf, x, z, i, dir, &mut is_border_vertex);
            let mut pz = z;
            match dir {
                0 => pz += 1,
                1 => {
                    px += 1;
                    pz += 1;
                }
                2 => px += 1,
                _ => {}
            }
            let mut r = 0i32;
            let s = chf.spans[i];
            if s.con[dir] != NOT_CONNECTED {
                let ai = chf.neighbour(x, z, dir, s.con[dir]);
                r = chf.spans[ai].reg as i32;
                if area != chf.areas[ai] {
                    is_area_border = true;
                }
            }
            if is_border_vertex {
                r |= BORDER_VERTEX;
            }
            if is_area_border {
                r |= AREA_BORDER;
            }
            points.push([px, py, pz, r]);
            flags[i] &= !(1u8 << dir);
            dir = (dir + 1) & 3;
        } else {
            let s = chf.spans[i];
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
}

fn distance_pt_seg(x: i32, z: i32, px: i32, pz: i32, qx: i32, qz: i32) -> f32 {
    let pqx = (qx - px) as f32;
    let pqz = (qz - pz) as f32;
    let mut dx = (x - px) as f32;
    let mut dz = (z - pz) as f32;
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    if t < 0.0 {
        t = 0.0;
    } else if t > 1.0 {
        t = 1.0;
    }
    dx = px as f32 + t * pqx - x as f32;
    dz = pz as f32 + t * pqz - z as f32;
    dx * dx + dz * dz
}

fn simplify_contour(points: &[[i32; 4]], simplified: &mut Vec<[i32; 4]>, max_error: f32, max_edge_len: i32, build_flags: i32) {
    let has_connections = points.iter().any(|p| p[3] & CONTOUR_REG_MASK != 0);
    let pn = points.len();
    if has_connections {
        for i in 0..pn {
            let ii = (i + 1) % pn;
            let different_regs = (points[i][3] & CONTOUR_REG_MASK) != (points[ii][3] & CONTOUR_REG_MASK);
            let area_borders = (points[i][3] & AREA_BORDER) != (points[ii][3] & AREA_BORDER);
            if different_regs || area_borders {
                simplified.push([points[i][0], points[i][1], points[i][2], i as i32]);
            }
        }
    }
    if simplified.is_empty() {
        let (mut ll, mut lli) = (points[0], 0usize);
        let (mut ur, mut uri) = (points[0], 0usize);
        for (i, p) in points.iter().enumerate() {
            let (x, z) = (p[0], p[2]);
            if x < ll[0] || (x == ll[0] && z < ll[2]) {
                ll = *p;
                lli = i;
            }
            if x > ur[0] || (x == ur[0] && z > ur[2]) {
                ur = *p;
                uri = i;
            }
        }
        simplified.push([ll[0], ll[1], ll[2], lli as i32]);
        simplified.push([ur[0], ur[1], ur[2], uri as i32]);
    }

    let pn_i = pn as i32;
    let mut i = 0;
    while i < simplified.len() {
        let ii = (i + 1) % simplified.len();
        let (mut ax, mut az, ai) = (simplified[i][0], simplified[i][2], simplified[i][3]);
        let (mut bx, mut bz, bi) = (simplified[ii][0], simplified[ii][2], simplified[ii][3]);
        let mut maxd = 0.0f32;
        let mut maxi = -1i32;
        let (mut ci, cinc, endi);
        if bx > ax || (bx == ax && bz > az) {
            cinc = 1;
            ci = (ai + cinc) % pn_i;
            endi = bi;
        } else {
            cinc = pn_i - 1;
            ci = (bi + cinc) % pn_i;
            endi = ai;
            std::mem::swap(&mut ax, &mut bx);
            std::mem::swap(&mut az, &mut bz);
        }
        let cf = points[ci as usize][3];
        if cf & CONTOUR_REG_MASK == 0 || cf & AREA_BORDER != 0 {
            while ci != endi {
                let p = points[ci as usize];
                let d = distance_pt_seg(p[0], p[2], ax, az, bx, bz);
                if d > maxd {
                    maxd = d;
                    maxi = ci;
                }
                ci = (ci + cinc) % pn_i;
            }
        }
        if maxi != -1 && maxd > max_error * max_error {
            let p = points[maxi as usize];
            simplified.insert(i + 1, [p[0], p[1], p[2], maxi]);
        } else {
            i += 1;
        }
    }

    if max_edge_len > 0 && build_flags & (TESS_WALL_EDGES | TESS_AREA_EDGES) != 0 {
        let mut i = 0;
        while i < simplified.len() {
            let ii = (i + 1) % simplified.len();
            let (ax, az, ai) = (simplified[i][0], simplified[i][2], simplified[i][3]);
            let (bx, bz, bi) = (simplified[ii][0], simplified[ii][2], simplified[ii][3]);
            let mut maxi = -1i32;
            let ci = ((ai + 1) % pn_i) as usize;
            let mut tess = false;
            if build_flags & TESS_WALL_EDGES != 0 && points[ci][3] & CONTOUR_REG_MASK == 0 {
                tess = true;
            }
            if build_flags & TESS_AREA_EDGES != 0 && points[ci][3] & AREA_BORDER != 0 {
                tess = true;
            }
            if tess {
                let (dx, dz) = (bx - ax, bz - az);
                if dx * dx + dz * dz > max_edge_len * max_edge_len {
                    let n = if bi < ai { bi + pn_i - ai } else { bi - ai };
                    if n > 1 {
                        maxi = if bx > ax || (bx == ax && bz > az) { (ai + n / 2) % pn_i } else { (ai + (n + 1) / 2) % pn_i };
                    }
                }
            }
            if maxi != -1 {
                let p = points[maxi as usize];
                simplified.insert(i + 1, [p[0], p[1], p[2], maxi]);
            } else {
                i += 1;
            }
        }
    }

    for s in simplified.iter_mut() {
        let ai = ((s[3] + 1) % pn_i) as usize;
        let bi = s[3] as usize;
        s[3] = (points[ai][3] & (CONTOUR_REG_MASK | AREA_BORDER)) | (points[bi][3] & BORDER_VERTEX);
    }
}

fn area_of_polygon_2d(verts: &[[i32; 4]]) -> i32 {
    let mut area = 0;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (verts[i], verts[j]);
        area += vi[0] * vj[2] - vj[0] * vi[2];
        j = i;
    }
    (area + 1) / 2
}

#[inline]
fn prev(i: usize, n: usize) -> usize {
    if i >= 1 { i - 1 } else { n - 1 }
}
#[inline]
fn next(i: usize, n: usize) -> usize {
    if i + 1 < n { i + 1 } else { 0 }
}
#[inline]
fn twice_area(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> i32 {
    (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])
}
#[inline]
fn left(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    twice_area(a, b, c) < 0
}
#[inline]
fn left_on(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    twice_area(a, b, c) <= 0
}
#[inline]
fn collinear(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    twice_area(a, b, c) == 0
}
fn intersect_prop(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4], d: &[i32; 4]) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) ^ left(a, b, d)) && (left(c, d, a) ^ left(c, d, b))
}
fn between(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4]) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a[0] != b[0] {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    } else {
        (a[2] <= c[2] && c[2] <= b[2]) || (a[2] >= c[2] && c[2] >= b[2])
    }
}
fn intersect(a: &[i32; 4], b: &[i32; 4], c: &[i32; 4], d: &[i32; 4]) -> bool {
    intersect_prop(a, b, c, d) || between(a, b, c) || between(a, b, d) || between(c, d, a) || between(c, d, b)
}
fn vequal(a: &[i32; 4], b: &[i32; 4]) -> bool {
    a[0] == b[0] && a[2] == b[2]
}
/// `skip` is the vertex whose incident edges are ignored; C++ passes it as an int that may be
/// -1 or any stale value, so it is compared as one.
fn intersect_seg_contour(d0: &[i32; 4], d1: &[i32; 4], skip: i32, verts: &[[i32; 4]]) -> bool {
    let n = verts.len();
    for k in 0..n {
        let k1 = next(k, n);
        if skip == k as i32 || skip == k1 as i32 {
            continue;
        }
        let (p0, p1) = (&verts[k], &verts[k1]);
        if vequal(d0, p0) || vequal(d1, p0) || vequal(d0, p1) || vequal(d1, p1) {
            continue;
        }
        if intersect(d0, d1, p0, p1) {
            return true;
        }
    }
    false
}
fn in_cone(i: usize, verts: &[[i32; 4]], pj: &[i32; 4]) -> bool {
    let n = verts.len();
    let pi = &verts[i];
    let pi1 = &verts[next(i, n)];
    let next_pin = &verts[prev(i, n)];
    if left_on(next_pin, pi, pi1) {
        return left(pi, pj, next_pin) && left(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, next_pin))
}

fn remove_degenerate_segments(simplified: &mut Vec<[i32; 4]>) {
    let mut npts = simplified.len();
    let mut i = 0;
    while i < npts {
        let ni = next(i, npts);
        if vequal(&simplified[i], &simplified[ni]) {
            simplified.remove(i);
            npts -= 1;
        }
        i += 1;
    }
}

fn merge_contours(ca: &mut Contour, cb: &mut Contour, ia: usize, ib: usize) {
    let mut verts = Vec::with_capacity(ca.verts.len() + cb.verts.len() + 2);
    for i in 0..=ca.verts.len() {
        verts.push(ca.verts[(ia + i) % ca.verts.len()]);
    }
    for i in 0..=cb.verts.len() {
        verts.push(cb.verts[(ib + i) % cb.verts.len()]);
    }
    ca.verts = verts;
    cb.verts.clear();
}

fn leftmost_vertex(c: &Contour) -> (i32, i32, usize) {
    let (mut minx, mut minz, mut leftmost) = (c.verts[0][0], c.verts[0][2], 0);
    for (i, v) in c.verts.iter().enumerate().skip(1) {
        if v[0] < minx || (v[0] == minx && v[2] < minz) {
            minx = v[0];
            minz = v[2];
            leftmost = i;
        }
    }
    (minx, minz, leftmost)
}

/// `mergeRegionHoles`: joins each hole to the outline through the shortest diagonal that crosses
/// nothing, left to right.
fn merge_region_holes(conts: &mut [Contour], outline: usize, holes: &[usize]) {
    let mut holes: Vec<(usize, i32, i32, usize)> = holes
        .iter()
        .map(|&h| {
            let (minx, minz, leftmost) = leftmost_vertex(&conts[h]);
            (h, minx, minz, leftmost)
        })
        .collect();
    holes.sort_by_key(|h| (h.1, h.2));
    let max_verts = conts[outline].verts.len() + holes.iter().map(|h| conts[h.0].verts.len()).sum::<usize>();
    let mut diags: Vec<(i32, i32)> = vec![(0, 0); max_verts.max(1)];

    for hi in 0..holes.len() {
        let hole = holes[hi].0;
        let mut index = -1i32;
        let mut best = holes[hi].3;
        for _ in 0..conts[hole].verts.len() {
            let corner = conts[hole].verts[best];
            let mut ndiags = 0;
            let ov = &conts[outline].verts;
            for j in 0..ov.len() {
                if in_cone(j, ov, &corner) {
                    let (dx, dz) = (ov[j][0] - corner[0], ov[j][2] - corner[2]);
                    diags[ndiags] = (j as i32, dx * dx + dz * dz);
                    ndiags += 1;
                }
            }
            diags[..ndiags].sort_by_key(|d| d.1);
            index = -1;
            for j in 0..ndiags {
                let pt = conts[outline].verts[diags[j].0 as usize];
                let skip = diags.get(hi).map_or(0, |d| d.0);
                let mut crosses = intersect_seg_contour(&pt, &corner, skip, &conts[outline].verts);
                let mut k = hi;
                while k < holes.len() && !crosses {
                    crosses |= intersect_seg_contour(&pt, &corner, -1, &conts[holes[k].0].verts);
                    k += 1;
                }
                if !crosses {
                    index = diags[j].0;
                    break;
                }
            }
            if index != -1 {
                break;
            }
            best = (best + 1) % conts[hole].verts.len();
        }
        if index == -1 {
            continue;
        }
        let (a, b) = if outline < hole {
            let (l, r) = conts.split_at_mut(hole);
            (&mut l[outline], &mut r[0])
        } else {
            let (l, r) = conts.split_at_mut(outline);
            (&mut r[0], &mut l[hole])
        };
        merge_contours(a, b, index as usize, best);
    }
}

/// `rcBuildContours`.
pub fn build_contours(chf: &CompactHeightfield, max_error: f32, max_edge_len: i32, build_flags: i32) -> ContourSet {
    let (w, h) = (chf.width, chf.height);
    let border = chf.border_size;
    let mut cset = ContourSet {
        conts: Vec::new(),
        bmin: chf.bmin,
        bmax: chf.bmax,
        cs: chf.cs,
        ch: chf.ch,
        width: chf.width - border * 2,
        height: chf.height - border * 2,
        border_size: border,
        max_error,
    };
    if border > 0 {
        let pad = border as f32 * chf.cs;
        cset.bmin[0] += pad;
        cset.bmin[2] += pad;
        cset.bmax[0] -= pad;
        cset.bmax[2] -= pad;
    }

    let mut flags = vec![0u8; chf.spans.len()];
    for z in 0..h {
        for x in 0..w {
            let c = chf.cells[(x + z * w) as usize];
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                let s = chf.spans[i];
                if s.reg == 0 || s.reg & BORDER_REG != 0 {
                    flags[i] = 0;
                    continue;
                }
                let mut outcome = 0u8;
                for dir in 0..4 {
                    let mut r = 0u16;
                    if s.con[dir] != NOT_CONNECTED {
                        r = chf.spans[chf.neighbour(x, z, dir, s.con[dir])].reg;
                    }
                    if r == s.reg {
                        outcome |= 1 << dir;
                    }
                }
                flags[i] = outcome ^ 0xf;
            }
        }
    }

    let mut verts: Vec<[i32; 4]> = Vec::new();
    let mut simplified: Vec<[i32; 4]> = Vec::new();
    for z in 0..h {
        for x in 0..w {
            let c = chf.cells[(x + z * w) as usize];
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                if flags[i] == 0 || flags[i] == 0xf {
                    flags[i] = 0;
                    continue;
                }
                let reg = chf.spans[i].reg;
                if reg == 0 || reg & BORDER_REG != 0 {
                    continue;
                }
                verts.clear();
                simplified.clear();
                walk_contour(chf, x, z, i, &mut flags, &mut verts);
                simplify_contour(&verts, &mut simplified, max_error, max_edge_len, build_flags);
                remove_degenerate_segments(&mut simplified);
                if simplified.len() >= 3 {
                    let mut cv = simplified.clone();
                    let mut rv = verts.clone();
                    if border > 0 {
                        for v in cv.iter_mut().chain(rv.iter_mut()) {
                            v[0] -= border;
                            v[2] -= border;
                        }
                    }
                    cset.conts.push(Contour { verts: cv, rverts: rv, reg, area: chf.areas[i] });
                }
            }
        }
    }

    if !cset.conts.is_empty() {
        let winding: Vec<i8> = cset.conts.iter().map(|c| if area_of_polygon_2d(&c.verts) < 0 { -1 } else { 1 }).collect();
        if winding.iter().any(|&w| w < 0) {
            let nregions = chf.max_regions as usize + 1;
            let mut outlines: Vec<Option<usize>> = vec![None; nregions];
            let mut holes: Vec<Vec<usize>> = vec![Vec::new(); nregions];
            for (i, c) in cset.conts.iter().enumerate() {
                if winding[i] > 0 {
                    outlines[c.reg as usize] = Some(i);
                } else {
                    holes[c.reg as usize].push(i);
                }
            }
            for r in 0..nregions {
                if holes[r].is_empty() {
                    continue;
                }
                if let Some(outline) = outlines[r] {
                    merge_region_holes(&mut cset.conts, outline, &holes[r]);
                }
            }
        }
    }
    cset
}
