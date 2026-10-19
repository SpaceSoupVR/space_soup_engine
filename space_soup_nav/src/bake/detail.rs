//! The detail mesh: per polygon, a small triangle mesh that follows the real floor height
//! (stairs, ramps) within `sample_max_error`, sampled from the compact heightfield. A port of
//! Recast's `rcBuildPolyMeshDetail` (`RecastMeshDetail.cpp`; zlib licence, Copyright (c)
//! 2009-2010 Mikko Mononen; altered: written in Rust). Float expressions keep the C++ order.

use super::compact::{CompactHeightfield, NOT_CONNECTED};
use super::heightfield::{DIR_X, DIR_Z};
use super::polymesh::{PolyMesh, MULTIPLE_REGS};

const UNSET_HEIGHT: u16 = 0xffff;
const MAX_VERTS: usize = 127;
const MAX_TRIS: usize = 255;
const MAX_VERTS_PER_EDGE: usize = 32;
const EV_UNDEF: i32 = -1;
const EV_HULL: i32 = -2;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetailMesh {
    /// Per polygon: vertex base, vertex count, triangle base, triangle count.
    pub meshes: Vec<[u32; 4]>,
    /// World metres.
    pub verts: Vec<[f32; 3]>,
    /// Indices into the polygon's own vertices (polygon vertices first), then edge flags.
    pub tris: Vec<[u8; 4]>,
}

struct HeightPatch {
    data: Vec<u16>,
    xmin: i32,
    ymin: i32,
    width: i32,
    height: i32,
}

type V = [f32; 3];

#[inline]
fn vdot2(a: &V, b: &V) -> f32 {
    a[0] * b[0] + a[2] * b[2]
}
#[inline]
fn vdist_sq2(p: &V, q: &V) -> f32 {
    let dx = q[0] - p[0];
    let dy = q[2] - p[2];
    dx * dx + dy * dy
}
#[inline]
fn vdist2(p: &V, q: &V) -> f32 {
    vdist_sq2(p, q).sqrt()
}
#[inline]
fn vcross2(p1: &V, p2: &V, p3: &V) -> f32 {
    let u1 = p2[0] - p1[0];
    let v1 = p2[2] - p1[2];
    let u2 = p3[0] - p1[0];
    let v2 = p3[2] - p1[2];
    u1 * v2 - v1 * u2
}
#[inline]
fn vsub(a: &V, b: &V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn circum_circle(p1: &V, p2: &V, p3: &V, c: &mut V, r: &mut f32) -> bool {
    const EPS: f32 = 1e-6;
    let v1 = [0.0f32; 3];
    let v2 = vsub(p2, p1);
    let v3 = vsub(p3, p1);
    let cp = vcross2(&v1, &v2, &v3);
    if cp.abs() > EPS {
        let v1sq = vdot2(&v1, &v1);
        let v2sq = vdot2(&v2, &v2);
        let v3sq = vdot2(&v3, &v3);
        c[0] = (v1sq * (v2[2] - v3[2]) + v2sq * (v3[2] - v1[2]) + v3sq * (v1[2] - v2[2])) / (2.0 * cp);
        c[1] = 0.0;
        c[2] = (v1sq * (v3[0] - v2[0]) + v2sq * (v1[0] - v3[0]) + v3sq * (v2[0] - v1[0])) / (2.0 * cp);
        *r = vdist2(c, &v1);
        *c = [c[0] + p1[0], c[1] + p1[1], c[2] + p1[2]];
        return true;
    }
    *c = *p1;
    *r = 0.0;
    false
}

fn dist_pt_tri(p: &V, a: &V, b: &V, c: &V) -> f32 {
    let v0 = vsub(c, a);
    let v1 = vsub(b, a);
    let v2 = vsub(p, a);
    let dot00 = vdot2(&v0, &v0);
    let dot01 = vdot2(&v0, &v1);
    let dot02 = vdot2(&v0, &v2);
    let dot11 = vdot2(&v1, &v1);
    let dot12 = vdot2(&v1, &v2);
    let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;
    const EPS: f32 = 1e-4;
    if u >= -EPS && v >= -EPS && (u + v) <= 1.0 + EPS {
        let y = a[1] + v0[1] * u + v1[1] * v;
        return (y - p[1]).abs();
    }
    f32::MAX
}

fn distance_pt_seg(pt: &V, p: &V, q: &V) -> f32 {
    let pqx = q[0] - p[0];
    let pqy = q[1] - p[1];
    let pqz = q[2] - p[2];
    let mut dx = pt[0] - p[0];
    let mut dy = pt[1] - p[1];
    let mut dz = pt[2] - p[2];
    let d = pqx * pqx + pqy * pqy + pqz * pqz;
    let mut t = pqx * dx + pqy * dy + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    if t < 0.0 {
        t = 0.0;
    } else if t > 1.0 {
        t = 1.0;
    }
    dx = p[0] + t * pqx - pt[0];
    dy = p[1] + t * pqy - pt[1];
    dz = p[2] + t * pqz - pt[2];
    dx * dx + dy * dy + dz * dz
}

fn distance_pt_seg_2d(pt: &V, p: &V, q: &V) -> f32 {
    let pqx = q[0] - p[0];
    let pqz = q[2] - p[2];
    let mut dx = pt[0] - p[0];
    let mut dz = pt[2] - p[2];
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
    dx = p[0] + t * pqx - pt[0];
    dz = p[2] + t * pqz - pt[2];
    dx * dx + dz * dz
}

fn dist_to_tri_mesh(p: &V, verts: &[V], tris: &[[i32; 4]]) -> f32 {
    let mut dmin = f32::MAX;
    for t in tris {
        let d = dist_pt_tri(p, &verts[t[0] as usize], &verts[t[1] as usize], &verts[t[2] as usize]);
        if d < dmin {
            dmin = d;
        }
    }
    if dmin == f32::MAX {
        return -1.0;
    }
    dmin
}

fn dist_to_poly(verts: &[V], p: &V) -> f32 {
    let mut dmin = f32::MAX;
    let mut c = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (&verts[i], &verts[j]);
        if ((vi[2] > p[2]) != (vj[2] > p[2])) && (p[0] < (vj[0] - vi[0]) * (p[2] - vi[2]) / (vj[2] - vi[2]) + vi[0]) {
            c = !c;
        }
        dmin = dmin.min(distance_pt_seg_2d(p, vj, vi));
        j = i;
    }
    if c { -dmin } else { dmin }
}

fn get_height(fx: f32, fy: f32, fz: f32, ics: f32, ch: f32, radius: i32, hp: &HeightPatch) -> u16 {
    let mut ix = (fx * ics + 0.01).floor() as i32;
    let mut iz = (fz * ics + 0.01).floor() as i32;
    ix = (ix - hp.xmin).clamp(0, hp.width - 1);
    iz = (iz - hp.ymin).clamp(0, hp.height - 1);
    let mut h = hp.data[(ix + iz * hp.width) as usize];
    if h == UNSET_HEIGHT {
        let (mut x, mut z, mut dx, mut dz) = (1i32, 0i32, 1i32, 0i32);
        let max_size = radius * 2 + 1;
        let max_iter = max_size * max_size - 1;
        let mut next_ring_iter_start = 8;
        let mut next_ring_iters = 16;
        let mut dmin = f32::MAX;
        for i in 0..max_iter {
            let (nx, nz) = (ix + x, iz + z);
            if nx >= 0 && nz >= 0 && nx < hp.width && nz < hp.height {
                let nh = hp.data[(nx + nz * hp.width) as usize];
                if nh != UNSET_HEIGHT {
                    let d = (nh as f32 * ch - fy).abs();
                    if d < dmin {
                        h = nh;
                        dmin = d;
                    }
                }
            }
            if i + 1 == next_ring_iter_start {
                if h != UNSET_HEIGHT {
                    break;
                }
                next_ring_iter_start += next_ring_iters;
                next_ring_iters += 8;
            }
            if x == z || (x < 0 && x == -z) || (x > 0 && x == 1 - z) {
                let tmp = dx;
                dx = -dz;
                dz = tmp;
            }
            x += dx;
            z += dz;
        }
    }
    h
}

fn find_edge(edges: &[[i32; 4]], s: i32, t: i32) -> i32 {
    edges.iter().position(|e| (e[0] == s && e[1] == t) || (e[0] == t && e[1] == s)).map_or(EV_UNDEF, |i| i as i32)
}

fn add_edge(edges: &mut Vec<[i32; 4]>, max_edges: usize, s: i32, t: i32, l: i32, r: i32) -> i32 {
    if edges.len() >= max_edges {
        return EV_UNDEF;
    }
    if find_edge(edges, s, t) == EV_UNDEF {
        edges.push([s, t, l, r]);
        return edges.len() as i32 - 1;
    }
    EV_UNDEF
}

fn update_left_face(e: &mut [i32; 4], s: i32, t: i32, f: i32) {
    if e[0] == s && e[1] == t && e[2] == EV_UNDEF {
        e[2] = f;
    } else if e[1] == s && e[0] == t && e[3] == EV_UNDEF {
        e[3] = f;
    }
}

fn overlap_seg_seg_2d(a: &V, b: &V, c: &V, d: &V) -> bool {
    let a1 = vcross2(a, b, d);
    let a2 = vcross2(a, b, c);
    if a1 * a2 < 0.0 {
        let a3 = vcross2(c, d, a);
        let a4 = a3 + a2 - a1;
        if a3 * a4 < 0.0 {
            return true;
        }
    }
    false
}

fn overlap_edges(pts: &[V], edges: &[[i32; 4]], s1: i32, t1: i32) -> bool {
    for e in edges {
        let (s0, t0) = (e[0], e[1]);
        if s0 == s1 || s0 == t1 || t0 == s1 || t0 == t1 {
            continue;
        }
        if overlap_seg_seg_2d(&pts[s0 as usize], &pts[t0 as usize], &pts[s1 as usize], &pts[t1 as usize]) {
            return true;
        }
    }
    false
}

fn complete_facet(pts: &[V], edges: &mut Vec<[i32; 4]>, max_edges: usize, nfaces: &mut i32, e: usize) {
    const EPS: f32 = 1e-5;
    let edge = edges[e];
    let (s, t) = if edge[2] == EV_UNDEF {
        (edge[0], edge[1])
    } else if edge[3] == EV_UNDEF {
        (edge[1], edge[0])
    } else {
        return;
    };
    let npts = pts.len() as i32;
    let mut pt = npts;
    let mut c = [0.0f32; 3];
    let mut r = -1.0f32;
    let (ps, pt_) = (&pts[s as usize], &pts[t as usize]);
    for u in 0..npts {
        if u == s || u == t {
            continue;
        }
        let pu = &pts[u as usize];
        if vcross2(ps, pt_, pu) > EPS {
            if r < 0.0 {
                pt = u;
                circum_circle(ps, pt_, pu, &mut c, &mut r);
                continue;
            }
            let d = vdist2(&c, pu);
            let tol = 0.001f32;
            if d > r * (1.0 + tol) {
                continue;
            } else if d < r * (1.0 - tol) {
                pt = u;
                circum_circle(ps, pt_, pu, &mut c, &mut r);
            } else {
                if overlap_edges(pts, edges, s, u) || overlap_edges(pts, edges, t, u) {
                    continue;
                }
                pt = u;
                circum_circle(ps, pt_, pu, &mut c, &mut r);
            }
        }
    }
    if pt < npts {
        update_left_face(&mut edges[e], s, t, *nfaces);
        let e1 = find_edge(edges, pt, s);
        if e1 == EV_UNDEF {
            add_edge(edges, max_edges, pt, s, *nfaces, EV_UNDEF);
        } else {
            update_left_face(&mut edges[e1 as usize], pt, s, *nfaces);
        }
        let e2 = find_edge(edges, t, pt);
        if e2 == EV_UNDEF {
            add_edge(edges, max_edges, t, pt, *nfaces, EV_UNDEF);
        } else {
            update_left_face(&mut edges[e2 as usize], t, pt, *nfaces);
        }
        *nfaces += 1;
    } else {
        update_left_face(&mut edges[e], s, t, EV_HULL);
    }
}

fn delaunay_hull(pts: &[V], hull: &[i32], tris: &mut Vec<[i32; 4]>, edges: &mut Vec<[i32; 4]>) {
    let mut nfaces = 0i32;
    let max_edges = pts.len() * 10;
    edges.clear();
    let nhull = hull.len();
    let mut j = nhull - 1;
    for i in 0..nhull {
        add_edge(edges, max_edges, hull[j], hull[i], EV_HULL, EV_UNDEF);
        j = i;
    }
    let mut current = 0;
    while current < edges.len() {
        if edges[current][2] == EV_UNDEF {
            complete_facet(pts, edges, max_edges, &mut nfaces, current);
        }
        if edges[current][3] == EV_UNDEF {
            complete_facet(pts, edges, max_edges, &mut nfaces, current);
        }
        current += 1;
    }
    tris.clear();
    tris.resize(nfaces as usize, [-1; 4]);
    for e in edges.iter() {
        if e[3] >= 0 {
            let t = &mut tris[e[3] as usize];
            if t[0] == -1 {
                t[0] = e[0];
                t[1] = e[1];
            } else if t[0] == e[1] {
                t[2] = e[0];
            } else if t[1] == e[0] {
                t[2] = e[1];
            }
        }
        if e[2] >= 0 {
            let t = &mut tris[e[2] as usize];
            if t[0] == -1 {
                t[0] = e[1];
                t[1] = e[0];
            } else if t[0] == e[0] {
                t[2] = e[1];
            } else if t[1] == e[1] {
                t[2] = e[0];
            }
        }
    }
    let mut i = 0;
    while i < tris.len() {
        let t = tris[i];
        if t[0] == -1 || t[1] == -1 || t[2] == -1 {
            let last = *tris.last().unwrap();
            tris[i] = last;
            tris.pop();
            continue;
        }
        i += 1;
    }
}

fn poly_min_extent(verts: &[V]) -> f32 {
    let n = verts.len();
    let mut min_dist = f32::MAX;
    for i in 0..n {
        let ni = (i + 1) % n;
        let (p1, p2) = (&verts[i], &verts[ni]);
        let mut max_edge_dist = 0.0f32;
        for (j, v) in verts.iter().enumerate() {
            if j == i || j == ni {
                continue;
            }
            max_edge_dist = max_edge_dist.max(distance_pt_seg_2d(v, p1, p2));
        }
        min_dist = min_dist.min(max_edge_dist);
    }
    min_dist.sqrt()
}

#[inline]
fn prev(i: usize, n: usize) -> usize {
    if i >= 1 { i - 1 } else { n - 1 }
}
#[inline]
fn next(i: usize, n: usize) -> usize {
    if i + 1 < n { i + 1 } else { 0 }
}

fn triangulate_hull(verts: &[V], hull: &[i32], nin: usize, tris: &mut Vec<[i32; 4]>) {
    let nhull = hull.len();
    let (mut start, mut left, mut right) = (0usize, 1usize, nhull - 1);
    let mut dmin = f32::MAX;
    for i in 0..nhull {
        if hull[i] as usize >= nin {
            continue;
        }
        let pi = prev(i, nhull);
        let ni = next(i, nhull);
        let (pv, cv, nv) = (&verts[hull[pi] as usize], &verts[hull[i] as usize], &verts[hull[ni] as usize]);
        let d = vdist2(pv, cv) + vdist2(cv, nv) + vdist2(nv, pv);
        if d < dmin {
            start = i;
            left = ni;
            right = pi;
            dmin = d;
        }
    }
    tris.push([hull[start], hull[left], hull[right], 0]);
    while next(left, nhull) != right {
        let nleft = next(left, nhull);
        let nright = prev(right, nhull);
        let cvleft = &verts[hull[left] as usize];
        let nvleft = &verts[hull[nleft] as usize];
        let cvright = &verts[hull[right] as usize];
        let nvright = &verts[hull[nright] as usize];
        let dleft = vdist2(cvleft, nvleft) + vdist2(nvleft, cvright);
        let dright = vdist2(cvright, nvright) + vdist2(cvleft, nvright);
        if dleft < dright {
            tris.push([hull[left], hull[nleft], hull[right], 0]);
            left = nleft;
        } else {
            tris.push([hull[left], hull[nright], hull[right], 0]);
            right = nright;
        }
    }
}

#[inline]
fn jitter_x(i: usize) -> f32 {
    (((i as u32).wrapping_mul(0x8da6_b343) & 0xffff) as f32 / 65535.0 * 2.0) - 1.0
}
#[inline]
fn jitter_y(i: usize) -> f32 {
    (((i as u32).wrapping_mul(0xd816_3841) & 0xffff) as f32 / 65535.0 * 2.0) - 1.0
}

fn on_hull(a: i32, b: i32, hull: &[i32]) -> bool {
    let nhull = hull.len() as i32;
    if a >= nhull || b >= nhull {
        return false;
    }
    let mut j = hull.len() - 1;
    for i in 0..hull.len() {
        if a == hull[j] && b == hull[i] {
            return true;
        }
        j = i;
    }
    false
}

fn set_tri_flags(tris: &mut [[i32; 4]], hull: &[i32]) {
    for t in tris.iter_mut() {
        let mut flags = 0;
        flags |= on_hull(t[0], t[1], hull) as i32;
        flags |= (on_hull(t[1], t[2], hull) as i32) << 2;
        flags |= (on_hull(t[2], t[0], hull) as i32) << 4;
        t[3] = flags;
    }
}

#[allow(clippy::too_many_arguments)]
fn build_poly_detail(
    input: &[V],
    sample_dist: f32,
    sample_max_error: f32,
    height_search_radius: i32,
    chf: &CompactHeightfield,
    hp: &HeightPatch,
    verts: &mut Vec<V>,
    tris: &mut Vec<[i32; 4]>,
    edges: &mut Vec<[i32; 4]>,
    samples: &mut Vec<[i32; 4]>,
) {
    let nin = input.len();
    let mut hull: Vec<i32> = Vec::with_capacity(MAX_VERTS);
    verts.clear();
    verts.extend_from_slice(input);
    edges.clear();
    tris.clear();
    let cs = chf.cs;
    let ics = 1.0 / cs;
    let min_extent = poly_min_extent(verts);

    if sample_dist > 0.0 {
        let mut edge = [[0.0f32; 3]; MAX_VERTS_PER_EDGE + 1];
        let mut j = nin - 1;
        for i in 0..nin {
            let (mut vj, mut vi) = (&input[j], &input[i]);
            let mut swapped = false;
            if (vj[0] - vi[0]).abs() < 1e-6 {
                if vj[2] > vi[2] {
                    std::mem::swap(&mut vj, &mut vi);
                    swapped = true;
                }
            } else if vj[0] > vi[0] {
                std::mem::swap(&mut vj, &mut vi);
                swapped = true;
            }
            let dx = vi[0] - vj[0];
            let dy = vi[1] - vj[1];
            let dz = vi[2] - vj[2];
            let d = (dx * dx + dz * dz).sqrt();
            let mut nn = 1 + (d / sample_dist).floor() as i32;
            if nn >= MAX_VERTS_PER_EDGE as i32 {
                nn = MAX_VERTS_PER_EDGE as i32 - 1;
            }
            if verts.len() as i32 + nn >= MAX_VERTS as i32 {
                nn = MAX_VERTS as i32 - 1 - verts.len() as i32;
            }
            for k in 0..(nn + 1).max(0) as usize {
                let u = k as f32 / nn as f32;
                let pos = &mut edge[k];
                pos[0] = vj[0] + dx * u;
                pos[1] = vj[1] + dy * u;
                pos[2] = vj[2] + dz * u;
                pos[1] = get_height(pos[0], pos[1], pos[2], ics, chf.ch, height_search_radius, hp) as f32 * chf.ch;
            }
            let mut idx = [0i32; MAX_VERTS_PER_EDGE];
            idx[1] = nn;
            let mut nidx = 2usize;
            let mut k = 0usize;
            while k < nidx - 1 {
                let (a, b) = (idx[k], idx[k + 1]);
                let (va, vb) = (edge[a.max(0) as usize], edge[b.max(0) as usize]);
                let mut maxd = 0.0f32;
                let mut maxi = -1i32;
                for m in a + 1..b {
                    let dev = distance_pt_seg(&edge[m as usize], &va, &vb);
                    if dev > maxd {
                        maxd = dev;
                        maxi = m;
                    }
                }
                if maxi != -1 && maxd > sample_max_error * sample_max_error {
                    for m in (k + 1..=nidx).rev() {
                        idx[m] = idx[m - 1];
                    }
                    idx[k + 1] = maxi;
                    nidx += 1;
                } else {
                    k += 1;
                }
            }
            hull.push(j as i32);
            if swapped {
                for k in (1..nidx - 1).rev() {
                    verts.push(edge[idx[k] as usize]);
                    hull.push(verts.len() as i32 - 1);
                }
            } else {
                for k in 1..nidx - 1 {
                    verts.push(edge[idx[k] as usize]);
                    hull.push(verts.len() as i32 - 1);
                }
            }
            j = i;
        }
    }

    if min_extent < sample_dist * 2.0 {
        triangulate_hull(verts, &hull, nin, tris);
        set_tri_flags(tris, &hull);
        return;
    }
    triangulate_hull(verts, &hull, nin, tris);
    if tris.is_empty() {
        return;
    }

    if sample_dist > 0.0 {
        let mut bmin = input[0];
        let mut bmax = input[0];
        for v in &input[1..] {
            for a in 0..3 {
                bmin[a] = bmin[a].min(v[a]);
                bmax[a] = bmax[a].max(v[a]);
            }
        }
        let x0 = (bmin[0] / sample_dist).floor() as i32;
        let x1 = (bmax[0] / sample_dist).ceil() as i32;
        let z0 = (bmin[2] / sample_dist).floor() as i32;
        let z1 = (bmax[2] / sample_dist).ceil() as i32;
        samples.clear();
        for z in z0..z1 {
            for x in x0..x1 {
                let pt = [x as f32 * sample_dist, (bmax[1] + bmin[1]) * 0.5, z as f32 * sample_dist];
                if dist_to_poly(input, &pt) > -sample_dist / 2.0 {
                    continue;
                }
                let h = get_height(pt[0], pt[1], pt[2], ics, chf.ch, height_search_radius, hp);
                samples.push([x, h as i32, z, 0]);
            }
        }
        let nsamples = samples.len();
        for _ in 0..nsamples {
            if verts.len() >= MAX_VERTS {
                break;
            }
            let mut bestpt = [0.0f32; 3];
            let mut bestd = 0.0f32;
            let mut besti = -1i32;
            for (i, s) in samples.iter().enumerate() {
                if s[3] != 0 {
                    continue;
                }
                let pt = [
                    s[0] as f32 * sample_dist + jitter_x(i) * cs * 0.1,
                    s[1] as f32 * chf.ch,
                    s[2] as f32 * sample_dist + jitter_y(i) * cs * 0.1,
                ];
                if tris.is_empty() {
                    continue;
                }
                let d = dist_to_tri_mesh(&pt, verts, tris);
                if d < 0.0 {
                    continue;
                }
                if d > bestd {
                    bestd = d;
                    besti = i as i32;
                    bestpt = pt;
                }
            }
            if bestd <= sample_max_error || besti == -1 {
                break;
            }
            samples[besti as usize][3] = 1;
            verts.push(bestpt);
            delaunay_hull(verts, &hull, tris, edges);
        }
    }

    if tris.len() > MAX_TRIS {
        tris.truncate(MAX_TRIS);
    }
    set_tri_flags(tris, &hull);
}

fn seed_array_with_poly_center(chf: &CompactHeightfield, poly: &[u16], verts: &[[u16; 3]], bs: i32, hp: &mut HeightPatch, array: &mut Vec<i32>) {
    const OFFSET: [i32; 18] = [0, 0, -1, -1, 0, -1, 1, -1, 1, 0, 1, 1, 0, 1, -1, 1, -1, 0];
    let (mut start_x, mut start_z, mut start_i) = (0i32, 0i32, -1i32);
    let mut dmin = UNSET_HEIGHT as i32;
    'outer: for &pv in poly {
        if dmin <= 0 {
            break;
        }
        for k in 0..9 {
            if dmin <= 0 {
                break 'outer;
            }
            let v = verts[pv as usize];
            let ax = v[0] as i32 + OFFSET[k * 2];
            let ay = v[1] as i32;
            let az = v[2] as i32 + OFFSET[k * 2 + 1];
            if ax < hp.xmin || ax >= hp.xmin + hp.width || az < hp.ymin || az >= hp.ymin + hp.height {
                continue;
            }
            let c = chf.cells[((ax + bs) + (az + bs) * chf.width) as usize];
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                if dmin <= 0 {
                    break;
                }
                let d = (ay - chf.spans[i].y as i32).abs();
                if d < dmin {
                    start_x = ax;
                    start_z = az;
                    start_i = i as i32;
                    dmin = d;
                }
            }
        }
    }
    let (mut pcx, mut pcz) = (0i32, 0i32);
    for &pv in poly {
        pcx += verts[pv as usize][0] as i32;
        pcz += verts[pv as usize][2] as i32;
    }
    pcx /= poly.len() as i32;
    pcz /= poly.len() as i32;

    array.clear();
    array.extend_from_slice(&[start_x, start_z, start_i]);
    let mut dirs = [0usize, 1, 2, 3];
    for d in hp.data.iter_mut() {
        *d = 0;
    }
    let (mut cx, mut cz, mut ci) = (-1i32, -1i32, -1i32);
    loop {
        if array.len() < 3 {
            break;
        }
        ci = array.pop().unwrap();
        cz = array.pop().unwrap();
        cx = array.pop().unwrap();
        if cx == pcx && cz == pcz {
            break;
        }
        let direct = if cx == pcx { dir_for_offset(0, if pcz > cz { 1 } else { -1 }) } else { dir_for_offset(if pcx > cx { 1 } else { -1 }, 0) };
        dirs.swap(direct, 3);
        let cspan = chf.spans[ci as usize];
        for &dir in &dirs {
            if cspan.con[dir] == NOT_CONNECTED {
                continue;
            }
            let nx = cx + DIR_X[dir];
            let nz = cz + DIR_Z[dir];
            let (hpx, hpz) = (nx - hp.xmin, nz - hp.ymin);
            if hpx < 0 || hpx >= hp.width || hpz < 0 || hpz >= hp.height {
                continue;
            }
            let k = (hpx + hpz * hp.width) as usize;
            if hp.data[k] != 0 {
                continue;
            }
            hp.data[k] = 1;
            array.extend_from_slice(&[nx, nz, chf.cells[((nx + bs) + (nz + bs) * chf.width) as usize].index as i32 + cspan.con[dir] as i32]);
        }
        dirs.swap(direct, 3);
    }
    array.clear();
    array.extend_from_slice(&[cx + bs, cz + bs, ci]);
    for d in hp.data.iter_mut() {
        *d = UNSET_HEIGHT;
    }
    if ci >= 0 {
        let k = (cx - hp.xmin + (cz - hp.ymin) * hp.width) as usize;
        if k < hp.data.len() {
            hp.data[k] = chf.spans[ci as usize].y;
        }
    }
}

fn dir_for_offset(x: i32, z: i32) -> usize {
    const DIRS: [i32; 5] = [3, 0, -1, 2, 1];
    DIRS[(((z + 1) << 1) + x) as usize] as usize
}

#[allow(clippy::too_many_arguments)]
fn get_height_data(chf: &CompactHeightfield, poly: &[u16], verts: &[[u16; 3]], bs: i32, hp: &mut HeightPatch, queue: &mut Vec<i32>, region: u16) {
    queue.clear();
    for d in hp.data.iter_mut() {
        *d = UNSET_HEIGHT;
    }
    let mut empty = true;
    if region != MULTIPLE_REGS {
        for hy in 0..hp.height {
            let y = hp.ymin + hy + bs;
            for hx in 0..hp.width {
                let x = hp.xmin + hx + bs;
                let c = chf.cells[(x + y * chf.width) as usize];
                for i in c.index as usize..(c.index + c.count as u32) as usize {
                    let s = chf.spans[i];
                    if s.reg == region {
                        hp.data[(hx + hy * hp.width) as usize] = s.y;
                        empty = false;
                        let mut border = false;
                        for dir in 0..4 {
                            if s.con[dir] != NOT_CONNECTED {
                                let ai = chf.neighbour(x, y, dir, s.con[dir]);
                                if chf.spans[ai].reg != region {
                                    border = true;
                                    break;
                                }
                            }
                        }
                        if border {
                            queue.extend_from_slice(&[x, y, i as i32]);
                        }
                        break;
                    }
                }
            }
        }
    }
    if empty {
        seed_array_with_poly_center(chf, poly, verts, bs, hp, queue);
    }
    let mut head = 0usize;
    while head * 3 < queue.len() {
        let (cx, cy, ci) = (queue[head * 3], queue[head * 3 + 1], queue[head * 3 + 2]);
        head += 1;
        let cspan = chf.spans[ci as usize];
        for dir in 0..4 {
            if cspan.con[dir] == NOT_CONNECTED {
                continue;
            }
            let ax = cx + DIR_X[dir];
            let ay = cy + DIR_Z[dir];
            let hx = ax - hp.xmin - bs;
            let hy = ay - hp.ymin - bs;
            if hx < 0 || hx >= hp.width || hy < 0 || hy >= hp.height {
                continue;
            }
            let k = (hx + hy * hp.width) as usize;
            if hp.data[k] != UNSET_HEIGHT {
                continue;
            }
            let ai = chf.cells[(ax + ay * chf.width) as usize].index as usize + cspan.con[dir] as usize;
            hp.data[k] = chf.spans[ai].y;
            queue.extend_from_slice(&[ax, ay, ai as i32]);
        }
    }
}

/// `rcBuildPolyMeshDetail`.
pub fn build_poly_mesh_detail(mesh: &PolyMesh, chf: &CompactHeightfield, sample_dist: f32, sample_max_error: f32) -> DetailMesh {
    let mut dmesh = DetailMesh::default();
    if mesh.verts.is_empty() || mesh.npolys() == 0 {
        return dmesh;
    }
    let (cs, ch) = (mesh.cs, mesh.ch);
    let orig = mesh.bmin;
    let bs = mesh.border_size;
    let height_search_radius = 1.max(mesh.max_edge_error.ceil() as i32);

    let mut bounds: Vec<[i32; 4]> = Vec::with_capacity(mesh.npolys());
    for i in 0..mesh.npolys() {
        let (mut xmin, mut xmax, mut ymin, mut ymax) = (chf.width, 0, chf.height, 0);
        for &v in mesh.poly_verts(i) {
            let v = mesh.verts[v as usize];
            xmin = xmin.min(v[0] as i32);
            xmax = xmax.max(v[0] as i32);
            ymin = ymin.min(v[2] as i32);
            ymax = ymax.max(v[2] as i32);
        }
        bounds.push([0.max(xmin - 1), chf.width.min(xmax + 1), 0.max(ymin - 1), chf.height.min(ymax + 1)]);
    }

    let mut edges = Vec::new();
    let mut tris = Vec::new();
    let mut arr = Vec::new();
    let mut samples = Vec::new();
    let mut verts: Vec<V> = Vec::new();
    for i in 0..mesh.npolys() {
        let pv = mesh.poly_verts(i);
        let mut poly: Vec<V> = pv
            .iter()
            .map(|&v| {
                let v = mesh.verts[v as usize];
                [v[0] as f32 * cs, v[1] as f32 * ch, v[2] as f32 * cs]
            })
            .collect();
        let b = bounds[i];
        let mut hp = HeightPatch { data: Vec::new(), xmin: b[0], ymin: b[2], width: b[1] - b[0], height: b[3] - b[2] };
        hp.data = vec![UNSET_HEIGHT; (hp.width.max(0) * hp.height.max(0)) as usize];
        get_height_data(chf, pv, &mesh.verts, bs, &mut hp, &mut arr, mesh.regs[i]);
        build_poly_detail(&poly, sample_dist, sample_max_error, height_search_radius, chf, &hp, &mut verts, &mut tris, &mut edges, &mut samples);
        let yoff = orig[1] + chf.ch;
        for v in verts.iter_mut() {
            v[0] += orig[0];
            v[1] += yoff;
            v[2] += orig[2];
        }
        for v in poly.iter_mut() {
            v[0] += orig[0];
            v[1] += orig[1];
            v[2] += orig[2];
        }
        dmesh.meshes.push([dmesh.verts.len() as u32, verts.len() as u32, dmesh.tris.len() as u32, tris.len() as u32]);
        dmesh.verts.extend_from_slice(&verts);
        dmesh.tris.extend(tris.iter().map(|t| [t[0] as u8, t[1] as u8, t[2] as u8, t[3] as u8]));
    }
    dmesh
}
