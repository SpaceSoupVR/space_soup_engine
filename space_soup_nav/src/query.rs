//! Point queries on the assembled world: which polygon a point is on, the floor height there, a
//! straight walk along the floor (raycast), and sliding a move along the floor
//! (`move_along_surface`). The algorithms are Detour's (`DetourNavMeshQuery.cpp`, zlib licence,
//! Copyright (c) 2009-2010 Mikko Mononen; altered: written in Rust over islands and links instead
//! of tiles) and are checked against C++ Detour's answers in `tests/query_parity.rs`.
//!
//! Positions are millimetres in `f64` (`DVec3`), the world's own unit; [`mm`] and [`metres`]
//! convert at the engine boundary.
//!
//! Two deliberate differences from Detour:
//! - A search box means exactly that box. Detour rounds its box out to whole cells, so it can
//!   return a polygon up to two cells outside the box asked for.
//! - Height detail is the floor's real height. Recast's detail vertices sit one cell height above
//!   its polygon vertices (an offset its own source questions), so Detour's heights on stairs and
//!   ramps read up to one cell high.

use glam::{DVec3, IVec3, Vec3};

use crate::asset::EdgeLink;
use crate::world::{LinkKind, NavWorld, PolyRef};

pub fn mm(v: Vec3) -> DVec3 {
    v.as_dvec3() * 1000.0
}

pub fn metres(v: DVec3) -> Vec3 {
    (v / 1000.0).as_vec3()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nearest {
    pub poly: PolyRef,
    /// The nearest point on the polygon (the point itself, on the floor, when it is over it).
    pub point: DVec3,
    pub over_poly: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RayHit {
    /// How far along start..end the walk got: `f64::MAX` when it reached the end.
    pub t: f64,
    /// The wall it hit, horizontal and unit length; zero when it reached the end.
    pub normal: DVec3,
    /// Every polygon the walk crossed, from the start polygon on.
    pub path: Vec<PolyRef>,
}

fn d(v: IVec3) -> DVec3 {
    v.as_dvec3()
}

/// `dtDistancePtSegSqr2D`: squared distance on the floor plane, and where along p..q.
pub fn dist_pt_seg_sqr_2d(pt: DVec3, p: DVec3, q: DVec3) -> (f64, f64) {
    let (pqx, pqz) = (q.x - p.x, q.z - p.z);
    let (mut dx, mut dz) = (pt.x - p.x, pt.z - p.z);
    let dd = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if dd > 0.0 {
        t /= dd;
    }
    t = t.clamp(0.0, 1.0);
    dx = p.x + t * pqx - pt.x;
    dz = p.z + t * pqz - pt.z;
    (dx * dx + dz * dz, t)
}

/// `dtPointInPolygon`: crossing test on the floor plane.
pub fn point_in_polygon(pt: DVec3, verts: &[DVec3]) -> bool {
    let mut c = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (verts[i], verts[j]);
        if ((vi.z > pt.z) != (vj.z > pt.z)) && (pt.x < (vj.x - vi.x) * (pt.z - vi.z) / (vj.z - vi.z) + vi.x) {
            c = !c;
        }
        j = i;
    }
    c
}

/// `dtClosestHeightPointTriangle`.
fn closest_height_point_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<f64> {
    const EPS: f64 = 1e-6;
    let (v0, v1, v2) = (c - a, b - a, p - a);
    let mut denom = v0.x * v1.z - v0.z * v1.x;
    if denom.abs() < EPS {
        return None;
    }
    let mut u = v1.z * v2.x - v1.x * v2.z;
    let mut v = v0.x * v2.z - v0.z * v2.x;
    if denom < 0.0 {
        denom = -denom;
        u = -u;
        v = -v;
    }
    if u >= 0.0 && v >= 0.0 && (u + v) <= denom {
        return Some(a.y + (v0.y * u + v1.y * v) / denom);
    }
    None
}

/// How far outside an edge a point may be and still count as on it, millimetres.
const ON_EDGE_MM: f64 = 0.05;

/// `dtIntersectSegmentPoly2D`: (tmin, tmax, edge entered by, edge left by), or `None` when the
/// segment misses the polygon. Edge `j` runs from vertex `j` to `j + 1`.
fn intersect_segment_poly_2d(p0: DVec3, p1: DVec3, verts: &[DVec3]) -> Option<(f64, f64, i32, i32)> {
    const EPS: f64 = 1e-6;
    let (mut tmin, mut tmax) = (0.0f64, 1.0f64);
    let (mut seg_min, mut seg_max) = (-1i32, -1i32);
    let dir = p1 - p0;
    let perp = |u: DVec3, v: DVec3| u.z * v.x - u.x * v.z;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let edge = verts[i] - verts[j];
        let diff = p0 - verts[j];
        let mut nn = perp(edge, diff);
        if nn.abs() <= ON_EDGE_MM * (edge.x * edge.x + edge.z * edge.z).sqrt() {
            nn = 0.0;
        }
        let dd = perp(dir, edge);
        if dd.abs() < EPS {
            if nn < 0.0 {
                return None;
            }
            j = i;
            continue;
        }
        let t = nn / dd;
        if dd < 0.0 {
            if t > tmin {
                tmin = t;
                seg_min = j as i32;
                if tmin > tmax {
                    return None;
                }
            }
        } else if t < tmax {
            tmax = t;
            seg_max = j as i32;
            if tmax < tmin {
                return None;
            }
        }
        j = i;
    }
    Some((tmin, tmax, seg_min, seg_max))
}

impl NavWorld {
    /// A polygon's vertices in world millimetres.
    pub fn poly_dverts(&self, p: PolyRef) -> Vec<DVec3> {
        self.poly_vertices(p).map(d).collect()
    }

    /// The polygon's floor as triangles: its height detail where it has some, else its fan.
    pub fn floor_triangles(&self, p: PolyRef) -> Vec<[DVec3; 3]> {
        let isl = &self.islands[p.island as usize];
        let asset = &isl.asset;
        let verts = self.poly_dverts(p);
        let range = asset.detail.ranges.get(p.poly as usize).copied().filter(|r| r.3 > 0);
        match range {
            Some((vb, _, tb, tc)) => asset.detail.triangles[tb as usize..(tb + tc as u32) as usize]
                .iter()
                .map(|t| {
                    t.map(|k| {
                        if (k as usize) < verts.len() {
                            verts[k as usize]
                        } else {
                            d(isl.turn.apply(asset.detail.vertices[vb as usize + k as usize - verts.len()]) + isl.offset)
                        }
                    })
                })
                .collect(),
            None => (1..verts.len() - 1).map(|i| [verts[0], verts[i], verts[i + 1]]).collect(),
        }
    }

    /// The floor's outline edges: the detail triangles' edges no other triangle of the polygon
    /// shares (Detour reads the same edges from flags Recast wrote).
    fn floor_outline(&self, tris: &[[DVec3; 3]]) -> Vec<(DVec3, DVec3)> {
        let mut out = Vec::new();
        for (ti, t) in tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let shared = tris.iter().enumerate().any(|(oi, o)| oi != ti && (0..3).any(|m| o[m] == b && o[(m + 1) % 3] == a));
                if !shared {
                    out.push((a, b));
                }
            }
        }
        out
    }

    /// `getPolyHeight`: the floor height under `pos` when `pos` is over the polygon.
    pub fn poly_height(&self, p: PolyRef, pos: DVec3) -> Option<f64> {
        if !point_in_polygon(pos, &self.poly_dverts(p)) {
            return None;
        }
        let tris = self.floor_triangles(p);
        for t in &tris {
            if let Some(h) = closest_height_point_triangle(pos, t[0], t[1], t[2]) {
                return Some(h);
            }
        }
        let mut best = (f64::MAX, pos.y);
        for t in &tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let (dd, s) = dist_pt_seg_sqr_2d(pos, a, b);
                if dd < best.0 {
                    best = (dd, a.lerp(b, s).y);
                }
            }
        }
        Some(best.1)
    }

    /// `closestPointOnPoly`: the point itself on the floor when over the polygon, else the nearest
    /// point of its outline.
    pub fn closest_point_on_poly(&self, p: PolyRef, pos: DVec3) -> (DVec3, bool) {
        if let Some(h) = self.poly_height(p, pos) {
            return (DVec3::new(pos.x, h, pos.z), true);
        }
        let tris = self.floor_triangles(p);
        let mut best = (f64::MAX, pos);
        for (a, b) in self.floor_outline(&tris) {
            let (dd, s) = dist_pt_seg_sqr_2d(pos, a, b);
            if dd < best.0 {
                best = (dd, a.lerp(b, s));
            }
        }
        (best.1, false)
    }

    /// Polygons whose bounds (height detail included) overlap the box, in island and lookup-tree
    /// order.
    pub fn polys_in_box(&self, min: DVec3, max: DVec3) -> Vec<PolyRef> {
        let mut out = Vec::new();
        for (ii, isl) in self.islands.iter().enumerate() {
            let climb = isl.asset.agent.climb_mm as f64;
            if max.x < isl.min.x as f64 || min.x > isl.max.x as f64 || max.z < isl.min.z as f64 || min.z > isl.max.z as f64 {
                continue;
            }
            if max.y < isl.min.y as f64 - climb * 4.0 || min.y > isl.max.y as f64 + climb * 4.0 {
                continue;
            }
            let back = crate::asset::Turn((4 - isl.turn.0 % 4) % 4);
            let (a, b) = (back.apply((min.floor()).as_ivec3() - isl.offset), back.apply((max.ceil()).as_ivec3() - isl.offset));
            let (qmin, qmax) = (a.min(b), a.max(b));
            let nodes = &isl.asset.bvh;
            let mut i = 0usize;
            while i < nodes.len() {
                let n = nodes[i];
                let overlap = n.min.x <= qmax.x && n.max.x >= qmin.x && n.min.y <= qmax.y && n.max.y >= qmin.y && n.min.z <= qmax.z && n.max.z >= qmin.z;
                let leaf = n.index >= 0;
                if leaf && overlap {
                    out.push(PolyRef { island: ii as u32, poly: n.index as u16 });
                }
                if overlap || leaf {
                    i += 1;
                } else {
                    i += (-n.index) as usize;
                }
            }
        }
        out
    }

    /// [`NavWorld::nearest_poly`] among one island's polygons only. A doorway's middle lies on
    /// the plane two pieces share, where the plain query can answer with either.
    pub fn nearest_poly_in(&self, island: u32, center: DVec3, half_extents: DVec3) -> Option<Nearest> {
        self.nearest_among(self.polys_in_box(center - half_extents, center + half_extents).into_iter().filter(|p| p.island == island), center)
    }

    fn nearest_among(&self, polys: impl Iterator<Item = PolyRef>, center: DVec3) -> Option<Nearest> {
        let mut best: Option<(f64, Nearest)> = None;
        for p in polys {
            let (point, over) = self.closest_point_on_poly(p, center);
            let diff = center - point;
            let dist = if over {
                let climb = self.islands[p.island as usize].asset.agent.climb_mm as f64;
                let dd = diff.y.abs() - climb;
                if dd > 0.0 { dd * dd } else { 0.0 }
            } else {
                diff.length_squared()
            };
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, Nearest { poly: p, point, over_poly: over }));
            }
        }
        best.map(|b| b.1)
    }

    /// `findNearestPoly`: the polygon nearest `center` within `half_extents`. A point over a
    /// polygon and within its agent's climb counts as on it.
    pub fn nearest_poly(&self, center: DVec3, half_extents: DVec3) -> Option<Nearest> {
        self.nearest_among(self.polys_in_box(center - half_extents, center + half_extents).into_iter(), center)
    }

    /// The polygon on the far side of edge `edge` of `p` at the point `at` on it, through an
    /// internal edge or an enabled portal.
    fn across_edge(&self, p: PolyRef, edge: usize, at: DVec3) -> Option<PolyRef> {
        let isl = &self.islands[p.island as usize];
        match isl.asset.polys[p.poly as usize].edges[edge] {
            EdgeLink::Internal(o) => Some(PolyRef { island: p.island, poly: o }),
            EdgeLink::Border => None,
            EdgeLink::Socket(_) => isl.links_from[p.poly as usize].iter().map(|&l| &self.links[l as usize]).find_map(|l| {
                if !l.enabled || l.kind != LinkKind::Portal || l.from_edge != Some(edge as u8) {
                    return None;
                }
                let (a, b) = (d(l.from_segment[0]), d(l.from_segment[1]));
                let (dd, _) = dist_pt_seg_sqr_2d(at, a, b);
                (dd <= 1.0).then_some(l.to)
            }),
        }
    }

    /// Detour's `raycast`: walks the floor in a straight line from `start` (on `start_poly`)
    /// towards `end`, polygon to polygon, until a wall or the end. Seen from above: use
    /// [`NavWorld::walkable_line`] to also require arriving at `end`'s height.
    pub fn raycast(&self, start_poly: PolyRef, start: DVec3, end: DVec3) -> RayHit {
        let mut hit = RayHit { t: 0.0, normal: DVec3::ZERO, path: Vec::new() };
        let mut cur = start_poly;
        loop {
            let verts = self.poly_dverts(cur);
            let Some((_, tmax, _, seg_max)) = intersect_segment_poly_2d(start, end, &verts) else {
                return hit;
            };
            if tmax > hit.t {
                hit.t = tmax;
            }
            hit.path.push(cur);
            if seg_max == -1 {
                hit.t = f64::MAX;
                return hit;
            }
            let at = start + (end - start) * tmax;
            match self.across_edge(cur, seg_max as usize, at) {
                Some(next) => cur = next,
                None => {
                    let n = verts.len();
                    let (va, vb) = (verts[seg_max as usize], verts[(seg_max as usize + 1) % n]);
                    let (dx, dz) = (vb.x - va.x, vb.z - va.z);
                    hit.normal = DVec3::new(dz, 0.0, -dx).normalize();
                    return hit;
                }
            }
            if hit.path.len() > 4096 {
                return hit;
            }
        }
    }

    /// Whether one can walk in a straight line from `start` (on `start_poly`) to `end`: the floor
    /// raycast gets there *and* ends on the floor at `end`'s height. [`NavWorld::raycast`] alone
    /// works from above, so it also "reaches" a point on a floor overhead or below.
    pub fn walkable_line(&self, start_poly: PolyRef, start: DVec3, end: DVec3) -> bool {
        let hit = self.raycast(start_poly, start, end);
        let climb = self.islands[start_poly.island as usize].asset.agent.climb_mm as f64;
        hit.t == f64::MAX && hit.path.last().and_then(|&p| self.poly_height(p, end)).is_some_and(|h| (h - end.y).abs() <= climb)
    }

    /// Moves an agent from `start` (on `start_poly`) towards `end` along the floor, the way a game
    /// should: [`NavWorld::move_along_surface`], then onto the floor's height, then a millimetre
    /// inside the floor if it ended exactly on an edge (an agent left on an edge line can find
    /// itself on neither side of it, and stuck at the next inside corner). Returns where it is and
    /// on which polygon.
    pub fn slide(&self, start_poly: PolyRef, start: DVec3, end: DVec3) -> (DVec3, PolyRef) {
        let (mut pos, visited) = match self.gap_crossing(start_poly, start, end) {
            Some((to, at, landing)) => self.move_along_surface(to, landing, landing + (end - at)),
            None => self.move_along_surface(start_poly, start, end),
        };
        let poly = *visited.last().unwrap_or(&start_poly);
        let verts = self.poly_dverts(poly);
        let n = verts.len();
        let near_edge = (0..n).map(|k| dist_pt_seg_sqr_2d(pos, verts[k], verts[(k + 1) % n]).0).fold(f64::MAX, f64::min);
        if near_edge < 1.0 || !point_in_polygon(pos, &verts) {
            let centre = verts.iter().copied().sum::<DVec3>() / n as f64;
            let flat = DVec3::new(centre.x - pos.x, 0.0, centre.z - pos.z);
            if flat.length() > 1.0 {
                pos += flat.normalize();
            }
        }
        if let Some(h) = self.poly_height(poly, pos) {
            pos.y = h;
        }
        (pos, poly)
    }

    /// Where the step `start` -> `end` leaves `poly` through a doorway welded across a gap: the
    /// polygon beyond, the crossing point, and the matching point on the far side a millimetre
    /// in. `None` if it crosses no such doorway (or none at all).
    fn gap_crossing(&self, poly: PolyRef, start: DVec3, end: DVec3) -> Option<(PolyRef, DVec3, DVec3)> {
        let isl = &self.islands[poly.island as usize];
        let step = DVec3::new(end.x - start.x, 0.0, end.z - start.z);
        if step.length() < 1e-6 {
            return None;
        }
        let mut best: Option<(f64, PolyRef, DVec3, DVec3)> = None;
        for l in isl.links_from[poly.poly as usize].iter().map(|&l| &self.links[l as usize]) {
            if !l.enabled || l.kind != LinkKind::Portal || l.from_segment == l.to_segment {
                continue;
            }
            let (a, b) = (d(l.from_segment[0]), d(l.from_segment[1]));
            let (c, e) = (d(l.to_segment[0]), d(l.to_segment[1]));
            let across = DVec3::new((c.x + e.x - a.x - b.x) / 2.0, 0.0, (c.z + e.z - a.z - b.z) / 2.0);
            if across.length() < 1.0 || step.dot(across) <= 0.0 {
                continue;
            }
            let ab = DVec3::new(b.x - a.x, 0.0, b.z - a.z);
            let denom = step.x * ab.z - step.z * ab.x;
            if denom.abs() < 1e-9 {
                continue;
            }
            let (sx, sz) = (a.x - start.x, a.z - start.z);
            let t = (sx * ab.z - sz * ab.x) / denom;
            let u = (sx * step.z - sz * step.x) / denom;
            let slack = 1.0 / ab.length().max(1.0);
            if !(-1e-3..=1.0).contains(&t) || u < -slack || u > 1.0 + slack {
                continue;
            }
            if best.as_ref().is_none_or(|x| t < x.0) {
                let u = u.clamp(0.0, 1.0);
                let at = start + (end - start) * t.max(0.0);
                best = Some((t, l.to, at, c.lerp(e, u) + across.normalize()));
            }
        }
        best.map(|(_, to, at, landing)| (to, at, landing))
    }

    /// Detour's `moveAlongSurface`: from `start` on `start_poly` towards `end`, staying on the
    /// floor and sliding along walls; a small breadth-first search, bounded like Detour's (64
    /// polygons, 48 queued), within the circle through `start` and `end`. Returns where it got
    /// and the polygons from `start_poly` to the one it ended on.
    pub fn move_along_surface(&self, start_poly: PolyRef, start: DVec3, end: DVec3) -> (DVec3, Vec<PolyRef>) {
        const MAX_NODES: usize = 64;
        const MAX_STACK: usize = 48;
        let mut nodes: Vec<(PolyRef, Option<usize>, bool)> = vec![(start_poly, None, true)];
        let mut stack: Vec<usize> = vec![0];
        let mut best_pos = start;
        let mut best_dist = f64::MAX;
        let mut best_node: Option<usize> = None;
        let search_pos = start.lerp(end, 0.5);
        let r = (start - end).length() / 2.0 + 1.0;
        let search_rad_sqr = r * r;
        while !stack.is_empty() {
            let cur_node = stack.remove(0);
            let cur = nodes[cur_node].0;
            let verts = self.poly_dverts(cur);
            if point_in_polygon(end, &verts) {
                best_node = Some(cur_node);
                best_pos = end;
                break;
            }
            let isl = &self.islands[cur.island as usize];
            let n = verts.len();
            let mut j = n - 1;
            for i in 0..n {
                let neis: Vec<PolyRef> = match isl.asset.polys[cur.poly as usize].edges[j] {
                    EdgeLink::Internal(o) => vec![PolyRef { island: cur.island, poly: o }],
                    EdgeLink::Border => Vec::new(),
                    EdgeLink::Socket(_) => isl.links_from[cur.poly as usize]
                        .iter()
                        .map(|&l| &self.links[l as usize])
                        .filter(|l| l.enabled && l.kind == LinkKind::Portal && l.from_edge == Some(j as u8))
                        .map(|l| l.to)
                        .take(8)
                        .collect(),
                };
                let (vj, vi) = (verts[j], verts[i]);
                if neis.is_empty() {
                    let (dd, tseg) = dist_pt_seg_sqr_2d(end, vj, vi);
                    if dd < best_dist {
                        best_pos = vj.lerp(vi, tseg);
                        best_dist = dd;
                        best_node = Some(cur_node);
                    }
                } else {
                    for nb in neis {
                        let ni = match nodes.iter().position(|x| x.0 == nb) {
                            Some(k) => k,
                            None if nodes.len() < MAX_NODES => {
                                nodes.push((nb, None, false));
                                nodes.len() - 1
                            }
                            None => continue,
                        };
                        if nodes[ni].2 {
                            continue;
                        }
                        let (dd, _) = dist_pt_seg_sqr_2d(search_pos, vj, vi);
                        if dd > search_rad_sqr {
                            continue;
                        }
                        if stack.len() < MAX_STACK {
                            nodes[ni].1 = Some(cur_node);
                            nodes[ni].2 = true;
                            stack.push(ni);
                        }
                    }
                }
                j = i;
            }
        }
        let mut visited = Vec::new();
        let mut at = best_node;
        while let Some(k) = at {
            visited.push(nodes[k].0);
            at = nodes[k].1;
        }
        visited.reverse();
        (best_pos, visited)
    }
}
