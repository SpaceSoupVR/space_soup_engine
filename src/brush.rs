//! Brush solids: convex volumes defined by their planes.
//!
//! The runtime half of `scene_editor_web/frontend/src/lib/brush.js`. A brush is
//! authored as a set of planes, each carrying its own surface properties, and
//! the polygons are derived by clipping every plane against all the others.
//!
//! # This file is a mirror, and it is held to that
//!
//! Two independent implementations of a geometry algorithm drift silently, and
//! the failure mode is specific and nasty: a wall the level designer walked
//! around in the editor is one the player walks *through* in the headset,
//! because the two produced different triangles from the same planes. Nothing
//! errors. Nothing logs. The scene loads.
//!
//! The terrain triangulation learned this the expensive way and now carries
//! `REFERENCE_INDEX_CHECKSUM`. This module carries the same guard from the
//! start: [`brush_checksum`] is byte-identical arithmetic to the JavaScript,
//! and `tests::matches_editor_checksum` pins a reference brush's value. If a
//! change here alters the geometry, that test fails rather than the headset
//! quietly disagreeing with the editor.
//!
//! Everything is `f64` for the same reason. The editor computes in JavaScript
//! numbers, which are `f64`; doing the derivation in `f32` here would round
//! differently at every clip and the checksums would part company for reasons
//! that have nothing to do with a bug.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Tolerance for plane classification, and the grid vertices are snapped to.
///
/// Shared by both implementations. Changing it changes the geometry.
const EPS: f64 = 1e-5;

/// How far a seed polygon extends before it is clipped down to a real face.
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
    // `hypot3` rather than `sqrt(dot)` would be more accurate and would NOT
    // match `Math.hypot`, which is what the editor uses. Matching wins.
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

/// A plane as `dot(n, p) = d`, with `n` pointing OUT of the solid.
///
/// A point is inside when `dot(n, p) <= d`. Inverting that convention turns a
/// solid inside out, and it renders only when the camera is inside it.
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

/// One face of a brush: a plane plus how a material sits on it.
///
/// `scale` is METRES PER TILE, not texels per unit as in the Quake and Source
/// lineage. That makes a surface's appearance independent of the material's
/// pixel dimensions, so replacing a 1K texture with a 4K one does not silently
/// rescale every wall it is used on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushFace {
    /// `[nx, ny, nz, d]`.
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
    0.25
}

impl BrushFace {
    pub fn plane(&self) -> Plane {
        Plane {
            n: [self.plane[0], self.plane[1], self.plane[2]],
            d: self.plane[3],
        }
    }

    /// The projection axes, falling back to the by-direction defaults.
    pub fn axes(&self) -> (Vec3, Vec3) {
        let (du, dv) = default_axes(self.plane().n);
        let u = self.u_axis.unwrap_or(du);
        let v = self.v_axis.unwrap_or(dv);
        rotate_axes(u, v, self.plane().n, self.rotation)
    }

    /// Where a world point lands in this face's texture, in tiles.
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

/// Default projection axes by which world direction a face mostly points.
///
/// Six discrete cases rather than a continuous basis, so that two faces of the
/// same wall get identical axes and their brickwork lines up across the corner.
/// Y-up, matching glTF and the renderer -- NOT the Z-up table you will find in
/// any Quake or Source reference.
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
    // Rodrigues, simplified: u and v are perpendicular to n already, so the
    // dot(n, u) * n term is zero and this is a plain rotation in the plane.
    let rot = |a: Vec3| add(scale(a, c), scale(cross(n, a), s));
    (rot(u), rot(v))
}

/// A convex solid: its faces, and nothing else. There is no vertex list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushSolid {
    pub faces: Vec<BrushFace>,
}

/// What a `GameObject` stores: the solids that make the shape, and the tool
/// solids carved out of them.
///
/// Keeping the cutting tools rather than baking their result is the
/// non-destructive half of the CSG design. The runtime evaluates them at load;
/// see `evaluate`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrushDef {
    #[serde(default)]
    pub solids: Vec<BrushSolid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtract: Vec<BrushSolid>,
}

/* -------------------------------------------------------------- geometry -- */

fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
    // The LEAST aligned world axis. Crossing with the most-aligned one gives a
    // zero-length vector and a NaN basis, on precisely the axis-aligned faces
    // that make up almost every building.
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

/// Sutherland-Hodgman, keeping `dot(n, p) <= d`.
///
/// The epsilon is applied to the classification, not the output, so a vertex
/// lying exactly on the cutting plane counts as on both sides and survives.
/// Dropping it is how clipping against a coincident plane silently deletes a
/// face.
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
    // `Math.round` in the editor rounds half AWAY FROM ZERO; Rust's `round`
    // does the same. `f64::round_ties_even` does not, and would part the two
    // implementations on exactly the values that sit on the grid.
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

/// The polygon of each face, or `None` where a plane bounds nothing.
///
/// A redundant plane -- one entirely outside the volume the others describe --
/// is normal rather than an error, and is very common after a subtraction.
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

/// Volume, via the divergence theorem over the face polygons.
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

/* ------------------------------------------------------------------- CSG -- */

fn face_inheriting(pl: Plane, from: Option<&BrushFace>) -> BrushFace {
    let (u, v) = default_axes(pl.n);
    BrushFace {
        plane: [pl.n[0], pl.n[1], pl.n[2], pl.d],
        material: from.map(|f| f.material.clone()).unwrap_or_else(default_material),
        u_axis: Some(u),
        v_axis: Some(v),
        scale: from.map(|f| f.scale).unwrap_or_else(default_scale),
        shift: from.map(|f| f.shift).unwrap_or([0.0, 0.0]),
        rotation: 0.0,
        lightmap_scale: from.map(|f| f.lightmap_scale).unwrap_or_else(default_lightmap),
        smoothing: 0,
    }
}

/// Cut a solid with a plane. The new surface inherits the material of whichever
/// existing face the cut is most parallel to, so slicing a brick wall gives two
/// brick walls rather than two walls with an untextured cross-section.
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

/// `target` minus `tool`, as disjoint convex pieces.
///
/// Peels off the part of the target outside each tool plane in turn; what
/// survives every plane is inside the tool and is discarded. An empty result
/// means the tool swallowed the target, which is the honest answer and not a
/// reason to hand back the target untouched.
pub fn subtract_solid(target: &BrushSolid, tool: &BrushSolid) -> Vec<BrushSolid> {
    if !is_valid_solid(target) {
        return Vec::new();
    }
    if !is_valid_solid(tool) {
        return vec![target.clone()];
    }
    // Bounds rejection first: the polygon derivation is quadratic and runs
    // several times per plane below, and most brush pairs in a level never
    // touch.
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
    /// The convex pieces this brush actually contributes to the level.
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

    /// Every material this brush needs loaded.
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

/* --------------------------------------------------------------- meshing -- */

/// Render data for one material.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrushMeshGroup {
    pub material: String,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    /// The face's u axis per vertex, with `w` carrying the bitangent's
    /// handedness -- four floats each.
    ///
    /// Emitted here rather than derived from the triangles' uvs by whoever
    /// renders this. The usual derivation is an approximation that goes
    /// degenerate on thin triangles, and it would be approximating something
    /// this data knows exactly: `uv` was GENERATED from these axes a few lines
    /// below, so anything that has to undo that is reconstructing an input.
    pub tangents: Vec<f32>,
    /// Lightmap coordinate, 0..1 over the brush's own atlas -- two floats each.
    ///
    /// A SECOND set, unrelated to `uvs`. Texture uv is in tiles and shared
    /// between faces so brickwork lines up across a corner; lighting needs one
    /// unshared patch per face, or two walls sample each other's shadows. See
    /// `brush_lightmap`, which decides the layout for the baker and for this
    /// together so the two cannot disagree about which texel is which surface.
    pub uv2: Vec<f32>,
    pub indices: Vec<u32>,
}

/// Positive when a polygon is wound counter-clockwise as seen from `n`'s side.
pub fn polygon_winding(poly: &[Vec3], n: Vec3) -> f64 {
    let mut sum = [0.0, 0.0, 0.0];
    for i in 0..poly.len() {
        sum = add(sum, cross(poly[i], poly[(i + 1) % poly.len()]));
    }
    dot(sum, n)
}

/// Triangulate a brush into render groups, one per material.
///
/// Vertices are NOT shared between faces even where they coincide: a brush's
/// faces meet at hard edges and need distinct normals, and sharing would
/// average them into a bevel that is not there.
///
/// Groups come back in first-seen order rather than sorted, because
/// [`brush_checksum`] hashes them in order and the editor's `Map` preserves
/// insertion order. Sorting here would be tidier and would break the parity
/// guarantee.
pub fn brush_mesh(brush: &BrushDef) -> Vec<BrushMeshGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, BrushMeshGroup> = BTreeMap::new();
    let layout = crate::brush_lightmap::brush_lightmap_layout(brush);

    for (si, solid) in brush.evaluate().into_iter().enumerate() {
        let polys = solid_polygons(&solid);
        for (i, poly) in polys.into_iter().enumerate() {
            let Some(poly) = poly else { continue };
            let face = &solid.faces[i];
            let material = face.material.clone();
            if !groups.contains_key(&material) {
                order.push(material.clone());
                groups.insert(
                    material.clone(),
                    BrushMeshGroup {
                        material: material.clone(),
                        ..Default::default()
                    },
                );
            }
            let g = groups.get_mut(&material).expect("just inserted");
            let base = (g.positions.len() / 3) as u32;
            let n = face.plane().n;
            // Measure the winding rather than assuming it. The seed polygon's
            // handedness relative to the outward normal is not guaranteed, and
            // assuming is how a solid comes out invisible from outside.
            let ordered: Vec<Vec3> = if polygon_winding(&poly, n) < 0.0 {
                poly.iter().rev().copied().collect()
            } else {
                poly
            };
            let (u_axis, v_axis) = face.axes();
            // Whether v runs the same way as n x u. Measured rather than
            // assumed: `default_axes` happens to be consistently left-handed
            // for all six directions, but a face whose axes were AUTHORED --
            // which is what aligning a texture in the editor writes -- can be
            // either. Getting it wrong flips the green channel of that face's
            // normal map, which reads as light arriving from the wrong side
            // rather than as a handedness bug.
            let handed = if dot(cross(n, u_axis), v_axis) < 0.0 { -1.0f32 } else { 1.0 };
            for p in &ordered {
                g.positions.push(p[0] as f32);
                g.positions.push(p[1] as f32);
                g.positions.push(p[2] as f32);
                g.normals.push(n[0] as f32);
                g.normals.push(n[1] as f32);
                g.normals.push(n[2] as f32);
                g.tangents.push(u_axis[0] as f32);
                g.tangents.push(u_axis[1] as f32);
                g.tangents.push(u_axis[2] as f32);
                g.tangents.push(handed);
                let uv = face.uv(*p);
                g.uvs.push(uv[0] as f32);
                g.uvs.push(uv[1] as f32);
                // A face with no chart -- degenerate, or dropped by the packer
                // -- samples the atlas's first texel. That is a real texel of
                // this brush's own bake rather than an out-of-range read, so it
                // is lit plausibly instead of black or garbage.
                let uv2 = match layout.chart(si, i) {
                    Some(c) => c.uv2(*p, layout.width, layout.height),
                    None => [0.0, 0.0],
                };
                g.uv2.push(uv2[0]);
                g.uv2.push(uv2[1]);
            }
            for k in 1..ordered.len().saturating_sub(1) {
                g.indices.push(base);
                g.indices.push(base + k as u32);
                g.indices.push(base + k as u32 + 1);
            }
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

    /// The tangent must lie IN the face, not across it.
    ///
    /// A tangent with a component along the normal tilts the whole frame, so a
    /// normal map would perturb the surface in a direction the surface does not
    /// have -- lighting that is subtly wrong everywhere rather than obviously
    /// wrong anywhere.
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

    /// Handedness is measured, not assumed.
    ///
    /// `default_axes` turns out to be consistently left-handed for all six
    /// directions, so an unauthored box shows one sign -- pinned here so that a
    /// change to those defaults is noticed rather than silently flipping every
    /// wall's normal map. What is NOT fixed is an AUTHORED axis pair, which is
    /// what aligning a texture in the editor writes, and which the next test
    /// covers.
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
        // The case the measurement exists for. Reversing v mirrors the basis,
        // and a renderer told otherwise lights the face from the wrong side.
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

    /// The tangent is the axis the uv was generated from, not a guess at it.
    ///
    /// Stepping along the tangent must increase u and leave v alone. If the two
    /// ever disagreed, a normal map would be applied rotated relative to the
    /// texture it belongs to.
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

/// A stable fingerprint of the evaluated geometry.
///
/// FNV-1a over the quantised positions then the indices of each material group,
/// in group order -- byte-for-byte the same arithmetic as `brushChecksum` in
/// the editor. This is the guard against the two implementations drifting; see
/// the module header for what drift actually costs.
pub fn brush_checksum(brush: &BrushDef) -> u32 {
    let mut h: u32 = 2166136261;
    let mut mix = |x: f64| {
        // `as i32` after rounding, matching the editor's `| 0`, so that both
        // wrap the same way on a value no sane brush will ever reach.
        let q = (x * 1000.0).round() as i32;
        for byte in 0..4 {
            h ^= ((q >> (byte * 8)) & 0xff) as u32;
            h = h.wrapping_mul(16777619);
        }
    };
    for g in brush_mesh(brush) {
        for x in &g.positions {
            mix(*x as f64);
        }
        for i in &g.indices {
            mix(*i as f64);
        }
    }
    h
}

/* ------------------------------------------------------------ primitives -- */

/// An axis-aligned box. Used by the tests, and by anything constructing level
/// geometry from code rather than from a file.
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
        // Wall minus the part of the doorway inside it.
        let expected = 4.0 * 3.0 * 0.3 - 1.0 * 2.1 * 0.3;
        assert!((brush.volume() - expected).abs() < 1e-3, "{}", brush.volume());
        // The doorway is genuinely open, and the wall either side is not.
        assert!(!pieces.iter().any(|s| contains_point(s, [2.0, 1.0, 0.15], EPS)));
        assert!(pieces.iter().any(|s| contains_point(s, [0.5, 1.0, 0.15], EPS)));
        assert!(pieces.iter().any(|s| contains_point(s, [3.5, 1.0, 0.15], EPS)));
        // And the lintel above it is.
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
        // A brush that renders only from inside is always a winding assumption
        // nobody measured. This measures it.
        for g in brush_mesh(&wall_with_door()) {
            for t in g.indices.chunks(3) {
                let p = |i: u32| {
                    let i = i as usize * 3;
                    [
                        g.positions[i] as f64,
                        g.positions[i + 1] as f64,
                        g.positions[i + 2] as f64,
                    ]
                };
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let face = cross(sub(b, a), sub(c, a));
                let i = t[0] as usize * 3;
                let stored = [
                    g.normals[i] as f64,
                    g.normals[i + 1] as f64,
                    g.normals[i + 2] as f64,
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
        // Default scale is 2 metres per tile, so two metres is exactly one tile.
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
        assert_eq!(wall.positions.len() / 3, 20); // five quads
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

    /// The parity guard. See the module header for what drift costs.
    ///
    /// These values come from running `brushChecksum` in the editor over the
    /// same two brushes. If this test fails after a change here, this
    /// implementation and the editor's now produce different triangles from the
    /// same planes, and a level will be a different shape in the headset than
    /// it was on screen.
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
            2111997196,
            "wall with a doorway disagrees with the editor"
        );
    }

    /// The editor writes a file; the engine has to read that exact file.
    ///
    /// Checksum parity proves the two AGREE about geometry. This proves the
    /// scene format actually carries a brush from one to the other -- a
    /// serialization mismatch would make every brush silently vanish on load
    /// with the object still present, which looks like a rendering bug and is
    /// not one.
    #[test]
    fn loads_a_brush_from_a_scene_file() {
        let dir = std::env::temp_dir().join(format!("brush_scene_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brushtest.json");
        // Written in the editor's on-disk shape: camelCase texture fields,
        // planes as four numbers, `subtract` present.
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
        // Same doorway, same three pieces, same volume as every other test here.
        assert_eq!(brush.evaluate().len(), 3);
        assert!((brush.volume() - 2.97).abs() < 1e-6, "{}", brush.volume());

        // And a save/load cycle through the engine is lossless.
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
        // Also pinned from the editor: three convex pieces, 2.97 cubic metres.
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
        // One missing pair silently shifts every following vertex's lighting by
        // one, which looks like a subtly wrong bake rather than a buffer that is
        // the wrong length.
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
        // Texture uv is in TILES and repeats; a lightmap coordinate is 0..1 and
        // must not. If these ever came out equal, the lighting would be tiled
        // across the wall along with the brickwork.
        let g = &brush_mesh(&room())[0];
        assert_ne!(g.uvs, g.uv2, "lightmap uv is just the texture uv");
        assert!(
            g.uvs.iter().any(|v| *v > 1.0),
            "the fixture is too small to tile, so this cannot tell the two apart",
        );
    }

    #[test]
    fn two_faces_do_not_share_lightmap_space() {
        // The whole point of a second uv set. Sampled at their centres, no two
        // faces of a box may land on the same texel.
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
