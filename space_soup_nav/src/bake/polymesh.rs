//! Contours to convex polygons: ear-clipping triangulation, greedy merging into polygons of up to
//! `nvp` vertices, removal of the extra vertices along tile borders, and adjacency. A port of
//! Recast's `rcBuildPolyMesh` (`RecastMesh.cpp`; zlib licence, Copyright (c) 2009-2010 Mikko
//! Mononen; altered: written in Rust).
//!
//! Kept on flat arrays laid out as Recast's (`polys` is `npolys * nvp * 2`: vertex indices then
//! neighbour words), so each loop maps onto its C++ original line for line.

use super::contour::{ContourSet, BORDER_VERTEX};

pub const MESH_NULL_IDX: u16 = 0xffff;
/// Region id of a polygon made from more than one region.
pub const MULTIPLE_REGS: u16 = 0;
const VERTEX_BUCKET_COUNT: usize = 1 << 12;

#[derive(Debug, Clone, PartialEq)]
pub struct PolyMesh {
    /// Cell coordinates.
    pub verts: Vec<[u16; 3]>,
    /// Per polygon: `nvp` vertex indices padded with [`MESH_NULL_IDX`], then `nvp` neighbour
    /// words (polygon index, or `0x8000 | dir` on a tile border, or [`MESH_NULL_IDX`]).
    pub polys: Vec<u16>,
    pub regs: Vec<u16>,
    pub flags: Vec<u16>,
    pub areas: Vec<u8>,
    pub nvp: usize,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    pub border_size: i32,
    pub max_edge_error: f32,
}

impl PolyMesh {
    pub fn npolys(&self) -> usize {
        self.regs.len()
    }

    /// Polygon `i`'s vertex indices, without padding.
    pub fn poly_verts(&self, i: usize) -> &[u16] {
        let p = &self.polys[i * self.nvp * 2..i * self.nvp * 2 + self.nvp];
        &p[..count_poly_verts(p)]
    }

    /// Polygon `i`'s neighbour words, one per edge.
    pub fn poly_neis(&self, i: usize) -> &[u16] {
        let n = self.poly_verts(i).len();
        &self.polys[i * self.nvp * 2 + self.nvp..i * self.nvp * 2 + self.nvp + n]
    }
}

fn count_poly_verts(p: &[u16]) -> usize {
    p.iter().position(|&v| v == MESH_NULL_IDX).unwrap_or(p.len())
}

#[inline]
fn prev(i: usize, n: usize) -> usize {
    if i >= 1 { i - 1 } else { n - 1 }
}
#[inline]
fn next(i: usize, n: usize) -> usize {
    if i + 1 < n { i + 1 } else { 0 }
}

/// Points are `[x, y, z, _]`; only x and z are used.
type P = [i32; 4];

#[inline]
fn twice_area(a: &P, b: &P, c: &P) -> i32 {
    (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])
}
#[inline]
fn left(a: &P, b: &P, c: &P) -> bool {
    twice_area(a, b, c) < 0
}
#[inline]
fn left_on(a: &P, b: &P, c: &P) -> bool {
    twice_area(a, b, c) <= 0
}
#[inline]
fn collinear(a: &P, b: &P, c: &P) -> bool {
    twice_area(a, b, c) == 0
}
fn intersect_prop(a: &P, b: &P, c: &P, d: &P) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) ^ left(a, b, d)) && (left(c, d, a) ^ left(c, d, b))
}
fn between(a: &P, b: &P, c: &P) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a[0] != b[0] {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    } else {
        (a[2] <= c[2] && c[2] <= b[2]) || (a[2] >= c[2] && c[2] >= b[2])
    }
}
fn intersect(a: &P, b: &P, c: &P, d: &P) -> bool {
    intersect_prop(a, b, c, d) || between(a, b, c) || between(a, b, d) || between(c, d, a) || between(c, d, b)
}
fn vequal(a: &P, b: &P) -> bool {
    a[0] == b[0] && a[2] == b[2]
}

const IDX: u32 = 0x0fff_ffff;
const EAR: u32 = 0x8000_0000;

#[inline]
fn pt<'a>(verts: &'a [P], indices: &[u32], i: usize) -> &'a P {
    &verts[(indices[i] & IDX) as usize]
}

fn diagonalie(i: usize, j: usize, n: usize, verts: &[P], indices: &[u32], strict: bool) -> bool {
    let d0 = pt(verts, indices, i);
    let d1 = pt(verts, indices, j);
    for k in 0..n {
        let k1 = next(k, n);
        if k == i || k1 == i || k == j || k1 == j {
            continue;
        }
        let p0 = pt(verts, indices, k);
        let p1 = pt(verts, indices, k1);
        if vequal(d0, p0) || vequal(d1, p0) || vequal(d0, p1) || vequal(d1, p1) {
            continue;
        }
        let crosses = if strict { intersect(d0, d1, p0, p1) } else { intersect_prop(d0, d1, p0, p1) };
        if crosses {
            return false;
        }
    }
    true
}

fn in_cone(i: usize, j: usize, n: usize, verts: &[P], indices: &[u32], strict: bool) -> bool {
    let pi = pt(verts, indices, i);
    let pj = pt(verts, indices, j);
    let pi1 = pt(verts, indices, next(i, n));
    let next_pin = pt(verts, indices, prev(i, n));
    if left_on(next_pin, pi, pi1) {
        return if strict { left(pi, pj, next_pin) && left(pj, pi, pi1) } else { left_on(pi, pj, next_pin) && left_on(pj, pi, pi1) };
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, next_pin))
}

fn diagonal(i: usize, j: usize, n: usize, verts: &[P], indices: &[u32]) -> bool {
    in_cone(i, j, n, verts, indices, true) && diagonalie(i, j, n, verts, indices, true)
}

fn diagonal_loose(i: usize, j: usize, n: usize, verts: &[P], indices: &[u32]) -> bool {
    in_cone(i, j, n, verts, indices, false) && diagonalie(i, j, n, verts, indices, false)
}

/// Ear clipping, shortest ear first. Returns the triangle count, negated when it had to give
/// up part way (the triangles so far are still written).
fn triangulate(mut n: usize, verts: &[P], indices: &mut [u32], tris: &mut Vec<[u32; 3]>) -> i32 {
    tris.clear();
    for i in 0..n {
        let i1 = next(i, n);
        let i2 = next(i1, n);
        if diagonal(i, i2, n, verts, indices) {
            indices[i1] |= EAR;
        }
    }
    while n > 3 {
        let mut min_len = -1i32;
        let mut mini = usize::MAX;
        for i in 0..n {
            let i1 = next(i, n);
            if indices[i1] & EAR != 0 {
                let p0 = pt(verts, indices, i);
                let p2 = pt(verts, indices, next(i1, n));
                let (dx, dz) = (p2[0] - p0[0], p2[2] - p0[2]);
                let len = dx * dx + dz * dz;
                if min_len < 0 || len < min_len {
                    min_len = len;
                    mini = i;
                }
            }
        }
        if mini == usize::MAX {
            min_len = -1;
            for i in 0..n {
                let i1 = next(i, n);
                let i2 = next(i1, n);
                if diagonal_loose(i, i2, n, verts, indices) {
                    let p0 = pt(verts, indices, i);
                    let p2 = pt(verts, indices, next(i2, n));
                    let (dx, dz) = (p2[0] - p0[0], p2[2] - p0[2]);
                    let len = dx * dx + dz * dz;
                    if min_len < 0 || len < min_len {
                        min_len = len;
                        mini = i;
                    }
                }
            }
            if mini == usize::MAX {
                return -(tris.len() as i32);
            }
        }
        let i = mini;
        let mut i1 = next(i, n);
        let i2 = next(i1, n);
        tris.push([indices[i] & IDX, indices[i1] & IDX, indices[i2] & IDX]);
        n -= 1;
        for k in i1..n {
            indices[k] = indices[k + 1];
        }
        if i1 >= n {
            i1 = 0;
        }
        let i = prev(i1, n);
        if diagonal(prev(i, n), i1, n, verts, indices) {
            indices[i] |= EAR;
        } else {
            indices[i] &= IDX;
        }
        if diagonal(i, next(i1, n), n, verts, indices) {
            indices[i1] |= EAR;
        } else {
            indices[i1] &= IDX;
        }
    }
    tris.push([indices[0] & IDX, indices[1] & IDX, indices[2] & IDX]);
    tris.len() as i32
}

fn vertex_hash(x: i32, y: i32, z: i32) -> usize {
    let (h1, h2, h3) = (0x8da6_b343u32, 0xd816_3841u32, 0xcb1a_b31fu32);
    let n = h1.wrapping_mul(x as u32).wrapping_add(h2.wrapping_mul(y as u32)).wrapping_add(h3.wrapping_mul(z as u32));
    (n as usize) & (VERTEX_BUCKET_COUNT - 1)
}

/// `addVertex`: shares a vertex with any already added at the same x, z and within 2 cells of y.
fn add_vertex(x: u16, y: u16, z: u16, verts: &mut Vec<[u16; 3]>, first: &mut [i32], next_vert: &mut Vec<i32>) -> u16 {
    let bucket = vertex_hash(x as i32, 0, z as i32);
    let mut i = first[bucket];
    while i != -1 {
        let v = verts[i as usize];
        if v[0] == x && (v[1] as i32 - y as i32).abs() <= 2 && v[2] == z {
            return i as u16;
        }
        i = next_vert[i as usize];
    }
    let i = verts.len();
    verts.push([x, y, z]);
    next_vert.push(first[bucket]);
    first[bucket] = i as i32;
    i as u16
}

fn uleft(a: [u16; 3], b: [u16; 3], c: [u16; 3]) -> bool {
    (b[0] as i32 - a[0] as i32) * (c[2] as i32 - a[2] as i32) - (c[0] as i32 - a[0] as i32) * (b[2] as i32 - a[2] as i32) < 0
}

/// `getPolyMergeValue`: the squared length of the shared edge if merging keeps the polygon
/// convex and within `nvp` vertices, else -1.
fn poly_merge_value(pa: &[u16], pb: &[u16], verts: &[[u16; 3]], nvp: usize) -> (i32, usize, usize) {
    let na = count_poly_verts(pa);
    let nb = count_poly_verts(pb);
    if na + nb - 2 > nvp {
        return (-1, 0, 0);
    }
    let (mut ea, mut eb) = (usize::MAX, usize::MAX);
    for i in 0..na {
        let (mut va0, mut va1) = (pa[i], pa[(i + 1) % na]);
        if va0 > va1 {
            std::mem::swap(&mut va0, &mut va1);
        }
        for j in 0..nb {
            let (mut vb0, mut vb1) = (pb[j], pb[(j + 1) % nb]);
            if vb0 > vb1 {
                std::mem::swap(&mut vb0, &mut vb1);
            }
            if va0 == vb0 && va1 == vb1 {
                ea = i;
                eb = j;
                break;
            }
        }
    }
    if ea == usize::MAX || eb == usize::MAX {
        return (-1, 0, 0);
    }
    let v = |k: u16| verts[k as usize];
    if !uleft(v(pa[(ea + na - 1) % na]), v(pa[ea]), v(pb[(eb + 2) % nb])) {
        return (-1, 0, 0);
    }
    if !uleft(v(pb[(eb + nb - 1) % nb]), v(pb[eb]), v(pa[(ea + 2) % na])) {
        return (-1, 0, 0);
    }
    let (a, b) = (v(pa[ea]), v(pa[(ea + 1) % na]));
    let dx = a[0] as i32 - b[0] as i32;
    let dz = a[2] as i32 - b[2] as i32;
    (dx * dx + dz * dz, ea, eb)
}

fn merge_poly_verts(polys: &mut [u16], a: usize, b: usize, ea: usize, eb: usize, nvp: usize) {
    let pa: Vec<u16> = polys[a * nvp..a * nvp + nvp].to_vec();
    let pb: Vec<u16> = polys[b * nvp..b * nvp + nvp].to_vec();
    let na = count_poly_verts(&pa);
    let nb = count_poly_verts(&pb);
    let mut scratch = vec![MESH_NULL_IDX; nvp];
    let mut n = 0;
    for i in 0..na - 1 {
        scratch[n] = pa[(ea + 1 + i) % na];
        n += 1;
    }
    for i in 0..nb - 1 {
        scratch[n] = pb[(eb + 1 + i) % nb];
        n += 1;
    }
    polys[a * nvp..a * nvp + nvp].copy_from_slice(&scratch);
}

/// The greedy merge loop shared by the build and by vertex removal. `polys` holds `npolys`
/// polygons of `nvp` entries; `on_merge(pa, pb)` runs before `pb` is overwritten by the last.
fn merge_polys(polys: &mut [u16], npolys: &mut usize, verts: &[[u16; 3]], nvp: usize, mut on_merge: impl FnMut(usize, usize, usize)) {
    let mut vert_polys: Vec<Vec<usize>> = vec![Vec::new(); verts.len()];
    let mut candidates: Vec<usize> = Vec::new();
    loop {
        for list in vert_polys.iter_mut() {
            list.clear();
        }
        for p in 0..*npolys {
            for &v in &polys[p * nvp..p * nvp + nvp] {
                if v == MESH_NULL_IDX {
                    break;
                }
                vert_polys[v as usize].push(p);
            }
        }
        let mut best = 0;
        let (mut bpa, mut bpb, mut bea, mut beb) = (0, 0, 0, 0);
        for j in 0..npolys.saturating_sub(1) {
            candidates.clear();
            for &v in &polys[j * nvp..j * nvp + nvp] {
                if v == MESH_NULL_IDX {
                    break;
                }
                candidates.extend(vert_polys[v as usize].iter().copied().filter(|&k| k > j));
            }
            candidates.sort_unstable();
            candidates.dedup();
            for &k in &candidates {
                let (v, ea, eb) = poly_merge_value(&polys[j * nvp..j * nvp + nvp], &polys[k * nvp..k * nvp + nvp], verts, nvp);
                if v > best {
                    best = v;
                    bpa = j;
                    bpb = k;
                    bea = ea;
                    beb = eb;
                }
            }
        }
        if best <= 0 {
            break;
        }
        merge_poly_verts(polys, bpa, bpb, bea, beb, nvp);
        let last = *npolys - 1;
        on_merge(bpa, bpb, last);
        if bpb != last {
            polys.copy_within(last * nvp..last * nvp + nvp, bpb * nvp);
        }
        *npolys -= 1;
    }
}

fn can_remove_vertex(mesh: &PolyMesh, rem: u16) -> bool {
    let nvp = mesh.nvp;
    let mut touched = 0;
    let mut remaining_edges = 0;
    for i in 0..mesh.npolys() {
        let p = &mesh.polys[i * nvp * 2..i * nvp * 2 + nvp];
        let nv = count_poly_verts(p);
        let removed = p[..nv].iter().filter(|&&v| v == rem).count();
        touched += removed;
        if removed > 0 {
            remaining_edges += nv as i32 - (removed as i32 + 1);
        }
    }
    if remaining_edges <= 2 {
        return false;
    }
    let mut edges: Vec<[i32; 3]> = Vec::with_capacity(touched * 2);
    for i in 0..mesh.npolys() {
        let p = &mesh.polys[i * nvp * 2..i * nvp * 2 + nvp];
        let nv = count_poly_verts(p);
        let mut k = nv - 1;
        for j in 0..nv {
            if p[j] == rem || p[k] == rem {
                let (mut a, mut b) = (p[j] as i32, p[k] as i32);
                if b == rem as i32 {
                    std::mem::swap(&mut a, &mut b);
                }
                let mut exists = false;
                for e in edges.iter_mut() {
                    if e[1] == b {
                        e[2] += 1;
                        exists = true;
                    }
                }
                if !exists {
                    edges.push([a, b, 1]);
                }
            }
            k = j;
        }
    }
    edges.iter().filter(|e| e[2] < 2).count() <= 2
}

fn push_front(v: i32, entries: &mut Vec<i32>) {
    entries.insert(0, v);
}

fn remove_vertex(mesh: &mut PolyMesh, rem: u16, max_tris: usize) {
    let nvp = mesh.nvp;
    let mut edges: Vec<[i32; 4]> = Vec::new();
    let mut i = 0usize;
    while i < mesh.npolys() {
        let base = i * nvp * 2;
        let nv = count_poly_verts(&mesh.polys[base..base + nvp]);
        if mesh.polys[base..base + nv].contains(&rem) {
            let mut k = nv - 1;
            for j in 0..nv {
                let (pj, pk) = (mesh.polys[base + j], mesh.polys[base + k]);
                if pj != rem && pk != rem {
                    edges.push([pk as i32, pj as i32, mesh.regs[i] as i32, mesh.areas[i] as i32]);
                }
                k = j;
            }
            let last = mesh.npolys() - 1;
            if i != last {
                mesh.polys.copy_within(last * nvp * 2..last * nvp * 2 + nvp, base);
            }
            for v in &mut mesh.polys[base + nvp..base + 2 * nvp] {
                *v = MESH_NULL_IDX;
            }
            mesh.regs[i] = mesh.regs[last];
            mesh.areas[i] = mesh.areas[last];
            mesh.regs.pop();
            mesh.areas.pop();
            mesh.polys.truncate(last * nvp * 2);
            continue;
        }
        i += 1;
    }

    mesh.verts.remove(rem as usize);
    for i in 0..mesh.npolys() {
        let base = i * nvp * 2;
        let nv = count_poly_verts(&mesh.polys[base..base + nvp]);
        for v in &mut mesh.polys[base..base + nv] {
            if *v > rem {
                *v -= 1;
            }
        }
    }
    for e in edges.iter_mut() {
        if e[0] > rem as i32 {
            e[0] -= 1;
        }
        if e[1] > rem as i32 {
            e[1] -= 1;
        }
    }
    if edges.is_empty() {
        return;
    }

    let mut hole = vec![edges[0][0]];
    let mut hreg = vec![edges[0][2]];
    let mut harea = vec![edges[0][3]];
    while !edges.is_empty() {
        let mut matched = false;
        let mut i = 0usize;
        while i < edges.len() {
            let [ea, eb, r, a] = edges[i];
            let mut add = false;
            if hole[0] == eb {
                push_front(ea, &mut hole);
                push_front(r, &mut hreg);
                push_front(a, &mut harea);
                add = true;
            } else if *hole.last().unwrap() == ea {
                hole.push(eb);
                hreg.push(r);
                harea.push(a);
                add = true;
            }
            if add {
                edges.swap_remove(i);
                matched = true;
                continue;
            }
            i += 1;
        }
        if !matched {
            break;
        }
    }

    let nhole = hole.len();
    let tverts: Vec<P> = hole
        .iter()
        .map(|&pi| {
            let v = mesh.verts[pi as usize];
            [v[0] as i32, v[1] as i32, v[2] as i32, 0]
        })
        .collect();
    let mut thole: Vec<u32> = (0..nhole as u32).collect();
    let mut tris = Vec::new();
    triangulate(nhole, &tverts, &mut thole, &mut tris);

    let mut polys: Vec<u16> = Vec::new();
    let mut pregs: Vec<u16> = Vec::new();
    let mut pareas: Vec<u8> = Vec::new();
    for t in &tris {
        let t = [t[0] as usize, t[1] as usize, t[2] as usize];
        if t[0] != t[1] && t[0] != t[2] && t[1] != t[2] {
            let mut p = vec![MESH_NULL_IDX; nvp];
            p[0] = hole[t[0]] as u16;
            p[1] = hole[t[1]] as u16;
            p[2] = hole[t[2]] as u16;
            polys.extend_from_slice(&p);
            pregs.push(if hreg[t[0]] != hreg[t[1]] || hreg[t[1]] != hreg[t[2]] { MULTIPLE_REGS } else { hreg[t[0]] as u16 });
            pareas.push(harea[t[0]] as u8);
        }
    }
    let mut npolys = pregs.len();
    if npolys == 0 {
        return;
    }
    if nvp > 3 {
        merge_polys(&mut polys, &mut npolys, &mesh.verts, nvp, |pa, pb, last| {
            if pregs[pa] != pregs[pb] {
                pregs[pa] = MULTIPLE_REGS;
            }
            pregs[pb] = pregs[last];
            pareas[pb] = pareas[last];
        });
    }
    for i in 0..npolys {
        if mesh.npolys() >= max_tris {
            break;
        }
        mesh.polys.extend_from_slice(&polys[i * nvp..i * nvp + nvp]);
        mesh.polys.extend(std::iter::repeat_n(MESH_NULL_IDX, nvp));
        mesh.regs.push(pregs[i]);
        mesh.areas.push(pareas[i]);
    }
}

/// `buildMeshAdjacency`: fills the neighbour words of shared edges.
fn build_mesh_adjacency(polys: &mut [u16], npolys: usize, nverts: usize, nvp: usize) {
    struct Edge {
        vert: [u16; 2],
        poly_edge: [u16; 2],
        poly: [u16; 2],
    }
    let mut first_edge = vec![MESH_NULL_IDX; nverts];
    let mut next_edge: Vec<u16> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let edge_of = |t: &[u16], j: usize| -> (u16, u16) {
        let v0 = t[j];
        let v1 = if j + 1 >= nvp || t[j + 1] == MESH_NULL_IDX { t[0] } else { t[j + 1] };
        (v0, v1)
    };
    for i in 0..npolys {
        let t = &polys[i * nvp * 2..i * nvp * 2 + nvp];
        for j in 0..nvp {
            if t[j] == MESH_NULL_IDX {
                break;
            }
            let (v0, v1) = edge_of(t, j);
            if v0 < v1 {
                edges.push(Edge { vert: [v0, v1], poly: [i as u16, i as u16], poly_edge: [j as u16, 0] });
                next_edge.push(first_edge[v0 as usize]);
                first_edge[v0 as usize] = (edges.len() - 1) as u16;
            }
        }
    }
    for i in 0..npolys {
        let t = &polys[i * nvp * 2..i * nvp * 2 + nvp];
        for j in 0..nvp {
            if t[j] == MESH_NULL_IDX {
                break;
            }
            let (v0, v1) = edge_of(t, j);
            if v0 > v1 {
                let mut e = first_edge[v1 as usize];
                while e != MESH_NULL_IDX {
                    let edge = &mut edges[e as usize];
                    if edge.vert[1] == v0 && edge.poly[0] == edge.poly[1] {
                        edge.poly[1] = i as u16;
                        edge.poly_edge[1] = j as u16;
                        break;
                    }
                    e = next_edge[e as usize];
                }
            }
        }
    }
    for e in &edges {
        if e.poly[0] != e.poly[1] {
            polys[e.poly[0] as usize * nvp * 2 + nvp + e.poly_edge[0] as usize] = e.poly[1];
            polys[e.poly[1] as usize * nvp * 2 + nvp + e.poly_edge[1] as usize] = e.poly[0];
        }
    }
}

/// `rcBuildPolyMesh`.
pub fn build_poly_mesh(cset: &ContourSet, nvp: usize) -> PolyMesh {
    let mut mesh = PolyMesh {
        verts: Vec::new(),
        polys: Vec::new(),
        regs: Vec::new(),
        flags: Vec::new(),
        areas: Vec::new(),
        nvp,
        bmin: cset.bmin,
        bmax: cset.bmax,
        cs: cset.cs,
        ch: cset.ch,
        border_size: cset.border_size,
        max_edge_error: cset.max_error,
    };
    let mut max_tris = 0;
    for c in &cset.conts {
        if c.verts.len() >= 3 {
            max_tris += c.verts.len() - 2;
        }
    }
    let mut vflags: Vec<u8> = Vec::new();
    let mut first_vert = vec![-1i32; VERTEX_BUCKET_COUNT];
    let mut next_vert: Vec<i32> = Vec::new();
    let mut tris = Vec::new();

    for cont in &cset.conts {
        if cont.verts.len() < 3 {
            continue;
        }
        let nv = cont.verts.len();
        let mut indices: Vec<u32> = (0..nv as u32).collect();
        let ntris = triangulate(nv, &cont.verts, &mut indices, &mut tris).unsigned_abs() as usize;
        let mut vidx = vec![0u16; nv];
        for (j, v) in cont.verts.iter().enumerate() {
            vidx[j] = add_vertex(v[0] as u16, v[1] as u16, v[2] as u16, &mut mesh.verts, &mut first_vert, &mut next_vert);
            if vflags.len() < mesh.verts.len() {
                vflags.resize(mesh.verts.len(), 0);
            }
            if v[3] & BORDER_VERTEX != 0 {
                vflags[vidx[j] as usize] = 1;
            }
        }
        let mut polys: Vec<u16> = Vec::new();
        for t in &tris[..ntris] {
            let t = [t[0] as usize, t[1] as usize, t[2] as usize];
            if t[0] != t[1] && t[0] != t[2] && t[1] != t[2] {
                let mut p = vec![MESH_NULL_IDX; nvp];
                p[0] = vidx[t[0]];
                p[1] = vidx[t[1]];
                p[2] = vidx[t[2]];
                polys.extend_from_slice(&p);
            }
        }
        let mut npolys = polys.len() / nvp;
        if npolys == 0 {
            continue;
        }
        if nvp > 3 {
            merge_polys(&mut polys, &mut npolys, &mesh.verts, nvp, |_, _, _| {});
        }
        for j in 0..npolys {
            mesh.polys.extend_from_slice(&polys[j * nvp..j * nvp + nvp]);
            mesh.polys.extend(std::iter::repeat_n(MESH_NULL_IDX, nvp));
            mesh.regs.push(cont.reg);
            mesh.areas.push(cont.area);
        }
    }

    vflags.resize(mesh.verts.len() + 1, 0);
    let mut i = 0usize;
    while i < mesh.verts.len() {
        if vflags[i] != 0 {
            if !can_remove_vertex(&mesh, i as u16) {
                i += 1;
                continue;
            }
            remove_vertex(&mut mesh, i as u16, max_tris);
            vflags.remove(i);
            continue;
        }
        i += 1;
    }

    let npolys = mesh.npolys();
    build_mesh_adjacency(&mut mesh.polys, npolys, mesh.verts.len(), nvp);

    if mesh.border_size > 0 {
        let (w, h) = (cset.width, cset.height);
        for i in 0..npolys {
            let base = i * nvp * 2;
            for j in 0..nvp {
                if mesh.polys[base + j] == MESH_NULL_IDX {
                    break;
                }
                if mesh.polys[base + nvp + j] != MESH_NULL_IDX {
                    continue;
                }
                let mut nj = j + 1;
                if nj >= nvp || mesh.polys[base + nj] == MESH_NULL_IDX {
                    nj = 0;
                }
                let va = mesh.verts[mesh.polys[base + j] as usize];
                let vb = mesh.verts[mesh.polys[base + nj] as usize];
                let word = if va[0] == 0 && vb[0] == 0 {
                    Some(0)
                } else if va[2] as i32 == h && vb[2] as i32 == h {
                    Some(1)
                } else if va[0] as i32 == w && vb[0] as i32 == w {
                    Some(2)
                } else if va[2] == 0 && vb[2] == 0 {
                    Some(3)
                } else {
                    None
                };
                if let Some(d) = word {
                    mesh.polys[base + nvp + j] = 0x8000 | d;
                }
            }
        }
    }
    mesh.flags = vec![0; npolys];
    mesh
}
