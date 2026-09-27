//! T-junction repair for brush geometry.
//!
//! # The crack this removes
//!
//! Subtracting one brush from another produces faces that END partway along a
//! neighbour's edge. The room carved out of a shell has a ceiling whose corner
//! lands at y = 3.1 on the shell's front-wall edge, which runs from y = -0.3 to
//! 3.4 without a vertex there. The two triangles meeting along that line do not
//! share an edge, only a line, and float rounding in the rasteriser puts them a
//! fraction of a pixel apart. The gap shows whatever is behind the wall -- the
//! grass outdoors -- as single pixels that flicker as the head moves.
//!
//! Measured on `test_room` before this existed: 408 vertex-on-edge incidences
//! across 204 triangles.
//!
//! # The repair
//!
//! The standard one from brush compilers (qbsp's T-junction pass): wherever a
//! vertex of any polygon lies strictly inside another polygon's edge, split that
//! edge at the vertex, so both sides share it exactly.
//!
//! Done on POLYGONS rather than triangles, and a repaired polygon is then
//! triangulated from its centroid rather than from a corner. A corner fan fails
//! precisely here: points inserted on the corner's own two edges are collinear
//! with it, those triangles have zero area, and the edge they were meant to
//! share is covered by a triangle that still spans it whole. From an interior
//! point every boundary segment gets a triangle of its own.
//!
//! Attributes are interpolated linearly, which is exact rather than approximate:
//! a brush face is planar and both its texture and lightmap coordinates are
//! affine functions of position on it.

use crate::brush::BrushPolygon;
use std::collections::HashSet;

/// How close, in metres, a vertex must be to an edge to count as lying on it,
/// and how far from the edge's ends it must be to count as inside it.
///
/// A tenth of a millimetre: well above float noise on level-scale coordinates,
/// far below any real gap between surfaces an author would intend.
pub const T_JUNCTION_TOLERANCE: f32 = 1e-4;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Every distinct vertex position across all polygons, sorted by x so an edge
/// only has to look at the points inside its own x-range.
fn vertex_index(polys: &[BrushPolygon]) -> Vec<[f32; 3]> {
    let q = |v: f32| (v / T_JUNCTION_TOLERANCE).round() as i64;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in polys {
        for v in &p.positions {
            if seen.insert([q(v[0]), q(v[1]), q(v[2])]) {
                out.push(*v);
            }
        }
    }
    out.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// The points lying strictly inside edge `a -> b`, as `(t, position)` in order.
fn points_inside_edge(index: &[[f32; 3]], a: [f32; 3], b: [f32; 3]) -> Vec<(f32, [f32; 3])> {
    let tol = T_JUNCTION_TOLERANCE;
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    if len2 <= tol * tol {
        return Vec::new();
    }
    let len = len2.sqrt();
    let (lo, hi) = (a[0].min(b[0]) - tol, a[0].max(b[0]) + tol);
    let start = index.partition_point(|p| p[0] < lo);
    let mut found: Vec<(f32, [f32; 3])> = Vec::new();
    for c in &index[start..] {
        if c[0] > hi {
            break;
        }
        let t = dot(sub(*c, a), ab) / len2;
        // Strictly INSIDE: at least the tolerance away from both ends, so an
        // edge's own endpoints and the corners it already shares are never
        // counted as junctions on it.
        if t * len <= tol || (1.0 - t) * len <= tol {
            continue;
        }
        let on_line = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
        let d = sub(*c, on_line);
        if dot(d, d) < tol * tol {
            found.push((t, *c));
        }
    }
    found.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    found.dedup_by(|x, y| {
        let d = sub(x.1, y.1);
        dot(d, d) < tol * tol
    });
    found
}

/// How many vertices lie inside some other polygon edge. Zero means watertight.
pub fn count_t_junctions(polys: &[BrushPolygon]) -> usize {
    let index = vertex_index(polys);
    polys
        .iter()
        .map(|p| {
            let n = p.positions.len();
            (0..n)
                .map(|k| points_inside_edge(&index, p.positions[k], p.positions[(k + 1) % n]).len())
                .sum::<usize>()
        })
        .sum()
}

/// Split every polygon edge at the vertices lying inside it.
///
/// Returns each polygon with a flag saying whether it changed, so an untouched
/// polygon can keep the exact triangulation it always had.
pub fn fix_t_junctions(polys: &[BrushPolygon]) -> Vec<(BrushPolygon, bool)> {
    let index = vertex_index(polys);
    polys
        .iter()
        .map(|p| {
            let n = p.positions.len();
            let mut out = BrushPolygon {
                material: p.material.clone(),
                normal: p.normal,
                tangent: p.tangent,
                ..Default::default()
            };
            let mut changed = false;
            for k in 0..n {
                let next = (k + 1) % n;
                out.positions.push(p.positions[k]);
                out.uvs.push(p.uvs[k]);
                out.uv2.push(p.uv2[k]);
                for (t, c) in points_inside_edge(&index, p.positions[k], p.positions[next]) {
                    changed = true;
                    // The OTHER polygon's position exactly, so the two sides
                    // share one vertex bit for bit.
                    out.positions.push(c);
                    out.uvs.push(lerp2(p.uvs[k], p.uvs[next], t));
                    out.uv2.push(lerp2(p.uv2[k], p.uv2[next], t));
                }
            }
            (out, changed)
        })
        .collect()
}

/// A polygon as triangles, with per-vertex attributes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Triangulated {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub uv2: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Triangulate a convex brush polygon.
///
/// An unrepaired polygon fans from its first corner, exactly as
/// `brush_mesh_in_atlas` does, so geometry that had no junction is unchanged.
/// A repaired one fans from its centroid; see the module note for why a corner
/// fan cannot be used once edges carry inserted points.
pub fn triangulate(poly: &BrushPolygon, repaired: bool) -> Triangulated {
    let n = poly.positions.len();
    let mut out = Triangulated::default();
    if n < 3 {
        return out;
    }
    out.positions = poly.positions.clone();
    out.uvs = poly.uvs.clone();
    out.uv2 = poly.uv2.clone();
    if !repaired {
        for k in 1..n - 1 {
            out.indices.extend([0, k as u32, k as u32 + 1]);
        }
        return out;
    }
    // EAR CLIPPING OVER STRICTLY CONVEX CORNERS -- every vertex used, none
    // invented, no triangle without area (2026-09-23).
    //
    // History, because each attempt failed differently. A fan from an INVENTED
    // centroid vertex reached zero T-junctions but put a vertex in no other
    // representation of the surface, and shipped a sawtooth along room edges
    // (2026-09-10). A fan from one real corner invented nothing, but a fan
    // triangle spanning a run of collinear inserted points has no area, and
    // dropping it discarded the very sub-edges that were the repair: test_room
    // went 408 -> 300, not 0. Both also made long slivers radiating from one
    // point.
    //
    // A convex polygon whose edges carry extra collinear points always has a
    // corner whose triangle with its two neighbours has real area. Cut that
    // corner off and the rest is still such a polygon. Collinear points are
    // never cut on their own (their triangle is empty) -- they stay until they
    // become the BASE of a neighbour's ear, so every one ends up as a triangle
    // vertex and every sub-edge survives. Among the valid corners the one whose
    // triangle has the largest smallest angle is cut first, which keeps the
    // triangles fat instead of fanning slivers.
    let pos = &poly.positions;
    let mut ring: Vec<usize> = (0..n).collect();
    while ring.len() > 3 {
        let m = ring.len();
        let mut best: Option<(usize, f32)> = None;
        for k in 0..m {
            let (a, b, c) = (ring[(k + m - 1) % m], ring[k], ring[(k + 1) % m]);
            if triangle_is_degenerate(pos[a], pos[b], pos[c]) {
                continue;
            }
            // NOTHING MAY LIE ON THE NEW CHORD a-c. Cutting this corner makes
            // a-c an edge of what remains; if another vertex sits on it, that
            // vertex is now a T-junction on our own triangle and the rest of
            // the ring can collapse into a collinear run with nothing left to
            // cut. Caught by `a_rectangle_with_points_on_one_edge_uses_every_sub_edge`:
            // cutting a far corner of the ceiling ran the chord straight across
            // the front edge, through both doorway corners.
            if ring
                .iter()
                .any(|&v| v != a && v != b && v != c && on_segment(pos[v], pos[a], pos[c]))
            {
                continue;
            }
            let q = min_angle(pos[a], pos[b], pos[c]);
            if best.map_or(true, |(_, bq)| q > bq) {
                best = Some((k, q));
            }
        }
        let Some((k, _)) = best else { break };
        let (a, b, c) = (ring[(k + m - 1) % m], ring[k], ring[(k + 1) % m]);
        out.indices.extend([a as u32, b as u32, c as u32]);
        ring.remove(k);
    }
    if ring.len() == 3 && !triangle_is_degenerate(pos[ring[0]], pos[ring[1]], pos[ring[2]]) {
        out.indices.extend([ring[0] as u32, ring[1] as u32, ring[2] as u32]);
    }
    out
}

/// Whether `p` lies on the segment a-b, strictly between its ends, to the
/// T-junction tolerance.
fn on_segment(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> bool {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    if len2 <= T_JUNCTION_TOLERANCE * T_JUNCTION_TOLERANCE {
        return false;
    }
    let t = dot(sub(p, a), ab) / len2;
    let len = len2.sqrt();
    if t * len <= T_JUNCTION_TOLERANCE || (1.0 - t) * len <= T_JUNCTION_TOLERANCE {
        return false;
    }
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
    let d = sub(p, q);
    dot(d, d) < T_JUNCTION_TOLERANCE * T_JUNCTION_TOLERANCE
}

/// The smallest interior angle of a triangle, in radians. Used to prefer fat
/// ears over slivers when several corners could be cut.
fn min_angle(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let ang = |p: [f32; 3], q: [f32; 3], r: [f32; 3]| {
        let u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let v = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
        let lu = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt().max(1e-12);
        let lv = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        ((u[0] * v[0] + u[1] * v[1] + u[2] * v[2]) / (lu * lv)).clamp(-1.0, 1.0).acos()
    };
    ang(a, b, c).min(ang(b, c, a)).min(ang(c, a, b))
}

/// Whether three points are collinear (or coincident) enough to enclose no
/// area worth rasterising.
///
/// Scaled by the edge lengths, so it means the same thing for a trim piece and
/// for a warehouse wall -- an absolute area threshold would call every small
/// triangle degenerate.
fn triangle_is_degenerate(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> bool {
    let e0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let x = e0[1] * e1[2] - e0[2] * e1[1];
    let y = e0[2] * e1[0] - e0[0] * e1[2];
    let z = e0[0] * e1[1] - e0[1] * e1[0];
    let two_area = (x * x + y * y + z * z).sqrt();
    let l0 = (e0[0] * e0[0] + e0[1] * e0[1] + e0[2] * e0[2]).sqrt();
    let l1 = (e1[0] * e1[0] + e1[1] * e1[1] + e1[2] * e1[2]).sqrt();
    two_area <= 1e-4 * l0.max(1e-6) * l1.max(1e-6)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// An axis-aligned quad in the z = 0 plane, uv = (x, y), uv2 = (x, y) / 4.
    fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> BrushPolygon {
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        BrushPolygon {
            material: "m".into(),
            positions: corners.iter().map(|c| [c[0], c[1], 0.0]).collect(),
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            uvs: corners.iter().map(|c| [c[0], c[1]]).collect(),
            uv2: corners.iter().map(|c| [c[0] * 0.25, c[1] * 0.25]).collect(),
        }
    }

    /// A 2 x 2 square with two 1 x 1 squares against its right edge: the point
    /// (2, 1) is a corner of both small squares and lies inside the big
    /// square's edge from (2, 0) to (2, 2). Exactly one junction.
    fn one_junction() -> Vec<BrushPolygon> {
        vec![quad(0.0, 0.0, 2.0, 2.0), quad(2.0, 0.0, 3.0, 1.0), quad(2.0, 1.0, 3.0, 2.0)]
    }

    fn area(t: &Triangulated) -> f32 {
        t.indices
            .chunks(3)
            .map(|tri| {
                let (a, b, c) = (t.positions[tri[0] as usize], t.positions[tri[1] as usize], t.positions[tri[2] as usize]);
                let (u, v) = (sub(b, a), sub(c, a));
                let x = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                0.5 * dot(x, x).sqrt()
            })
            .sum()
    }

    /// THE DETECTOR, checked on both branches before anything relies on it.
    ///
    /// A "zero junctions after the repair" assertion is worthless if the counter
    /// can only ever say zero -- so it must find the one known junction here
    /// AND report none where two equal squares share a whole edge.
    #[test]
    fn the_detector_finds_a_known_junction_and_invents_none() {
        assert_eq!(count_t_junctions(&one_junction()), 1, "missed the known junction at (2, 1)");
        let clean = vec![quad(0.0, 0.0, 1.0, 1.0), quad(1.0, 0.0, 2.0, 1.0)];
        assert_eq!(count_t_junctions(&clean), 0, "reported a junction between squares sharing a whole edge");
    }

    #[test]
    fn repair_leaves_no_junction_behind() {
        let fixed: Vec<BrushPolygon> = fix_t_junctions(&one_junction()).into_iter().map(|(p, _)| p).collect();
        assert_eq!(count_t_junctions(&fixed), 0);
        // And it split the RIGHT polygon: the big square gained one vertex.
        assert_eq!(fixed[0].positions.len(), 5);
        assert_eq!(fixed[1].positions.len(), 4);
    }

    #[test]
    fn repair_changes_no_surface_area() {
        for (p, repaired) in fix_t_junctions(&one_junction()) {
            let before = area(&triangulate(&quad_from(&p), false));
            let after = area(&triangulate(&p, repaired));
            assert!((before - after).abs() < 1e-5, "area {before} became {after}");
        }
    }

    /// The same outline without the inserted points, for measuring area.
    fn quad_from(p: &BrushPolygon) -> BrushPolygon {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for v in &p.positions {
            x0 = x0.min(v[0]);
            y0 = y0.min(v[1]);
            x1 = x1.max(v[0]);
            y1 = y1.max(v[1]);
        }
        quad(x0, y0, x1, y1)
    }

    /// A corner fan would leave degenerate triangles; the centroid fan leaves
    /// none, which is what makes the split edge actually covered.
    #[test]
    fn a_repaired_polygon_has_no_zero_area_triangle() {
        let (p, repaired) = fix_t_junctions(&one_junction()).remove(0);
        assert!(repaired);
        let t = triangulate(&p, true);
        for tri in t.indices.chunks(3) {
            let one = Triangulated {
                positions: t.positions.clone(),
                indices: tri.to_vec(),
                ..Default::default()
            };
            assert!(area(&one) > 1e-6, "degenerate triangle {tri:?}");
        }
    }

    /// Inserted and centroid vertices carry the texture and lightmap coordinates
    /// the face's affine maps give at their positions.
    #[test]
    fn attributes_stay_on_the_faces_affine_maps() {
        let (p, repaired) = fix_t_junctions(&one_junction()).remove(0);
        let t = triangulate(&p, repaired);
        for i in 0..t.positions.len() {
            let v = t.positions[i];
            assert!((t.uvs[i][0] - v[0]).abs() < 1e-5 && (t.uvs[i][1] - v[1]).abs() < 1e-5, "uv off the map at {v:?}");
            assert!((t.uv2[i][0] - v[0] * 0.25).abs() < 1e-5 && (t.uv2[i][1] - v[1] * 0.25).abs() < 1e-5, "uv2 off the map at {v:?}");
        }
    }

    #[test]
    fn an_untouched_polygon_triangulates_exactly_as_the_corner_fan() {
        let q = quad(0.0, 0.0, 1.0, 1.0);
        let t = triangulate(&q, false);
        assert_eq!(t.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(t.positions, q.positions);
    }
}

#[cfg(test)]
mod corner_fan_tests {
    use super::{triangulate, BrushPolygon};

    /// A wall quad with an extra vertex inserted mid-edge -- exactly what a
    /// T-junction repair produces, and exactly the shape that breaks a naive
    /// fan from vertex 0.
    fn repaired_wall() -> BrushPolygon {
        let positions = vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],   // <- inserted, COLLINEAR with its neighbours
            [4.0, 0.0, 0.0],
            [4.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ];
        let uvs = positions.iter().map(|p| [p[0] * 0.25, p[1] * 0.25]).collect();
        let uv2 = positions.iter().map(|p| [p[0] * 0.25, p[1] * 0.25]).collect();
        BrushPolygon { positions, uvs, uv2, ..Default::default() }
    }

    fn area(p: &[[f32; 3]], idx: &[u32]) -> f32 {
        idx.chunks(3)
            .map(|t| {
                let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
                let e0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let e1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let x = e0[1] * e1[2] - e0[2] * e1[1];
                let y = e0[2] * e1[0] - e0[0] * e1[2];
                let z = e0[0] * e1[1] - e0[1] * e1[0];
                0.5 * (x * x + y * y + z * z).sqrt()
            })
            .sum()
    }

    /// THE REGRESSION THIS REPLACES. The old repaired path pushed the
    /// boundary's mean as an extra vertex -- a point present in no other
    /// representation of this geometry, while the lightmap is baked against
    /// the unrepaired mesh.
    #[test]
    fn a_repaired_polygon_invents_no_new_vertices() {
        let poly = repaired_wall();
        let t = triangulate(&poly, true);
        assert_eq!(
            t.positions.len(),
            poly.positions.len(),
            "the repaired path added a vertex; the lightmap and shadow map were \
             built without it",
        );
        assert_eq!(t.uvs.len(), poly.uvs.len());
        assert_eq!(t.uv2.len(), poly.uv2.len());
    }

    /// Fanning from a collinear inserted vertex -- or from a vertex next to one
    /// -- yields triangles of zero area. That is very likely why a centroid was
    /// reached for; a real corner avoids it without inventing anything.
    #[test]
    fn no_triangle_is_degenerate_even_though_a_vertex_is_collinear() {
        let poly = repaired_wall();
        let t = triangulate(&poly, true);
        assert_eq!(t.indices.len() % 3, 0);
        for tri in t.indices.chunks(3) {
            let a = t.positions[tri[0] as usize];
            let b = t.positions[tri[1] as usize];
            let c = t.positions[tri[2] as usize];
            let e0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let x = e0[1] * e1[2] - e0[2] * e1[1];
            let y = e0[2] * e1[0] - e0[0] * e1[2];
            let z = e0[0] * e1[1] - e0[1] * e1[0];
            let two_a = (x * x + y * y + z * z).sqrt();
            assert!(two_a > 1e-5, "degenerate triangle {tri:?} in the repaired fan");
        }
    }

    /// The surface must still be exactly covered -- no gaps, no double cover.
    #[test]
    fn the_repaired_fan_covers_the_same_area() {
        let poly = repaired_wall();
        let repaired = triangulate(&poly, true);
        let plain = triangulate(&poly, false);
        let (a, b) = (area(&repaired.positions, &repaired.indices), area(&plain.positions, &plain.indices));
        assert!((a - b).abs() < 1e-3, "repaired fan covers {a}, plain covers {b}");
        assert!(a > 11.9 && a < 12.1, "a 4x3 wall should cover 12 m2, got {a}");
    }

    /// The repaired fan never emits MORE triangles than a plain one, and never
    /// emits none. It may emit fewer, because a fan triangle spanning a
    /// collinear run is dropped -- that triangle covered no area, which
    /// `the_repaired_fan_covers_the_same_area` is what actually proves.
    #[test]
    fn the_repaired_fan_is_no_larger_than_a_plain_one() {
        let poly = repaired_wall();
        let r = triangulate(&poly, true).indices.len();
        let p = triangulate(&poly, false).indices.len();
        assert!(r > 0, "the repaired polygon produced no triangles at all");
        assert!(r <= p, "the repaired fan emitted {r} indices against a plain {p}");
    }
}

#[cfg(test)]
mod ear_clip_tests {
    use super::*;
    use crate::brush::BrushPolygon;

    /// The front of test_room's hall ceiling: a rectangle whose front edge
    /// carries the two doorway corners at x = +-0.8. Every sub-edge of that
    /// edge must appear as a triangle edge, or the T-junction stays open.
    #[test]
    fn a_rectangle_with_points_on_one_edge_uses_every_sub_edge() {
        let positions = vec![
            [2.7, 3.1, 3.7],
            [0.8, 3.1, 3.7],
            [-0.8, 3.1, 3.7],
            [-2.7, 3.1, 3.7],
            [-2.7, 3.1, -15.7],
            [2.7, 3.1, -15.7],
        ];
        let poly = BrushPolygon {
            positions: positions.clone(),
            uvs: vec![[0.0, 0.0]; 6],
            uv2: vec![[0.0, 0.0]; 6],
            normal: [0.0, -1.0, 0.0],
            ..Default::default()
        };
        let t = triangulate(&poly, true);
        let edges: std::collections::HashSet<(u32, u32)> = t
            .indices
            .chunks(3)
            .flat_map(|c| [(c[0], c[1]), (c[1], c[2]), (c[2], c[0])])
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        for (a, b) in [(0u32, 1u32), (1, 2), (2, 3)] {
            assert!(edges.contains(&(a, b)), "sub-edge {a}-{b} missing; triangles {:?}", t.indices);
        }
        assert!(!edges.contains(&(0, 3)), "the whole front edge is still one triangle edge: {:?}", t.indices);
    }
}
