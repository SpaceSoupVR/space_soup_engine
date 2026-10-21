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

/// One face of a brush: of a solid, or of a carve (`subtract`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceRef {
    pub carve: bool,
    pub solid: usize,
    pub face: usize,
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

impl BrushSolid {
    /// An axis-aligned box about `centre`, `half` its half extents, every
    /// face in `material` at the default texture scale (the web editor's
    /// new-brush box).
    pub fn cuboid(centre: Vec3, half: Vec3, material: &str) -> Self {
        let faces = (0..6)
            .map(|i| {
                let (axis, sign) = (i / 2, if i % 2 == 0 { 1.0 } else { -1.0 });
                let mut n = [0.0; 3];
                n[axis] = sign;
                BrushFace {
                    plane: [n[0], n[1], n[2], sign * centre[axis] + half[axis]],
                    material: material.to_string(),
                    u_axis: None,
                    v_axis: None,
                    scale: default_scale(),
                    shift: [0.0; 2],
                    rotation: 0.0,
                    lightmap_scale: default_lightmap(),
                    smoothing: 0,
                }
            })
            .collect();
        BrushSolid { faces }
    }

    /// The solid moved by `delta`, its textures with it: each face's shift
    /// follows, so a moved wall keeps its brickwork rather than sliding
    /// through it (texture lock, always on, as the web editor's
    /// `translateSolid` had it).
    pub fn translate(&mut self, delta: Vec3) {
        for f in &mut self.faces {
            let (u, v) = f.axes();
            let su = if f.scale[0] == 0.0 { 1.0 } else { f.scale[0] };
            let sv = if f.scale[1] == 0.0 { 1.0 } else { f.scale[1] };
            f.plane[3] += dot([f.plane[0], f.plane[1], f.plane[2]], delta);
            f.shift[0] -= dot(delta, u) / su;
            f.shift[1] -= dot(delta, v) / sv;
        }
    }
}

impl BrushDef {
    /// Every solid and carve moved by `delta`, textures locked (see
    /// `BrushSolid::translate`). A brush's planes are in the world, so moving
    /// its object's `cuboid` alone moves nothing but the box that reports it.
    pub fn translate(&mut self, delta: Vec3) {
        for s in self.solids.iter_mut().chain(self.subtract.iter_mut()) {
            s.translate(delta);
        }
    }

    /// Every solid and carve carried through x' = `a` x + `t` (`a` rows, any
    /// invertible map: a turn, a resize, both). See `BrushSolid::transform`.
    pub fn transform(&mut self, a: [[f64; 3]; 3], t: Vec3, anchor: Vec3, stretch_textures: bool) {
        for s in self.solids.iter_mut().chain(self.subtract.iter_mut()) {
            s.transform(a, t, anchor, stretch_textures);
        }
    }
}

fn mat_vec(m: [[f64; 3]; 3], v: Vec3) -> Vec3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

/// The transpose of the inverse (the map normals and texture axes take).
fn inverse_transpose(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    // Rows of the cofactor matrix over the determinant are the rows of
    // the inverse's transpose.
    let c0 = cross(m[1], m[2]);
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    let det = dot(m[0], c0);
    (det.abs() > 1e-12).then(|| [scale(c0, 1.0 / det), scale(c1, 1.0 / det), scale(c2, 1.0 / det)])
}

impl BrushSolid {
    /// THE SOLID CARRIED THROUGH x' = `a` x + `t`, its textures with it.
    /// Each plane takes the inverse transpose; each face's texture axes are
    /// made explicit (the defaults follow the normal's largest axis, and a
    /// turned face would jump to another). With `stretch_textures` the
    /// texture is fixed to the solid and stretches with a resize, as a turn
    /// carries it; without, a resize keeps the texture's size in the world,
    /// pinned where `anchor` (the resize's fixed point) projects onto each
    /// face, as the web editor's `resizeBrushToCuboid` did.
    pub fn transform(&mut self, a: [[f64; 3]; 3], t: Vec3, anchor: Vec3, stretch_textures: bool) {
        let Some(it) = inverse_transpose(a) else { return };
        for f in &mut self.faces {
            let pl = f.plane();
            let (u, v) = f.axes();
            let su = if f.scale[0] == 0.0 { 1.0 } else { f.scale[0] };
            let sv = if f.scale[1] == 0.0 { 1.0 } else { f.scale[1] };
            // Where the anchor meets this face, and where that goes.
            let on = sub(anchor, scale(pl.n, pl.distance(anchor)));
            let on2 = add(mat_vec(a, on), t);
            let n2 = normalize(mat_vec(it, pl.n));
            f.plane = [n2[0], n2[1], n2[2], dot(n2, on2)];
            let (ut, vt) = (mat_vec(it, u), mat_vec(it, v));
            let (lu, lv) = (dot(ut, ut).sqrt().max(1e-12), dot(vt, vt).sqrt().max(1e-12));
            let (u2, v2) = (scale(ut, 1.0 / lu), scale(vt, 1.0 / lv));
            if stretch_textures {
                // uv(x) = x.u / s + shift, and x = a^-1 (x' - t).
                f.shift[0] -= dot(t, ut) / su;
                f.shift[1] -= dot(t, vt) / sv;
                f.scale = [su / lu, sv / lv];
            } else {
                f.shift[0] += (dot(on, u) - dot(on2, u2)) / su;
                f.shift[1] += (dot(on, v) - dot(on2, v2)) / sv;
            }
            f.u_axis = Some(u2);
            f.v_axis = Some(v2);
            f.rotation = 0.0;
        }
    }
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

    /// THE FACE A POINT ON THE BRUSH'S SURFACE LIES ON: of the solids and
    /// carves holding `p` (within `eps` of every plane), the face whose
    /// plane it is nearest, if that is within `eps`. A carve's face is the
    /// wall the carve leaves, so editing it edits what is seen there.
    pub fn face_ref_at(&self, p: Vec3, eps: f64) -> Option<FaceRef> {
        let mut best: Option<(f64, FaceRef)> = None;
        for (carve, list) in [(false, &self.solids), (true, &self.subtract)] {
            for (solid, s) in list.iter().enumerate() {
                let d: Vec<f64> = s.faces.iter().map(|f| f.plane().distance(p)).collect();
                if d.iter().any(|x| *x > eps) {
                    continue;
                }
                for (face, x) in d.iter().enumerate() {
                    let x = x.abs();
                    if x <= eps && best.is_none_or(|b| x < b.0) {
                        best = Some((x, FaceRef { carve, solid, face }));
                    }
                }
            }
        }
        best.map(|(_, r)| r)
    }

    /// `face_ref_at`'s face itself.
    pub fn face_at_mut(&mut self, p: Vec3, eps: f64) -> Option<&mut BrushFace> {
        let r = self.face_ref_at(p, eps)?;
        self.face_mut(r)
    }

    pub fn face(&self, r: FaceRef) -> Option<&BrushFace> {
        let list = if r.carve { &self.subtract } else { &self.solids };
        list.get(r.solid)?.faces.get(r.face)
    }

    pub fn face_mut(&mut self, r: FaceRef) -> Option<&mut BrushFace> {
        let list = if r.carve { &mut self.subtract } else { &mut self.solids };
        list.get_mut(r.solid)?.faces.get_mut(r.face)
    }

    /// THE FACE PUSHED `by` METRES along its outward normal (pulled when
    /// negative): its solid grows or shrinks on that side alone, as a face
    /// drag in a brush editor. Refused (false, nothing changed) when the
    /// solid would be left with no volume, or a face it had would vanish.
    pub fn push_face(&mut self, r: FaceRef, by: f64) -> bool {
        let list = if r.carve { &mut self.subtract } else { &mut self.solids };
        let Some(solid) = list.get_mut(r.solid) else { return false };
        if r.face >= solid.faces.len() {
            return false;
        }
        let before = solid.clone();
        solid.faces[r.face].plane[3] += by;
        let kept = solid_polygons(solid).iter().all(|p| p.as_ref().is_some_and(|p| p.len() >= 3));
        if !kept || solid_volume(solid) <= 1e-9 {
            *solid = before;
            return false;
        }
        true
    }

    /// Carve `index` (of `subtract`) reshaped to the box `centre` +- `half`,
    /// through its planes about its own box (`BrushSolid::transform`), so its
    /// walls keep their texture's size in the world. False when there is no
    /// such carve, it has no volume, or the size asked is not positive.
    pub fn reshape_carve(&mut self, index: usize, centre: Vec3, half: Vec3) -> bool {
        let Some(carve) = self.subtract.get_mut(index) else { return false };
        let Some((lo, hi)) = solid_bounds(carve) else { return false };
        let was_half = [0, 1, 2].map(|i| (hi[i] - lo[i]) / 2.0);
        if was_half.iter().chain(half.iter()).any(|h| *h <= 1e-6) {
            return false;
        }
        let was_centre = [0, 1, 2].map(|i| (hi[i] + lo[i]) / 2.0);
        let k = [0, 1, 2].map(|i| half[i] / was_half[i]);
        let a = [[k[0], 0.0, 0.0], [0.0, k[1], 0.0], [0.0, 0.0, k[2]]];
        // x' = k (x - c) + c'.
        let t = [0, 1, 2].map(|i| centre[i] - k[i] * was_centre[i]);
        carve.transform(a, t, was_centre, false);
        true
    }

    /// Every face of every solid in `material` (carves keep theirs: they
    /// draw nothing of their own).
    pub fn paint(&mut self, material: &str) {
        for f in self.solids.iter_mut().flat_map(|s| s.faces.iter_mut()) {
            f.material = material.to_string();
        }
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
            mix(*x as f64);
        }
        for i in &g.indices {
            mix(*i as f64);
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
        let total: usize = exposed.iter().flatten().map(|f| f.len()).sum();
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
            "the hall ceiling spans z {:?}; the room is -15.7..3.7",
            ceiling_z,
        );
    }
}

#[cfg(test)]
mod translate_tests {
    use super::*;

    /// A moved brush's faces pass through the moved points and show them the
    /// same texture coordinates they had: the plane and the texture move
    /// together, a rotated texture included.
    #[test]
    fn a_moved_brush_carries_its_planes_and_its_texture() {
        let face = |n: Vec3, d: f64, rotation: f64| BrushFace {
            plane: [n[0], n[1], n[2], d],
            material: "m".into(),
            u_axis: None,
            v_axis: None,
            scale: [2.0, 0.5],
            shift: [0.25, 0.0],
            rotation,
            lightmap_scale: 0.125,
            smoothing: 0,
        };
        let mut brush = BrushDef {
            solids: vec![BrushSolid { faces: vec![face([1.0, 0.0, 0.0], 3.0, 0.0), face([0.0, 0.0, -1.0], 2.0, 30.0)] }],
            subtract: vec![BrushSolid { faces: vec![face([0.0, 1.0, 0.0], 1.0, 0.0)] }],
        };
        let before = brush.clone();
        let delta = [1.5, -0.25, 4.0];
        brush.translate(delta);
        let all = |b: &BrushDef| b.solids.iter().chain(b.subtract.iter()).flat_map(|s| s.faces.clone()).collect::<Vec<_>>();
        for (old, new) in all(&before).iter().zip(all(&brush).iter()) {
            // A point on the old face, and where it went.
            let n = [old.plane[0], old.plane[1], old.plane[2]];
            let p = scale(n, old.plane[3]);
            let q = add(p, delta);
            assert!(new.plane().distance(q).abs() < 1e-9, "the face did not move with the brush");
            let (a, b) = (old.uv(p), new.uv(q));
            assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "the texture slid: {a:?} -> {b:?}");
        }
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;

    fn cube() -> BrushDef {
        let face = |n: Vec3, d: f64| BrushFace {
            plane: [n[0], n[1], n[2], d],
            material: "default".into(),
            u_axis: None,
            v_axis: None,
            scale: [2.0, 2.0],
            shift: [0.25, 0.5],
            rotation: 0.0,
            lightmap_scale: 0.125,
            smoothing: 0,
        };
        let faces = vec![
            face([1.0, 0.0, 0.0], 1.0),
            face([-1.0, 0.0, 0.0], 1.0),
            face([0.0, 1.0, 0.0], 1.0),
            face([0.0, -1.0, 0.0], 1.0),
            face([0.0, 0.0, 1.0], 1.0),
            face([0.0, 0.0, -1.0], 1.0),
        ];
        BrushDef { solids: vec![BrushSolid { faces }], subtract: Vec::new() }
    }

    fn turn_y(deg: f64) -> [[f64; 3]; 3] {
        let (s, c) = deg.to_radians().sin_cos();
        [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
    }

    #[test]
    fn a_turned_brush_turns_its_planes_and_carries_its_texture() {
        let mut b = cube();
        let before = b.clone();
        let a = turn_y(30.0);
        let centre = [2.0, 0.0, -1.0];
        let t = sub(centre, mat_vec(a, centre));
        b.transform(a, t, centre, true);
        for (f0, f1) in before.solids[0].faces.iter().zip(&b.solids[0].faces) {
            // A point on the old face, carried, lies on the new face with the same uv.
            let p = scale(f0.plane().n, f0.plane[3]);
            let p = add(p, scale(f0.axes().0, 0.3));
            let q = add(mat_vec(a, p), t);
            assert!(f1.plane().distance(q).abs() < 1e-9, "{:?}", f1.plane);
            let (uv0, uv1) = (f0.uv(p), f1.uv(q));
            assert!((uv0[0] - uv1[0]).abs() < 1e-9 && (uv0[1] - uv1[1]).abs() < 1e-9, "{uv0:?} vs {uv1:?}");
        }
    }

    #[test]
    fn a_resized_brush_keeps_its_texture_the_same_size_in_the_world() {
        let mut b = cube();
        let a = [[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        b.transform(a, [0.0; 3], [0.0; 3], false);
        let faces = &b.solids[0].faces;
        // The +x face moved out to x = 2; the floor stayed and grew.
        assert!((faces[0].plane[3] - 2.0).abs() < 1e-9);
        assert!((faces[2].plane[3] - 1.0).abs() < 1e-9);
        // The top face's texture repeats every 2 m as it did, pinned at the anchor's foot.
        let f = &faces[2];
        let (a0, a1) = (f.uv([0.0, 1.0, 0.0]), f.uv([2.0, 1.0, 0.0]));
        assert!(((a1[0] - a0[0]).abs() - 1.0).abs() < 1e-9, "{a0:?} {a1:?}");
        assert!((a0[0] - cube().solids[0].faces[2].uv([0.0, 1.0, 0.0])[0]).abs() < 1e-9);
    }

    #[test]
    fn a_new_box_brush_is_the_box_it_was_asked_for() {
        let mut b = BrushDef { solids: vec![BrushSolid::cuboid([1.0, 2.0, -3.0], [0.5, 1.0, 2.0], "Bricks075A")], subtract: Vec::new() };
        let (lo, hi) = b.bounds().unwrap();
        for (x, y) in lo.iter().zip([0.5, 1.0, -5.0]).chain(hi.iter().zip([1.5, 3.0, -1.0])) {
            assert!((x - y).abs() < 1e-9, "{lo:?} {hi:?}");
        }
        assert!((b.volume() - 8.0).abs() < 1e-6);
        assert_eq!(b.materials(), vec!["Bricks075A".to_string()]);
        b.paint("Marble020");
        assert_eq!(b.materials(), vec!["Marble020".to_string()]);
    }

    #[test]
    fn a_point_on_a_face_finds_that_face() {
        let mut b = BrushDef {
            solids: vec![BrushSolid::cuboid([0.0, 1.0, 0.0], [2.0, 1.0, 2.0], "Bricks075A")],
            subtract: vec![BrushSolid::cuboid([0.0, 1.0, 0.0], [1.0, 0.5, 1.0], "Rock063")],
        };
        // The top of the outer box, and the inside of the carve's floor.
        b.face_at_mut([1.5, 2.0, 0.3], 1e-3).unwrap().material = "Marble020".into();
        assert_eq!(b.solids[0].faces[2].material, "Marble020");
        assert_eq!(b.face_at_mut([0.2, 0.5, 0.2], 1e-3).unwrap().plane, [0.0, -1.0, 0.0, -0.5]);
        assert!(b.face_at_mut([0.0, 3.0, 0.0], 1e-3).is_none(), "a point off the brush finds nothing");
    }

    #[test]
    fn a_reshaped_carve_is_the_box_asked_for_and_cuts_its_brush() {
        let mut b = BrushDef {
            solids: vec![BrushSolid::cuboid([0.0, 1.5, 0.0], [3.0, 1.5, 3.0], "Bricks075A")],
            subtract: vec![BrushSolid::cuboid([0.0, 1.5, 0.0], [2.8, 1.3, 2.8], "Marble020")],
        };
        assert!(b.reshape_carve(0, [1.0, 1.2, 0.0], [0.5, 1.0, 3.5]));
        let (lo, hi) = solid_bounds(&b.subtract[0]).unwrap();
        for (x, y) in lo.iter().zip([0.5, 0.2, -3.5]).chain(hi.iter().zip([1.5, 2.2, 3.5])) {
            assert!((x - y).abs() < 1e-9, "{lo:?} {hi:?}");
        }
        // A 1 x 2 m doorway through the whole 6 m brush.
        assert!((b.volume() - (6.0 * 3.0 * 6.0 - 1.0 * 2.0 * 6.0)).abs() < 1e-6, "{}", b.volume());
        assert_eq!(b.subtract[0].faces[0].material, "Marble020");
        assert!(!b.reshape_carve(1, [0.0; 3], [1.0; 3]), "no such carve");
        assert!(!b.reshape_carve(0, [0.0; 3], [0.0, 1.0, 1.0]), "no flat carves");
    }

    #[test]
    fn a_pushed_face_moves_its_side_alone_and_cannot_turn_the_solid_inside_out() {
        let mut b = BrushDef { solids: vec![BrushSolid::cuboid([0.0, 1.0, 0.0], [1.0, 1.0, 1.0], "Bricks075A")], subtract: Vec::new() };
        let top = b.face_ref_at([0.2, 2.0, 0.3], 1e-6).unwrap();
        assert_eq!(top, FaceRef { carve: false, solid: 0, face: 2 });
        assert!(b.push_face(top, 0.5));
        assert!((b.volume() - 2.0 * 2.0 * 2.5).abs() < 1e-9 && (b.bounds().unwrap().1[1] - 2.5).abs() < 1e-9);
        assert!(b.push_face(top, -1.5));
        assert!((b.bounds().unwrap().1[1] - 1.0).abs() < 1e-9);
        assert!(!b.push_face(top, -1.0), "the top cannot pass the floor");
        assert!((b.bounds().unwrap().1[1] - 1.0).abs() < 1e-9, "a refused push changes nothing");
        assert!(!b.push_face(FaceRef { carve: true, solid: 0, face: 0 }, 1.0), "no such carve");
    }

    #[test]
    fn an_impossible_map_changes_nothing() {
        let mut b = cube();
        let before = format!("{:?}", b);
        b.transform([[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]], [0.0; 3], [0.0; 3], true);
        assert_eq!(before, format!("{:?}", b));
    }
}
