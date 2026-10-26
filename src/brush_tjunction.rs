use crate::brush::BrushPolygon;
use std::collections::HashSet;

pub const T_JUNCTION_TOLERANCE: f32 = 1e-4;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn lerp_point2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

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

fn points_inside_edge(index: &[[f32; 3]], a: [f32; 3], b: [f32; 3]) -> Vec<(f32, [f32; 3])> {
    let tol = T_JUNCTION_TOLERANCE;
    let ab = sub(b, a);
    let len_sq = dot(ab, ab);
    if len_sq <= tol * tol {
        return Vec::new();
    }
    let len = len_sq.sqrt();
    let (lo, hi) = (a[0].min(b[0]) - tol, a[0].max(b[0]) + tol);
    let start = index.partition_point(|p| p[0] < lo);
    let mut found: Vec<(f32, [f32; 3])> = Vec::new();
    for c in &index[start..] {
        if c[0] > hi {
            break;
        }
        let t = dot(sub(*c, a), ab) / len_sq;

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

#[must_use]
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

#[must_use]
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

                    out.positions.push(c);
                    out.uvs.push(lerp_point2(p.uvs[k], p.uvs[next], t));
                    out.uv2.push(lerp_point2(p.uv2[k], p.uv2[next], t));
                }
            }
            (out, changed)
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Triangulated {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub uv2: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

#[must_use]
pub fn triangulate(poly: &BrushPolygon, repaired: bool) -> Triangulated {
    let n = poly.positions.len();
    let mut out = Triangulated::default();
    if n < 3 {
        return out;
    }
    out.positions.clone_from(&poly.positions);
    out.uvs.clone_from(&poly.uvs);
    out.uv2.clone_from(&poly.uv2);
    if !repaired {
        for k in 1..n - 1 {
            out.indices.extend([0, k as u32, k as u32 + 1]);
        }
        return out;
    }

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

            if ring
                .iter()
                .any(|&v| v != a && v != b && v != c && on_segment(pos[v], pos[a], pos[c]))
            {
                continue;
            }
            let q = min_angle(pos[a], pos[b], pos[c]);
            if best.is_none_or(|(_, bq)| q > bq) {
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

fn on_segment(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> bool {
    let ab = sub(b, a);
    let len_sq = dot(ab, ab);
    if len_sq <= T_JUNCTION_TOLERANCE * T_JUNCTION_TOLERANCE {
        return false;
    }
    let t = dot(sub(p, a), ab) / len_sq;
    let len = len_sq.sqrt();
    if t * len <= T_JUNCTION_TOLERANCE || (1.0 - t) * len <= T_JUNCTION_TOLERANCE {
        return false;
    }
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
    let d = sub(p, q);
    dot(d, d) < T_JUNCTION_TOLERANCE * T_JUNCTION_TOLERANCE
}

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

    fn repaired_wall() -> BrushPolygon {
        let positions = vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
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

    #[test]
    fn the_repaired_fan_covers_the_same_area() {
        let poly = repaired_wall();
        let repaired = triangulate(&poly, true);
        let plain = triangulate(&poly, false);
        let (a, b) = (area(&repaired.positions, &repaired.indices), area(&plain.positions, &plain.indices));
        assert!((a - b).abs() < 1e-3, "repaired fan covers {a}, plain covers {b}");
        assert!(a > 11.9 && a < 12.1, "a 4x3 wall should cover 12 m2, got {a}");
    }

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
