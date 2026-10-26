use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const EPS: f64 = 1e-5;

const HUGE: f64 = 1e5;

pub type Vec3 = [f64; 3];

#[inline]
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn length(a: Vec3) -> f64 {

    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l < EPS {
        [0.0, 1.0, 0.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub n: Vec3,
    pub d: f64,
}

impl Plane {
    pub fn new(n: Vec3, d: f64) -> Self {
        Plane { n: normalize(n), d }
    }

    #[inline]
    pub fn distance(&self, p: Vec3) -> f64 {
        dot(self.n, p) - self.d
    }

    pub fn flipped(&self) -> Self {
        Plane {
            n: scale(self.n, -1.0),
            d: -self.d,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushFace {

    pub plane: [f64; 4],
    #[serde(default = "default_material")]
    pub material: String,
    #[serde(default, rename = "uAxis")]
    pub u_axis: Option<Vec3>,
    #[serde(default, rename = "vAxis")]
    pub v_axis: Option<Vec3>,
    #[serde(default = "default_scale")]
    pub scale: [f64; 2],
    #[serde(default)]
    pub shift: [f64; 2],
    #[serde(default)]
    pub rotation: f64,
    #[serde(default = "default_lightmap", rename = "lightmapScale")]
    pub lightmap_scale: f64,
    #[serde(default)]
    pub smoothing: u32,
}

fn default_material() -> String {
    "default".to_string()
}
fn default_scale() -> [f64; 2] {
    [2.0, 2.0]
}

fn default_lightmap() -> f64 {
    0.125
}

impl BrushFace {
    pub fn plane(&self) -> Plane {
        Plane {
            n: [self.plane[0], self.plane[1], self.plane[2]],
            d: self.plane[3],
        }
    }

    pub fn axes(&self) -> (Vec3, Vec3) {
        let (du, dv) = default_axes(self.plane().n);
        let u = self.u_axis.unwrap_or(du);
        let v = self.v_axis.unwrap_or(dv);
        rotate_axes(u, v, self.plane().n, self.rotation)
    }

    pub fn uv(&self, p: Vec3) -> [f64; 2] {
        let (u, v) = self.axes();
        let su = if self.scale[0] == 0.0 { 1.0 } else { self.scale[0] };
        let sv = if self.scale[1] == 0.0 { 1.0 } else { self.scale[1] };
        [
            dot(p, u) / su + self.shift[0],
            dot(p, v) / sv + self.shift[1],
        ]
    }
}

pub fn default_axes(n: Vec3) -> (Vec3, Vec3) {
    let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
    if ay >= ax && ay >= az {
        if n[1] >= 0.0 {
            ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
        } else {
            ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0])
        }
    } else if ax >= az {
        if n[0] >= 0.0 {
            ([0.0, 0.0, -1.0], [0.0, -1.0, 0.0])
        } else {
            ([0.0, 0.0, 1.0], [0.0, -1.0, 0.0])
        }
    } else if n[2] >= 0.0 {
        ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0])
    } else {
        ([-1.0, 0.0, 0.0], [0.0, -1.0, 0.0])
    }
}

fn rotate_axes(u: Vec3, v: Vec3, n: Vec3, degrees: f64) -> (Vec3, Vec3) {
    let r = degrees.to_radians();
    let (c, s) = (r.cos(), r.sin());

    let rot = |a: Vec3| add(scale(a, c), scale(cross(n, a), s));
    (rot(u), rot(v))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushSolid {
    pub faces: Vec<BrushFace>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrushDef {
    #[serde(default)]
    pub solids: Vec<BrushSolid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtract: Vec<BrushSolid>,
}

fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());

    let seed = if ax <= ay && ax <= az {
        [1.0, 0.0, 0.0]
    } else if ay <= az {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let u = normalize(cross(n, seed));
    (u, normalize(cross(n, u)))
}

fn seed_polygon(pl: Plane) -> Vec<Vec3> {
    let (u, v) = plane_basis(pl.n);
    let centre = scale(pl.n, pl.d);
    vec![
        add(centre, add(scale(u, -HUGE), scale(v, -HUGE))),
        add(centre, add(scale(u, HUGE), scale(v, -HUGE))),
        add(centre, add(scale(u, HUGE), scale(v, HUGE))),
        add(centre, add(scale(u, -HUGE), scale(v, HUGE))),
    ]
}

pub fn clip_polygon(poly: &[Vec3], pl: Plane) -> Vec<Vec3> {
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let da = pl.distance(a);
        let db = pl.distance(b);
        if da <= EPS {
            out.push(a);
        }
        if (da > EPS && db < -EPS) || (da < -EPS && db > EPS) {
            let t = da / (da - db);
            out.push(add(a, scale(sub(b, a), t)));
        }
    }
    out
}

#[inline]
fn snap(x: f64) -> f64 {

    (x / EPS).round() * EPS
}

fn dedupe(poly: Vec<Vec3>) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::with_capacity(poly.len());
    for p in poly {
        let q = [snap(p[0]), snap(p[1]), snap(p[2])];
        if let Some(last) = out.last() {
            if length(sub(*last, q)) < EPS * 10.0 {
                continue;
            }
        }
        out.push(q);
    }
    if out.len() > 2 && length(sub(out[0], out[out.len() - 1])) < EPS * 10.0 {
        out.pop();
    }
    out
}

pub fn solid_polygons(solid: &BrushSolid) -> Vec<Option<Vec<Vec3>>> {
    (0..solid.faces.len())
        .map(|i| {
            let mut poly = seed_polygon(solid.faces[i].plane());
            for (j, other) in solid.faces.iter().enumerate() {
                if j == i || poly.len() < 3 {
                    continue;
                }
                poly = clip_polygon(&poly, other.plane());
            }
            let poly = dedupe(poly);
            if poly.len() >= 3 {
                Some(poly)
            } else {
                None
            }
        })
        .collect()
}

pub fn is_valid_solid(solid: &BrushSolid) -> bool {
    solid.faces.len() >= 4 && solid_polygons(solid).iter().flatten().count() >= 4
}

pub fn solid_bounds(solid: &BrushSolid) -> Option<(Vec3, Vec3)> {
    let mut min: Option<Vec3> = None;
    let mut max: Vec3 = [0.0; 3];
    for poly in solid_polygons(solid).into_iter().flatten() {
        for p in poly {
            match min.as_mut() {
                None => {
                    min = Some(p);
                    max = p;
                }
                Some(lo) => {
                    for i in 0..3 {
                        if p[i] < lo[i] {
                            lo[i] = p[i];
                        }
                        if p[i] > max[i] {
                            max[i] = p[i];
                        }
                    }
                }
            }
        }
    }
    min.map(|lo| (lo, max))
}

pub fn solid_volume(solid: &BrushSolid) -> f64 {
    let polys = solid_polygons(solid);
    let mut total = 0.0;
    for (i, poly) in polys.iter().enumerate() {
        let Some(poly) = poly else { continue };
        let n = solid.faces[i].plane().n;
        let mut area = 0.0;
        for k in 1..poly.len().saturating_sub(1) {
            area += length(cross(sub(poly[k], poly[0]), sub(poly[k + 1], poly[0]))) / 2.0;
        }
        total += area * dot(n, poly[0]) / 3.0;
    }
    total.abs()
}

pub fn contains_point(solid: &BrushSolid, p: Vec3, tolerance: f64) -> bool {
    solid.faces.iter().all(|f| f.plane().distance(p) <= tolerance)
}

const EXPOSURE_PROBE: f64 = 1e-3;

fn polygon_area(poly: &[Vec3], n: Vec3) -> f64 {
    polygon_winding(poly, n).abs() * 0.5
}

fn polygon_centroid(poly: &[Vec3]) -> Vec3 {
    let mut c = [0.0; 3];
    for p in poly {
        c = add(c, *p);
    }
    scale(c, 1.0 / poly.len().max(1) as f64)
}

fn subtract_cover(poly: &[Vec3], face: Plane, cover: &BrushSolid) -> Vec<Vec<Vec3>> {
    let mut kept = Vec::new();
    let mut rest: Vec<Vec3> = poly.to_vec();
    for f in &cover.faces {
        let pl = f.plane();
        let parallel = dot(pl.n, face.n).abs() > 1.0 - 1e-9;
        if parallel && rest.iter().all(|p| pl.distance(*p).abs() <= EPS) {
            continue;
        }
        let outside = dedupe(clip_polygon(&rest, pl.flipped()));
        let inside = dedupe(clip_polygon(&rest, pl));
        if outside.len() >= 3 && polygon_area(&outside, face.n) > EPS * EPS {
            kept.push(outside);
        }
        rest = inside;
        if rest.len() < 3 {
            return kept;
        }
    }
    let probe = add(polygon_centroid(&rest), scale(face.n, EXPOSURE_PROBE));
    if !contains_point(cover, probe, -EPS) {
        kept.push(rest);
    }
    kept
}

pub fn exposed_fragments(solids: &[BrushSolid]) -> Vec<Vec<Vec<Vec<Vec3>>>> {
    let bounds: Vec<Option<(Vec3, Vec3)>> = solids.iter().map(solid_bounds).collect();
    let overlaps = |a: (Vec3, Vec3), b: (Vec3, Vec3)| {
        (0..3).all(|k| a.0[k] <= b.1[k] + EPS && b.0[k] <= a.1[k] + EPS)
    };
    solids
        .iter()
        .enumerate()
        .map(|(si, solid)| {
            solid_polygons(solid)
                .into_iter()
                .enumerate()
                .map(|(fi, poly)| {
                    let Some(poly) = poly else { return Vec::new() };
                    let face = solid.faces[fi].plane();
                    let mut frags = vec![poly];
                    for (oj, other) in solids.iter().enumerate() {
                        if oj == si {
                            continue;
                        }
                        let (Some(a), Some(b)) = (bounds[si], bounds[oj]) else { continue };
                        if !overlaps(a, b) {
                            continue;
                        }
                        frags = frags
                            .iter()
                            .flat_map(|f| subtract_cover(f, face, other))
                            .collect();
                        if frags.is_empty() {
                            break;
                        }
                    }
                    frags
                })
                .collect()
        })
        .collect()
}

fn face_inheriting(pl: Plane, from: Option<&BrushFace>) -> BrushFace {
    let (u, v) = default_axes(pl.n);
    BrushFace {
        plane: [pl.n[0], pl.n[1], pl.n[2], pl.d],
        material: from.map_or_else(default_material, |f| f.material.clone()),
        u_axis: Some(u),
        v_axis: Some(v),
        scale: from.map_or_else(default_scale, |f| f.scale),
        shift: from.map_or([0.0, 0.0], |f| f.shift),
        rotation: 0.0,
        lightmap_scale: from.map_or_else(default_lightmap, |f| f.lightmap_scale),
        smoothing: 0,
    }
}

pub fn split_solid(solid: &BrushSolid, cut: Plane) -> (Option<BrushSolid>, Option<BrushSolid>) {
    let mut best: Option<&BrushFace> = None;
    let mut best_alignment = f64::NEG_INFINITY;
    for f in &solid.faces {
        let a = dot(f.plane().n, cut.n).abs();
        if a > best_alignment {
            best_alignment = a;
            best = Some(f);
        }
    }

    let mut back_faces = solid.faces.clone();
    back_faces.push(face_inheriting(cut, best));
    let mut front_faces = solid.faces.clone();
    front_faces.push(face_inheriting(cut.flipped(), best));

    let back = BrushSolid { faces: back_faces };
    let front = BrushSolid { faces: front_faces };
    (
        if is_valid_solid(&front) { Some(front) } else { None },
        if is_valid_solid(&back) { Some(back) } else { None },
    )
}

pub fn subtract_solid(target: &BrushSolid, tool: &BrushSolid) -> Vec<BrushSolid> {
    if !is_valid_solid(target) {
        return Vec::new();
    }
    if !is_valid_solid(tool) {
        return vec![target.clone()];
    }

    match (solid_bounds(target), solid_bounds(tool)) {
        (Some((amin, amax)), Some((bmin, bmax))) => {
            for i in 0..3 {
                if amin[i] >= bmax[i] - EPS || amax[i] <= bmin[i] + EPS {
                    return vec![target.clone()];
                }
            }
        }
        _ => return vec![target.clone()],
    }

    let mut pieces = Vec::new();
    let mut remaining = Some(target.clone());
    for f in &tool.faces {
        let Some(current) = remaining.take() else { break };
        let (front, back) = split_solid(&current, f.plane());
        if let Some(front) = front {
            pieces.push(front);
        }
        remaining = back;
    }
    pieces
}

impl BrushDef {

    pub fn evaluate(&self) -> Vec<BrushSolid> {
        if self.solids.is_empty() {
            return Vec::new();
        }
        if self.subtract.is_empty() {
            return self.solids.clone();
        }
        let mut out = Vec::new();
        for solid in &self.solids {
            let mut current = vec![solid.clone()];
            for tool in &self.subtract {
                let mut next = Vec::new();
                for piece in &current {
                    next.extend(subtract_solid(piece, tool));
                }
                current = next;
                if current.is_empty() {
                    break;
                }
            }
            out.extend(current);
        }
        out
    }

    pub fn volume(&self) -> f64 {
        self.evaluate().iter().map(solid_volume).sum()
    }

    pub fn materials(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.solids {
            for f in &s.faces {
                if !out.contains(&f.material) {
                    out.push(f.material.clone());
                }
            }
        }
        out
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut acc: Option<(Vec3, Vec3)> = None;
        for s in &self.solids {
            let Some((lo, hi)) = solid_bounds(s) else { continue };
            match acc.as_mut() {
                None => acc = Some((lo, hi)),
                Some((amin, amax)) => {
                    for i in 0..3 {
                        amin[i] = amin[i].min(lo[i]);
                        amax[i] = amax[i].max(hi[i]);
                    }
                }
            }
        }
        acc
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrushMeshGroup {
    pub material: String,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,

    pub tangents: Vec<f32>,

    pub uv2: Vec<f32>,
    pub indices: Vec<u32>,
}

pub fn polygon_winding(poly: &[Vec3], n: Vec3) -> f64 {
    let mut sum = [0.0, 0.0, 0.0];
    for i in 0..poly.len() {
        sum = add(sum, cross(poly[i], poly[(i + 1) % poly.len()]));
    }
    dot(sum, n)
}

pub fn brush_mesh(brush: &BrushDef) -> Vec<BrushMeshGroup> {
    let layout = crate::brush_lightmap::brush_lightmap_layout(brush);
    brush_mesh_in_atlas(brush, &layout, 0)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrushPolygon {
    pub material: String,
    pub positions: Vec<[f32; 3]>,
    pub normal: [f32; 3],

    pub tangent: [f32; 4],
    pub uvs: Vec<[f32; 2]>,
    pub uv2: Vec<[f32; 2]>,
}

pub fn brush_polygons_in_atlas(
    brush: &BrushDef,
    layout: &crate::brush_lightmap::BrushLightmapLayout,
    object: usize,
) -> Vec<BrushPolygon> {
    let mut out = Vec::new();
    let solids = brush.evaluate();

    let exposed = exposed_fragments(&solids);
    for (si, solid) in solids.iter().enumerate() {
        for (i, poly) in exposed[si]
            .iter()
            .enumerate()
            .flat_map(|(i, frags)| frags.iter().map(move |f| (i, f.clone())))
        {
            let face = &solid.faces[i];
            let n = face.plane().n;
            let ordered: Vec<Vec3> = if polygon_winding(&poly, n) < 0.0 {
                poly.iter().rev().copied().collect()
            } else {
                poly
            };
            let (u_axis, v_axis) = face.axes();
            let handed = if dot(cross(n, u_axis), v_axis) < 0.0 { -1.0f32 } else { 1.0 };
            let mut p = BrushPolygon {
                material: face.material.clone(),
                normal: [n[0] as f32, n[1] as f32, n[2] as f32],
                tangent: [u_axis[0] as f32, u_axis[1] as f32, u_axis[2] as f32, handed],
                ..Default::default()
            };
            let chart = layout.chart(object, si, i);
            for q in &ordered {
                p.positions.push([q[0] as f32, q[1] as f32, q[2] as f32]);
                let uv = face.uv(*q);
                p.uvs.push([uv[0] as f32, uv[1] as f32]);
                p.uv2.push(match chart {
                    Some(c) => c.uv2(*q, layout.width, layout.height),
                    None => [0.0, 0.0],
                });
            }
            out.push(p);
        }
    }
    out
}

pub fn brush_mesh_in_atlas(
    brush: &BrushDef,
    layout: &crate::brush_lightmap::BrushLightmapLayout,
    object: usize,
) -> Vec<BrushMeshGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, BrushMeshGroup> = BTreeMap::new();

    for poly in brush_polygons_in_atlas(brush, layout, object) {
        if !groups.contains_key(&poly.material) {
            order.push(poly.material.clone());
            groups.insert(
                poly.material.clone(),
                BrushMeshGroup {
                    material: poly.material.clone(),
                    ..Default::default()
                },
            );
        }
        let g = groups.get_mut(&poly.material).expect("just inserted");
        let base = (g.positions.len() / 3) as u32;
        for k in 0..poly.positions.len() {
            g.positions.extend_from_slice(&poly.positions[k]);
            g.normals.extend_from_slice(&poly.normal);
            g.tangents.extend_from_slice(&poly.tangent);
            g.uvs.extend_from_slice(&poly.uvs[k]);
            g.uv2.extend_from_slice(&poly.uv2[k]);
        }
        for k in 1..poly.positions.len().saturating_sub(1) {
            g.indices.push(base);
            g.indices.push(base + k as u32);
            g.indices.push(base + k as u32 + 1);
        }
    }

    order
        .into_iter()
        .filter_map(|m| groups.remove(&m))
        .collect()
}

#[cfg(test)]
mod tangent_tests {
    use super::*;

    fn wall() -> BrushDef {
        BrushDef {
            solids: vec![block_solid([-2.0, 0.0, -0.15], [2.0, 2.0, 0.15], "concrete")],
            subtract: Vec::new(),
        }
    }

    #[test]
    fn every_vertex_gets_a_tangent() {
        let groups = brush_mesh(&wall());
        for g in &groups {
            assert_eq!(
                g.tangents.len() / 4,
                g.positions.len() / 3,
                "four tangent floats per vertex"
            );
        }
    }

    #[test]
    fn a_tangent_is_perpendicular_to_its_face() {
        for g in brush_mesh(&wall()) {
            for i in 0..g.positions.len() / 3 {
                let n = [g.normals[i * 3], g.normals[i * 3 + 1], g.normals[i * 3 + 2]];
                let t = [g.tangents[i * 4], g.tangents[i * 4 + 1], g.tangents[i * 4 + 2]];
                let d = n[0] * t[0] + n[1] * t[1] + n[2] * t[2];
                assert!(d.abs() < 1e-5, "tangent {t:?} is not in the face of {n:?}: {d}");
                let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-5, "tangent must be unit: {len}");
            }
        }
    }

    #[test]
    fn an_unauthored_box_has_one_consistent_handedness() {
        let mut signs = std::collections::HashSet::new();
        for g in brush_mesh(&wall()) {
            for i in 0..g.tangents.len() / 4 {
                signs.insert(g.tangents[i * 4 + 3].to_bits());
            }
        }
        assert_eq!(signs.len(), 1, "the defaults should agree with each other: {signs:?}");
        assert_eq!(
            f32::from_bits(*signs.iter().next().unwrap()),
            -1.0,
            "default_axes is left-handed; if this changed, every normal map moved"
        );
    }

    #[test]
    fn an_authored_flip_flips_the_reported_handedness() {

        let mut solid = block_solid([-2.0, 0.0, -0.15], [2.0, 2.0, 0.15], "concrete");
        let (u, v) = solid.faces[0].axes();
        solid.faces[0].u_axis = Some(u);
        solid.faces[0].v_axis = Some([-v[0], -v[1], -v[2]]);
        let flipped = BrushDef { solids: vec![solid], subtract: Vec::new() };

        let mut signs = std::collections::HashSet::new();
        for g in brush_mesh(&flipped) {
            for i in 0..g.tangents.len() / 4 {
                signs.insert(g.tangents[i * 4 + 3].to_bits());
            }
        }
        assert_eq!(
            signs.len(),
            2,
            "one flipped face must differ from the five that were not: {signs:?}"
        );
    }

    #[test]
    fn stepping_along_the_tangent_increases_u() {
        let solid = block_solid([-2.0, 0.0, -0.15], [2.0, 2.0, 0.15], "concrete");
        for face in &solid.faces {
            let (u_axis, _) = face.axes();
            let p = [0.0, 1.0, 0.0];
            let step = [p[0] + u_axis[0], p[1] + u_axis[1], p[2] + u_axis[2]];
            let a = face.uv(p);
            let b = face.uv(step);
            assert!(b[0] > a[0], "u must increase along the tangent: {a:?} -> {b:?}");
            assert!((b[1] - a[1]).abs() < 1e-9, "and v must not move: {a:?} -> {b:?}");
        }
    }
}

pub fn brush_checksum(brush: &BrushDef) -> u32 {
    let mut h: u32 = 2166136261;
    let mut mix = |x: f64| {

        let q = (x * 1000.0).round() as i32;
        for byte in 0..4 {
            h ^= ((q >> (byte * 8)) & 0xff) as u32;
            h = h.wrapping_mul(16777619);
        }
    };
    for g in brush_mesh(brush) {
        for x in &g.positions {
            mix(f64::from(*x));
        }
        for i in &g.indices {
            mix(f64::from(*i));
        }
    }
    h
}

pub fn block_solid(min: Vec3, max: Vec3, material: &str) -> BrushSolid {
    const NORMALS: [Vec3; 6] = [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    let faces = NORMALS
        .iter()
        .map(|n| {
            let corner = [
                if n[0] > 0.0 { max[0] } else { min[0] },
                if n[1] > 0.0 { max[1] } else { min[1] },
                if n[2] > 0.0 { max[2] } else { min[2] },
            ];
            let pl = Plane::new(*n, dot(*n, corner));
            let (u, v) = default_axes(pl.n);
            BrushFace {
                plane: [pl.n[0], pl.n[1], pl.n[2], pl.d],
                material: material.to_string(),
                u_axis: Some(u),
                v_axis: Some(v),
                scale: default_scale(),
                shift: [0.0, 0.0],
                rotation: 0.0,
                lightmap_scale: default_lightmap(),
                smoothing: 0,
            }
        })
        .collect();
    BrushSolid { faces }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> BrushSolid {
        block_solid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "default")
    }

    fn wall_with_door() -> BrushDef {
        BrushDef {
            solids: vec![block_solid([0.0, 0.0, 0.0], [4.0, 3.0, 0.3], "brick")],
            subtract: vec![block_solid([1.5, 0.0, -1.0], [2.5, 2.1, 1.0], "brick")],
        }
    }

    #[test]
    fn derives_six_quads_from_a_box() {
        let polys = solid_polygons(&unit());
        assert_eq!(polys.iter().flatten().count(), 6);
        for poly in polys.iter().flatten() {
            assert_eq!(poly.len(), 4);
        }
    }

    #[test]
    fn measures_volume() {
        assert!((solid_volume(&unit()) - 1.0).abs() < 1e-6);
        let b = block_solid([0.0, 0.0, 0.0], [2.0, 3.0, 4.0], "x");
        assert!((solid_volume(&b) - 24.0).abs() < 1e-4);
    }

    #[test]
    fn cuts_a_doorway_through_a_wall() {
        let brush = wall_with_door();
        let pieces = brush.evaluate();

        let expected = 4.0 * 3.0 * 0.3 - 1.0 * 2.1 * 0.3;
        assert!((brush.volume() - expected).abs() < 1e-3, "{}", brush.volume());

        assert!(!pieces.iter().any(|s| contains_point(s, [2.0, 1.0, 0.15], EPS)));
        assert!(pieces.iter().any(|s| contains_point(s, [0.5, 1.0, 0.15], EPS)));
        assert!(pieces.iter().any(|s| contains_point(s, [3.5, 1.0, 0.15], EPS)));

        assert!(pieces.iter().any(|s| contains_point(s, [2.0, 2.5, 0.15], EPS)));
    }

    #[test]
    fn a_tool_that_swallows_the_target_leaves_nothing() {
        let brush = BrushDef {
            solids: vec![unit()],
            subtract: vec![block_solid([-1.0, -1.0, -1.0], [2.0, 2.0, 2.0], "x")],
        };
        assert_eq!(brush.evaluate().len(), 0);
        assert!(brush.volume() < 1e-9);
    }

    #[test]
    fn every_triangle_is_wound_outward() {

        for g in brush_mesh(&wall_with_door()) {
            for t in g.indices.chunks(3) {
                let p = |i: u32| {
                    let i = i as usize * 3;
                    [
                        f64::from(g.positions[i]),
                        f64::from(g.positions[i + 1]),
                        f64::from(g.positions[i + 2]),
                    ]
                };
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let face = cross(sub(b, a), sub(c, a));
                let i = t[0] as usize * 3;
                let stored = [
                    f64::from(g.normals[i]),
                    f64::from(g.normals[i + 1]),
                    f64::from(g.normals[i + 2]),
                ];
                assert!(dot(face, stored) > 0.0, "triangle wound inward");
            }
        }
    }

    #[test]
    fn texture_scale_is_metres_per_tile() {
        let s = block_solid([0.0, 0.0, 0.0], [4.0, 1.0, 4.0], "x");
        let top = s.faces.iter().find(|f| f.plane().n[1] > 0.9).unwrap();
        let a = top.uv([0.0, 1.0, 0.0]);
        let b = top.uv([2.0, 1.0, 0.0]);

        assert!((b[0] - a[0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn opposing_walls_share_projection_axes_so_corners_line_up() {
        let (up, _) = default_axes([1.0, 0.0, 0.0]);
        let (un, _) = default_axes([-1.0, 0.0, 0.0]);
        for i in 0..3 {
            assert!((up[i].abs() - un[i].abs()).abs() < 1e-12);
        }
    }

    #[test]
    fn v_runs_downward_on_walls_so_lettering_is_upright() {
        let (_, v) = default_axes([0.0, 0.0, -1.0]);
        assert!(v[1] < 0.0);
    }

    #[test]
    fn groups_mesh_output_by_material() {
        let mut solid = block_solid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "wall");
        solid.faces[2].material = "floor".into();
        let groups = brush_mesh(&BrushDef {
            solids: vec![solid],
            subtract: vec![],
        });
        assert_eq!(groups.len(), 2);
        let wall = groups.iter().find(|g| g.material == "wall").unwrap();
        assert_eq!(wall.positions.len() / 3, 20);
    }

    #[test]
    fn round_trips_through_json() {
        let brush = wall_with_door();
        let text = serde_json::to_string(&brush).unwrap();
        let back: BrushDef = serde_json::from_str(&text).unwrap();
        assert_eq!(brush_checksum(&back), brush_checksum(&brush));
    }

    #[test]
    fn reads_a_face_written_without_texture_fields() {
        let face: BrushFace = serde_json::from_str(r#"{"plane":[0,1,0,1]}"#).unwrap();
        assert_eq!(face.material, "default");
        assert_eq!(face.scale, [2.0, 2.0]);
        assert_eq!(face.axes().0, default_axes([0.0, 1.0, 0.0]).0);
    }

    #[test]
    fn matches_editor_checksum() {
        let plain = BrushDef {
            solids: vec![unit()],
            subtract: vec![],
        };
        assert_eq!(
            brush_checksum(&plain),
            3876927263,
            "plain block disagrees with the editor"
        );

        assert_eq!(
            brush_checksum(&wall_with_door()),
            4259575697,
            "wall with a doorway disagrees with the editor"
        );
    }

    #[test]
    fn loads_a_brush_from_a_scene_file() {
        let dir = std::env::temp_dir().join(format!("brush_scene_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brushtest.json");

        std::fs::write(
            &path,
            r#"{
              "name": "brushtest",
              "objects": [{
                "id": "wall",
                "cuboid": { "position": [2, 1.5, 0.15], "half_size": [2, 1.5, 0.15] },
                "brush": {
                  "solids": [{ "faces": [
                    { "plane": [1,0,0,4], "material": "brick", "scale": [2,2] },
                    { "plane": [-1,0,0,0], "material": "brick", "scale": [2,2] },
                    { "plane": [0,1,0,3], "material": "brick", "scale": [2,2] },
                    { "plane": [0,-1,0,0], "material": "brick", "scale": [2,2] },
                    { "plane": [0,0,1,0.3], "material": "brick", "scale": [2,2] },
                    { "plane": [0,0,-1,0], "material": "brick", "scale": [2,2] }
                  ]}],
                  "subtract": [{ "faces": [
                    { "plane": [1,0,0,2.5], "material": "brick" },
                    { "plane": [-1,0,0,-1.5], "material": "brick" },
                    { "plane": [0,1,0,2.1], "material": "brick" },
                    { "plane": [0,-1,0,0], "material": "brick" },
                    { "plane": [0,0,1,1], "material": "brick" },
                    { "plane": [0,0,-1,1], "material": "brick" }
                  ]}]
                }
              }]
            }"#,
        )
        .unwrap();

        let scene = crate::scene::Scene::load(&path).expect("scene with a brush loads");
        let brush = scene.objects[0]
            .brush
            .as_ref()
            .expect("the brush survived the load");
        assert_eq!(brush.solids.len(), 1);
        assert_eq!(brush.subtract.len(), 1);
        assert_eq!(brush.materials(), vec!["brick".to_string()]);

        assert_eq!(brush.evaluate().len(), 3);
        assert!((brush.volume() - 2.97).abs() < 1e-6, "{}", brush.volume());

        let out = dir.join("resaved.json");
        scene.save(&out).expect("saves");
        let again = crate::scene::Scene::load(&out).expect("reloads");
        assert_eq!(
            brush_checksum(again.objects[0].brush.as_ref().unwrap()),
            brush_checksum(brush)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn matches_editor_piece_count_and_volume() {

        let brush = wall_with_door();
        assert_eq!(brush.evaluate().len(), 3);
        assert!((brush.volume() - 2.97).abs() < 1e-6, "{}", brush.volume());
    }
}

#[cfg(test)]
mod uv2_tests {
    use super::*;

    fn room() -> BrushDef {
        BrushDef {
            solids: vec![block_solid([-4.0, 0.0, -4.0], [4.0, 3.0, 4.0], "default")],
            subtract: Vec::new(),
        }
    }

    #[test]
    fn every_vertex_gets_a_lightmap_coordinate() {

        for g in brush_mesh(&room()) {
            assert_eq!(g.uv2.len() / 2, g.positions.len() / 3, "material {}", g.material);
        }
    }

    #[test]
    fn lightmap_coordinates_stay_inside_the_atlas() {
        for g in brush_mesh(&room()) {
            for (i, c) in g.uv2.iter().enumerate() {
                assert!((0.0..=1.0).contains(c), "uv2[{i}] = {c} is outside the atlas");
            }
        }
    }

    #[test]
    fn the_two_uv_sets_are_genuinely_different() {

        let g = &brush_mesh(&room())[0];
        assert_ne!(g.uvs, g.uv2, "lightmap uv is just the texture uv");
        assert!(
            g.uvs.iter().any(|v| *v > 1.0),
            "the fixture is too small to tile, so this cannot tell the two apart",
        );
    }

    #[test]
    fn two_faces_do_not_share_lightmap_space() {

        let brush = room();
        let layout = crate::brush_lightmap::brush_lightmap_layout(&brush);
        let mut seen = std::collections::HashSet::new();
        for c in &layout.charts {
            let uv = c.uv2(c.texel_world(c.w / 2, c.h / 2), layout.width, layout.height);
            let key = (
                (uv[0] * layout.width as f32) as u32,
                (uv[1] * layout.height as f32) as u32,
            );
            assert!(seen.insert(key), "two faces sample texel {key:?}");
        }
    }
}

#[cfg(test)]
mod exposed_surface_tests {
    use super::*;

    fn front_and_back(frag: &[Vec3], n: Vec3) -> (Vec3, Vec3) {
        let c = polygon_centroid(frag);
        (add(c, scale(n, 0.01)), sub(c, scale(n, 0.01)))
    }

    fn inside_any(solids: &[BrushSolid], p: Vec3) -> bool {
        solids.iter().any(|s| contains_point(s, p, -1e-4))
    }

    #[test]
    fn faces_two_pieces_press_together_are_gone() {
        let a = block_solid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "default");
        let b = block_solid([1.0, 0.0, 0.0], [2.0, 1.0, 1.0], "default");
        let solids = vec![a, b];
        let exposed = exposed_fragments(&solids);
        for (si, solid) in solids.iter().enumerate() {
            for (fi, face) in solid.faces.iter().enumerate() {
                let n = face.plane().n;
                for frag in &exposed[si][fi] {
                    let (front, _) = front_and_back(frag, n);
                    assert!(
                        !inside_any(&solids, front),
                        "solid {si} face {fi} survives facing INTO the other block",
                    );
                }
            }
        }
        let total: usize = exposed.iter().flatten().map(std::vec::Vec::len).sum();
        assert_eq!(total, 10, "a 2x1x1 box has 10 unit faces showing, not {total}");
    }

    #[test]
    fn a_face_is_trimmed_where_another_piece_covers_it() {

        let wall = block_solid([2.7, 0.0, -2.0], [3.0, 3.4, 2.0], "default");
        let ceiling = block_solid([-2.7, 3.1, -2.0], [2.7, 3.4, 2.0], "default");
        let solids = vec![wall, ceiling];
        let exposed = exposed_fragments(&solids);
        let inner = solids[0]
            .faces
            .iter()
            .position(|f| f.plane().n[0] < -0.9)
            .expect("the wall's inner face");
        let frags = &exposed[0][inner];
        assert!(!frags.is_empty(), "the inner face vanished entirely");
        let top = frags.iter().flatten().map(|p| p[1]).fold(f64::MIN, f64::max);

        assert!((top - 3.1).abs() < 1e-6, "inner wall face reaches y = {top}, not 3.1");
    }

    #[test]
    fn the_shipped_hall_draws_only_surface_that_borders_empty_space() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../game/scenes/test_room.json");
        let scene = crate::scene::Scene::load(std::path::Path::new(path)).expect("test_room loads");
        let hall = scene
            .objects
            .iter()
            .find(|o| o.id == "hall_shell")
            .and_then(|o| o.brush.as_ref())
            .expect("hall_shell brush");
        let solids = hall.evaluate();
        let exposed = exposed_fragments(&solids);
        let mut buried = 0;
        let mut ceiling_z = (f64::MAX, f64::MIN);
        for (si, solid) in solids.iter().enumerate() {
            for (fi, face) in solid.faces.iter().enumerate() {
                let n = face.plane().n;
                for frag in &exposed[si][fi] {
                    let (front, back) = front_and_back(frag, n);
                    if inside_any(&solids, front) || !inside_any(&solids, back) {
                        buried += 1;
                    }
                    if n[1] < -0.9 && (frag[0][1] - 3.1).abs() < 1e-6 {
                        for p in frag {
                            ceiling_z = (ceiling_z.0.min(p[2]), ceiling_z.1.max(p[2]));
                        }
                    }
                }
            }
        }
        assert_eq!(buried, 0, "{buried} drawn fragments face into solid or have nothing behind them");
        assert!(
            (ceiling_z.0 + 15.7).abs() < 1e-6 && (ceiling_z.1 - 3.7).abs() < 1e-6,
            "the hall ceiling spans z {ceiling_z:?}; the room is -15.7..3.7",
        );
    }
}
