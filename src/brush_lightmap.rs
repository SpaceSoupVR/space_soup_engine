use crate::brush::BrushDef;
use crate::brush::Vec3;

pub const GUTTER: u32 = 4;

pub const MAX_ATLAS: u32 = 1024;

const MIN_CHART: u32 = 2;

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn mul(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

struct Measured {
    object: usize,
    solid: usize,
    face: usize,
    part: usize,
    w: f64,
    h: f64,
    min_u: f64,
    min_v: f64,
    u: Vec3,
    v: Vec3,
    normal: Vec3,

    plane_d: f64,
    texel: f64,

    group: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrushChart {

    pub object: usize,
    pub solid: usize,
    pub face: usize,
    /// Which connected piece of the face's plane this chart holds
    /// (`fragment_parts`): pieces that do not touch never share texels.
    #[serde(default)]
    pub part: usize,

    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,

    pub origin: Vec3,

    pub du: Vec3,
    pub dv: Vec3,
    pub normal: Vec3,
}

impl BrushChart {

    #[must_use]
    pub fn texel_world(&self, tx: u32, ty: u32) -> Vec3 {
        add(
            self.origin,
            add(mul(self.du, f64::from(tx)), mul(self.dv, f64::from(ty))),
        )
    }

    #[must_use]
    pub fn uv2(&self, p: Vec3, atlas_w: u32, atlas_h: u32) -> [f32; 2] {
        let rel = sub(p, self.origin);
        let lu = dot(self.du, self.du);
        let lv = dot(self.dv, self.dv);
        let tx = if lu > 0.0 { dot(rel, self.du) / lu } else { 0.0 };
        let ty = if lv > 0.0 { dot(rel, self.dv) / lv } else { 0.0 };

        [
            ((f64::from(self.x) + tx + 0.5) / f64::from(atlas_w)) as f32,
            ((f64::from(self.y) + ty + 0.5) / f64::from(atlas_h)) as f32,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrushLightmapLayout {
    pub width: u32,
    pub height: u32,
    pub charts: Vec<BrushChart>,

    pub density_scale: f64,
}

pub const SUN_MASK_SCALE: u32 = 4;

pub const SUN_MASK_DISTANCE_TEXELS: f32 = 4.0;

pub const STATIONARY_MASK_SCALE: u32 = 2;

pub const STATIONARY_MASK_DISTANCE_TEXELS: f32 = 4.0;

impl BrushLightmapLayout {

    #[must_use]
    pub fn scaled(&self, s: u32) -> Self {
        let f = f64::from(s);
        let back = 0.5 / f - 0.5;
        Self {
            width: self.width * s,
            height: self.height * s,
            density_scale: self.density_scale * f,
            charts: self
                .charts
                .iter()
                .map(|c| BrushChart {
                    x: c.x * s,
                    y: c.y * s,
                    w: c.w * s,
                    h: c.h * s,
                    origin: add(c.origin, mul(add(c.du, c.dv), back)),
                    du: mul(c.du, 1.0 / f),
                    dv: mul(c.dv, 1.0 / f),
                    ..c.clone()
                })
                .collect(),
        }
    }

    /// The face's first chart (its only one unless pieces of it lie apart).
    #[must_use]
    pub fn chart(&self, object: usize, solid: usize, face: usize) -> Option<&BrushChart> {
        self.charts
            .iter()
            .find(|c| c.object == object && c.solid == solid && c.face == face)
    }

    /// The chart of one piece of a face (`fragment_parts`).
    pub fn chart_part(&self, object: usize, solid: usize, face: usize, part: usize) -> Option<&BrushChart> {
        self.charts
            .iter()
            .find(|c| c.object == object && c.solid == solid && c.face == face && c.part == part)
    }
}

/// Faces of one brush on one plane, as the layout welds them: same normal,
/// offset, texture axes and texel size.
fn plane_key(face: &crate::brush::BrushFace) -> ([i64; 3], i64, [i64; 3], [i64; 3], i64) {
    const Q: f64 = 1e4;
    let q = |x: f64| (x * Q).round() as i64;
    let qv = |v: Vec3| [q(v[0]), q(v[1]), q(v[2])];
    let (u, v) = face.axes();
    let pl = face.plane();
    (qv(pl.n), q(pl.d), qv(u), qv(v), q(if face.lightmap_scale > 1e-6 { face.lightmap_scale } else { 0.25 }))
}

fn point_segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-18)).clamp(0.0, 1.0);
    let d = sub(p, add(a, mul(ab, t)));
    dot(d, d).sqrt()
}

/// Two fragments of one plane meet: a corner of one lies on an edge of the
/// other (the cuts that made them leave T-junctions, not shared corners).
fn fragments_touch(a: &[Vec3], b: &[Vec3]) -> bool {
    const TOUCH: f64 = 1e-5;
    let bounds = |p: &[Vec3]| {
        p.iter().fold(([f64::MAX; 3], [f64::MIN; 3]), |(lo, hi), q| {
            ([lo[0].min(q[0]), lo[1].min(q[1]), lo[2].min(q[2])], [hi[0].max(q[0]), hi[1].max(q[1]), hi[2].max(q[2])])
        })
    };
    let ((alo, ahi), (blo, bhi)) = (bounds(a), bounds(b));
    if (0..3).any(|k| alo[k] > bhi[k] + TOUCH || blo[k] > ahi[k] + TOUCH) {
        return false;
    }
    let on_edge = |p: Vec3, poly: &[Vec3]| (0..poly.len()).any(|i| point_segment_distance(p, poly[i], poly[(i + 1) % poly.len()]) < TOUCH);
    a.iter().any(|&p| on_edge(p, b)) || b.iter().any(|&p| on_edge(p, a))
}

/// WHICH TEXELS OF EACH CHART ITS OWN SURFACE COVERS (`layout.charts`
/// order, `w * h` row by row, at whatever scale `layout` is -- a
/// `scaled` copy gives the masks' texels). A chart is a rectangle, and the
/// faces welded into it need not fill it: a hall's outer wall with a
/// hallway built against it is a frame round a hole. The hole's texels
/// stand in open air -- the hallway's lamp-lit inside -- so a bake lit them,
/// and the outer wall's edge texels filtered that light into a white line
/// down the joint at night (headset, 2026-10-21). A texel is covered when a
/// fragment of its chart (any face welded into the same rectangle) holds its
/// centre or passes within 0.75 of a texel of it; the bake treats the rest
/// as invalid and fills them from the covered ones.
pub fn chart_coverage(brushes: &[&BrushDef], layout: &BrushLightmapLayout) -> Vec<Vec<bool>> {
    let scene = crate::brush::scene_exposed_fragments(brushes);
    let parts: Vec<Vec<Vec<Vec<usize>>>> = scene.iter().map(|(s, e)| fragment_parts(s, e)).collect();
    // Every fragment of every face sharing a rectangle (a welded group).
    let mut by_rect: std::collections::HashMap<(usize, u32, u32), Vec<&[Vec3]>> = std::collections::HashMap::new();
    for c in &layout.charts {
        let Some((_, exposed)) = scene.get(c.object) else { continue };
        let Some(frags) = exposed.get(c.solid).and_then(|f| f.get(c.face)) else { continue };
        let entry = by_rect.entry((c.object, c.x, c.y)).or_default();
        for (k, f) in frags.iter().enumerate() {
            if parts[c.object][c.solid][c.face].get(k) == Some(&c.part) {
                entry.push(f.as_slice());
            }
        }
    }
    const REACH: f64 = 0.75;
    layout
        .charts
        .iter()
        .map(|c| {
            let (lu, lv) = (dot(c.du, c.du), dot(c.dv, c.dv));
            // In texel units from texel (0, 0)'s centre.
            let to_texels = |p: Vec3| {
                let rel = sub(p, c.origin);
                [if lu > 0.0 { dot(rel, c.du) / lu } else { 0.0 }, if lv > 0.0 { dot(rel, c.dv) / lv } else { 0.0 }]
            };
            let polys: Vec<Vec<[f64; 2]>> = by_rect
                .get(&(c.object, c.x, c.y))
                .map(|fs| fs.iter().map(|f| f.iter().map(|p| to_texels(*p)).collect()).collect())
                .unwrap_or_default();
            if polys.is_empty() {
                return vec![true; (c.w * c.h) as usize];
            }
            let mut out = Vec::with_capacity((c.w * c.h) as usize);
            for ty in 0..c.h {
                for tx in 0..c.w {
                    let q = [tx as f64, ty as f64];
                    out.push(polys.iter().any(|poly| inside_2d(poly, q) || distance_2d(poly, q) <= REACH));
                }
            }
            out
        })
        .collect()
}

fn inside_2d(poly: &[[f64; 2]], q: [f64; 2]) -> bool {
    let mut inside = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn distance_2d(poly: &[[f64; 2]], q: [f64; 2]) -> f64 {
    (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ab, aq) = ([b[0] - a[0], b[1] - a[1]], [q[0] - a[0], q[1] - a[1]]);
            let len = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if len > 0.0 { ((aq[0] * ab[0] + aq[1] * ab[1]) / len).clamp(0.0, 1.0) } else { 0.0 };
            let (dx, dy) = (aq[0] - ab[0] * t, aq[1] - ab[1] * t);
            (dx * dx + dy * dy).sqrt()
        })
        .fold(f64::MAX, f64::min)
}

/// EACH EXPOSED FRAGMENT'S PART: the fragments of a brush on one plane that
/// touch, through any chain of others, are one part; ones that do not are
/// separate parts, laid out as separate charts with gutters between. A hall's
/// outer wall and the end of a hallway built against it lie on one plane 0.3
/// m apart; in one chart the lamp-lit end and the moonlit outside shared
/// texels, and the filtering drew the lamp's light down the outside joint
/// (headset, 2026-10-21). `[solid][face][fragment]`, numbered per plane.
pub fn fragment_parts(solids: &[crate::brush::BrushSolid], exposed: &[Vec<Vec<Vec<Vec3>>>]) -> Vec<Vec<Vec<usize>>> {
    let mut items: Vec<(([i64; 3], i64, [i64; 3], [i64; 3], i64), &[Vec3])> = Vec::new();
    for (si, faces) in exposed.iter().enumerate() {
        for (fi, frags) in faces.iter().enumerate() {
            let key = plane_key(&solids[si].faces[fi]);
            for f in frags {
                items.push((key, f.as_slice()));
            }
        }
    }
    let mut root: Vec<usize> = (0..items.len()).collect();
    fn find(root: &mut [usize], mut i: usize) -> usize {
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    for i in 0..items.len() {
        for j in i + 1..items.len() {
            if items[i].0 == items[j].0 && fragments_touch(items[i].1, items[j].1) {
                let (a, b) = (find(&mut root, i), find(&mut root, j));
                if a != b {
                    root[b.max(a)] = a.min(b);
                }
            }
        }
    }
    // Number each plane's parts 0, 1, ... in the order they are first met.
    let mut numbers: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut per_key: std::collections::HashMap<([i64; 3], i64, [i64; 3], [i64; 3], i64), usize> = std::collections::HashMap::new();
    let mut part_of = Vec::with_capacity(items.len());
    for i in 0..items.len() {
        let r = find(&mut root, i);
        let n = *numbers.entry(r).or_insert_with(|| {
            let next = per_key.entry(items[i].0).or_insert(0);
            *next += 1;
            *next - 1
        });
        part_of.push(n);
    }
    let mut it = part_of.into_iter();
    exposed.iter().map(|faces| faces.iter().map(|frags| frags.iter().map(|_| it.next().unwrap_or(0)).collect()).collect()).collect()
}

#[must_use]
pub fn brush_lightmap_layout(brush: &BrushDef) -> BrushLightmapLayout {
    scene_brush_lightmap_layout(&[brush])
}

#[must_use]
pub fn scene_brush_lightmap_layout(brushes: &[&BrushDef]) -> BrushLightmapLayout {

    let mut measured = Vec::new();
    // Charts of the surface the level shows: each brush trimmed by the
    // others as well as itself (`brush::scene_exposed_fragments`).
    for (oi, (solids, exposed)) in crate::brush::scene_exposed_fragments(brushes).iter().enumerate() {
        // A face's fragments that do not touch get charts of their own, so
        // one plane split by another brush does not share texels across it.
        let parts = fragment_parts(solids, exposed);
        for (si, solid) in solids.iter().enumerate() {
            for (fi, all) in exposed[si].iter().enumerate() {
                let mut numbers: Vec<usize> = parts[si][fi].clone();
                numbers.sort_unstable();
                numbers.dedup();
                for part in numbers {
                    let poly: Vec<crate::brush::Vec3> = all
                        .iter()
                        .zip(&parts[si][fi])
                        .filter(|(_, p)| **p == part)
                        .flat_map(|(f, _)| f.iter().copied())
                        .collect();
                    if poly.len() < 3 {
                        continue;
                    }
                    let face = &solid.faces[fi];
                    let (u, v) = face.axes();
                    let (mut min_u, mut max_u) = (f64::MAX, f64::MIN);
                    let (mut min_v, mut max_v) = (f64::MAX, f64::MIN);
                    for p in &poly {
                        let (pu, pv) = (dot(*p, u), dot(*p, v));
                        min_u = min_u.min(pu);
                        max_u = max_u.max(pu);
                        min_v = min_v.min(pv);
                        max_v = max_v.max(pv);
                    }
                    let texel = if face.lightmap_scale > 1e-6 { face.lightmap_scale } else { 0.25 };
                    measured.push(Measured {
                        object: oi,
                        solid: si,
                        face: fi,
                        part,
                        w: (max_u - min_u).max(0.0),
                        h: (max_v - min_v).max(0.0),
                        min_u,
                        min_v,
                        u,
                        v,
                        normal: face.plane().n,
                        plane_d: face.plane().d,
                        texel,

                        group: 0,
                    });
                }
            }
        }
    }
    weld_coplanar(&mut measured);
    if measured.is_empty() {
        return BrushLightmapLayout { width: 1, height: 1, charts: Vec::new(), density_scale: 1.0 };
    }

    let mut density_scale = 1.0_f64;
    for _ in 0..12 {
        if let Some(layout) = try_pack(&measured, density_scale) {
            return layout;
        }
        density_scale *= 0.5;
    }

    try_pack(&measured, 0.0).unwrap_or(BrushLightmapLayout {
        width: 1,
        height: 1,
        charts: Vec::new(),
        density_scale: 0.0,
    })
}

fn texels_across(metres: f64, per_metre: f64) -> u32 {
    let exact = metres * per_metre;

    let eps = exact.abs() * 1e-9 + 1e-9;
    (exact - eps).ceil().max(1.0) as u32
}

fn span_per_texel(metres: f64, count: u32) -> f64 {
    if count == 0 || metres <= 0.0 {
        return metres.max(1.0);
    }
    metres / f64::from(count)
}

fn chart_at(m: &Measured, x: u32, y: u32, w: u32, h: u32) -> BrushChart {
    let du = mul(m.u, span_per_texel(m.w, w));
    let dv = mul(m.v, span_per_texel(m.h, h));
    let origin = add(
        add(mul(m.u, m.min_u), mul(m.v, m.min_v)),
        add(mul(du, 0.5), mul(dv, 0.5)),
    );
    let along = dot(origin, m.normal);
    let origin = add(origin, mul(m.normal, m.plane_d - along));
    BrushChart {
        object: m.object,
        solid: m.solid,
        face: m.face,
        part: m.part,
        x,
        y,
        w,
        h,
        origin,
        du,
        dv,
        normal: m.normal,
    }
}

fn weld_coplanar(measured: &mut [Measured]) {

    const Q: f64 = 1e4;
    let q = |x: f64| (x * Q).round() as i64;
    let qv = |v: Vec3| [q(v[0]), q(v[1]), q(v[2])];

    let mut keys: Vec<([i64; 3], i64, [i64; 3], [i64; 3], i64, usize)> = Vec::new();
    for (i, m) in measured.iter().enumerate() {
        keys.push((qv(m.normal), q(m.plane_d), qv(m.u), qv(m.v), q(m.texel), i));
    }
    let mut groups: std::collections::HashMap<
        ([i64; 3], i64, [i64; 3], [i64; 3], i64, usize, usize),
        usize,
    > = std::collections::HashMap::new();
    for (k0, k1, k2, k3, k4, i) in keys {

        let key = (k0, k1, k2, k3, k4, measured[i].object, measured[i].part);
        let next = groups.len();
        let g = *groups.entry(key).or_insert(next);
        measured[i].group = g;
    }

    let mut extents: Vec<Option<(f64, f64, f64, f64)>> = vec![None; groups.len()];
    for m in measured.iter() {
        let e = &mut extents[m.group];
        let (lo_u, hi_u) = (m.min_u, m.min_u + m.w);
        let (lo_v, hi_v) = (m.min_v, m.min_v + m.h);
        *e = Some(match *e {
            None => (lo_u, hi_u, lo_v, hi_v),
            Some((a, b, c, d)) => (a.min(lo_u), b.max(hi_u), c.min(lo_v), d.max(hi_v)),
        });
    }
    for m in measured.iter_mut() {
        if let Some((lo_u, hi_u, lo_v, hi_v)) = extents[m.group] {
            m.min_u = lo_u;
            m.min_v = lo_v;
            m.w = (hi_u - lo_u).max(0.0);
            m.h = (hi_v - lo_v).max(0.0);
        }
    }
}

fn try_pack(measured: &[Measured], density_scale: f64) -> Option<BrushLightmapLayout> {
    let sized: Vec<(u32, u32)> = measured
        .iter()
        .map(|m| {
            let per_metre = if m.texel > 0.0 { density_scale / m.texel } else { 0.0 };
            (
                (texels_across(m.w, per_metre)).clamp(MIN_CHART, MAX_ATLAS),
                (texels_across(m.h, per_metre)).clamp(MIN_CHART, MAX_ATLAS),
            )
        })
        .collect();

    let area: u64 = sized
        .iter()
        .map(|(w, h)| u64::from(w + 2 * GUTTER) * u64::from(h + 2 * GUTTER))
        .sum();
    let mut width = (area as f64).sqrt().ceil().max(4.0) as u32;
    width = width.next_power_of_two().min(MAX_ATLAS);

    let mut charts = Vec::with_capacity(measured.len());
    let (mut shelf_y, mut shelf_h, mut cursor_x) = (0u32, 0u32, 0u32);

    let group_count = measured.iter().map(|m| m.group + 1).max().unwrap_or(0);
    let mut placed: Vec<Option<(u32, u32, u32, u32)>> = vec![None; group_count];
    for (m, (w, h)) in measured.iter().zip(sized.iter()) {
        if let Some((px, py, pw2, ph2)) = placed[m.group] {

            let per_metre = if m.texel > 0.0 { density_scale / m.texel } else { 0.0 };
            let _ = per_metre;
            charts.push(chart_at(m, px, py, pw2, ph2));
            continue;
        }
        let (pw, ph) = (w + 2 * GUTTER, h + 2 * GUTTER);
        if pw > width {
            return None;
        }
        if cursor_x + pw > width {
            shelf_y += shelf_h;
            shelf_h = 0;
            cursor_x = 0;
        }
        if shelf_y + ph > MAX_ATLAS {
            return None;
        }

        let du = mul(m.u, span_per_texel(m.w, *w));
        let dv = mul(m.v, span_per_texel(m.h, *h));

        let origin = add(
            add(mul(m.u, m.min_u), mul(m.v, m.min_v)),
            add(mul(du, 0.5), mul(dv, 0.5)),
        );

        let along = dot(origin, m.normal);
        let origin = add(origin, mul(m.normal, m.plane_d - along));

        let (cx, cy) = (cursor_x + GUTTER, shelf_y + GUTTER);
        placed[m.group] = Some((cx, cy, *w, *h));
        charts.push(BrushChart {
            object: m.object,
            solid: m.solid,
            face: m.face,
            part: m.part,
            x: cx,
            y: cy,
            w: *w,
            h: *h,
            origin,
            du,
            dv,
            normal: m.normal,
        });
        cursor_x += pw;
        shelf_h = shelf_h.max(ph);
    }

    let height = (shelf_y + shelf_h).max(1).next_power_of_two().min(MAX_ATLAS);
    if shelf_y + shelf_h > height {
        return None;
    }
    Some(BrushLightmapLayout { width, height, charts, density_scale })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{block_solid, BrushDef};

    fn room() -> BrushDef {
        BrushDef {
            solids: vec![block_solid([-4.0, 0.0, -4.0], [4.0, 3.0, 4.0], "default")],
            subtract: Vec::new(),
        }
    }

    fn brush_at(min: Vec3, max: Vec3) -> BrushDef {
        BrushDef { solids: vec![block_solid(min, max, "default")], subtract: Vec::new() }
    }

    fn dist_to_plane(n: Vec3, d: f64, p: Vec3) -> f64 {
        dot(n, p) - d
    }

    #[test]
    fn every_face_gets_a_chart() {
        let layout = brush_lightmap_layout(&room());
        assert_eq!(layout.charts.len(), 6);
        for f in 0..6 {
            assert!(layout.chart(0, 0, f).is_some(), "face {f} has nowhere to put its light");
        }
    }

    #[test]
    fn every_chart_fits_inside_the_atlas() {
        let layout = brush_lightmap_layout(&room());
        for c in &layout.charts {
            assert!(c.x + c.w <= layout.width, "{c:?} runs off the right of {}", layout.width);
            assert!(c.y + c.h <= layout.height, "{c:?} runs off the bottom of {}", layout.height);
        }
    }

    #[test]
    fn charts_never_overlap_even_counting_their_gutters() {

        const MIN_SEPARATION: u32 = 1;
        let layout = brush_lightmap_layout(&room());
        for (i, a) in layout.charts.iter().enumerate() {
            for b in layout.charts.iter().skip(i + 1) {
                let sep_x = a.x + a.w + MIN_SEPARATION <= b.x || b.x + b.w + MIN_SEPARATION <= a.x;
                let sep_y = a.y + a.h + MIN_SEPARATION <= b.y || b.y + b.h + MIN_SEPARATION <= a.y;
                assert!(sep_x || sep_y, "charts overlap or touch:\n{a:?}\n{b:?}");
            }
        }
    }

    #[test]
    fn a_texel_lands_on_the_face_it_belongs_to() {

        let brush = brush_at([2.0, 1.0, -5.0], [7.0, 4.0, -1.0]);
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        for c in &layout.charts {
            let face = &solids[c.solid].faces[c.face];
            let (n, d) = (face.plane().n, face.plane().d);
            for (tx, ty) in [(0, 0), (c.w / 2, c.h / 2), (c.w - 1, c.h - 1)] {
                let p = c.texel_world(tx, ty);
                assert!(
                    dist_to_plane(n, d, p).abs() < 1e-6,
                    "texel ({tx},{ty}) of {c:?} is {} off its plane",
                    dist_to_plane(n, d, p),
                );
            }
        }
    }

    #[test]
    fn a_face_that_divides_exactly_gets_no_wasted_row() {
        assert_eq!(texels_across(3.0, 4.0), 12, "3 m at 4 texels/m is 12, not 13");
        assert_eq!(texels_across(8.0, 4.0), 32);

        assert_eq!(texels_across(3.1, 4.0), 13);
        assert_eq!(texels_across(0.05, 4.0), 1, "a sliver still gets one texel");
    }

    #[test]
    fn every_texel_centre_lies_within_its_own_face_not_merely_on_its_plane() {

        let brush = brush_at([-4.0, 0.0, -4.0], [4.0, 3.0, -3.75]);
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        for c in &layout.charts {
            let polys = crate::brush::solid_polygons(&solids[c.solid]);
            let poly = polys[c.face].as_ref().expect("charted face must have a polygon");

            let (mut min_u, mut max_u) = (f64::MAX, f64::MIN);
            let (mut min_v, mut max_v) = (f64::MAX, f64::MIN);
            let (u, v) = (c.du, c.dv);
            let (lu, lv) = (dot(u, u).sqrt(), dot(v, v).sqrt());
            let (un, vn) = (mul(u, 1.0 / lu), mul(v, 1.0 / lv));
            for p in poly {
                let (pu, pv) = (dot(*p, un), dot(*p, vn));
                min_u = min_u.min(pu);
                max_u = max_u.max(pu);
                min_v = min_v.min(pv);
                max_v = max_v.max(pv);
            }
            for (tx, ty) in [(0, 0), (c.w - 1, 0), (0, c.h - 1), (c.w - 1, c.h - 1)] {
                let p = c.texel_world(tx, ty);
                let (pu, pv) = (dot(p, un), dot(p, vn));
                assert!(
                    pu >= min_u - 1e-6 && pu <= max_u + 1e-6,
                    "texel ({tx},{ty}) of {c:?} sits at u={pu} outside the face's \
                     own {min_u}..{max_u}",
                );
                assert!(
                    pv >= min_v - 1e-6 && pv <= max_v + 1e-6,
                    "texel ({tx},{ty}) of {c:?} sits at v={pv} outside the face's \
                     own {min_v}..{max_v}",
                );
            }
        }
    }

    #[test]
    fn uv2_and_texel_world_are_inverses() {

        let layout = brush_lightmap_layout(&room());
        for c in &layout.charts {
            for (tx, ty) in [(0, 0), (1, 1), (c.w - 1, c.h - 1)] {
                let world = c.texel_world(tx, ty);
                let uv = c.uv2(world, layout.width, layout.height);
                let back_x = f64::from(uv[0]) * f64::from(layout.width) - 0.5 - f64::from(c.x);
                let back_y = f64::from(uv[1]) * f64::from(layout.height) - 0.5 - f64::from(c.y);
                assert!((back_x - f64::from(tx)).abs() < 1e-3, "u round trip {back_x} vs {tx}");
                assert!((back_y - f64::from(ty)).abs() < 1e-3, "v round trip {back_y} vs {ty}");
            }
        }
    }

        #[test]
    fn uv2_of_a_faces_own_points_stays_inside_its_own_chart() {

        let brush = room();
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        for c in &layout.charts {
            let polys = crate::brush::solid_polygons(&solids[c.solid]);
            let poly = polys[c.face].as_ref().expect("charted face must have a polygon");
            for p in poly {
                let uv = c.uv2(*p, layout.width, layout.height);
                let tx = f64::from(uv[0]) * f64::from(layout.width);
                let ty = f64::from(uv[1]) * f64::from(layout.height);
                assert!(
                    tx >= f64::from(c.x) - 1e-6 && tx <= f64::from(c.x + c.w) + 1e-6,
                    "corner sampled at {tx}, chart spans {}..{}", c.x, c.x + c.w,
                );
                assert!(
                    ty >= f64::from(c.y) - 1e-6 && ty <= f64::from(c.y + c.h) + 1e-6,
                    "corner sampled at {ty}, chart spans {}..{}", c.y, c.y + c.h,
                );
            }
        }
    }

    #[test]
    fn a_bigger_wall_gets_more_texels() {

        let small = brush_lightmap_layout(&brush_at([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
        let large = brush_lightmap_layout(&brush_at([0.0, 0.0, 0.0], [16.0, 1.0, 16.0]));
        let biggest = |l: &BrushLightmapLayout| l.charts.iter().map(|c| c.w * c.h).max().unwrap();
        assert!(
            biggest(&large) > biggest(&small) * 4,
            "large {} vs small {}", biggest(&large), biggest(&small),
        );
    }

    #[test]
    fn an_enormous_brush_loses_density_rather_than_the_atlas_growing() {

        let huge = brush_lightmap_layout(&brush_at([0.0, 0.0, 0.0], [400.0, 60.0, 400.0]));
        assert!(huge.width <= MAX_ATLAS && huge.height <= MAX_ATLAS, "{}x{}", huge.width, huge.height);
        assert!(huge.density_scale < 1.0, "density should have been reduced to fit");
        assert_eq!(huge.charts.len(), 6, "reducing density must not drop faces");
    }

    #[test]
    fn an_ordinary_room_keeps_the_density_it_asked_for() {
        assert_eq!(brush_lightmap_layout(&room()).density_scale, 1.0);
    }

    #[test]
    fn the_layout_is_stable_across_calls() {

        assert_eq!(brush_lightmap_layout(&room()), brush_lightmap_layout(&room()));
    }

    #[test]
    fn the_hallway_end_and_the_halls_outer_wall_get_separate_charts() {
        // The brick hall's x = 10 wall faces -x both inside the hallway (its
        // end wall, lamp-lit) and outside it (moonlit). With the strip under
        // the hallway's walls trimmed away the two do not touch, and must not
        // share texels.
        let path = &crate::test_game_dir().join("scenes/test_room.json");
        let scene = crate::scene::Scene::load(std::path::Path::new(path)).expect("test_room loads");
        let defs: Vec<&BrushDef> = scene.objects.iter().filter_map(|o| o.brush.as_ref()).collect();
        let hall = scene.objects.iter().filter(|o| o.brush.is_some()).position(|o| o.id == "brick_hall").expect("brick_hall");
        let layout = scene_brush_lightmap_layout(&defs);
        let evaluated = crate::brush::scene_exposed_fragments(&defs);
        let (solids, exposed) = &evaluated[hall];
        let parts = fragment_parts(solids, exposed);
        let (mut inside, mut outside) = (Vec::new(), Vec::new());
        for (si, solid) in solids.iter().enumerate() {
            for (fi, face) in solid.faces.iter().enumerate() {
                let pl = face.plane();
                if pl.n[0] > -0.9 || (pl.d.abs() - 10.0).abs() > 1e-6 {
                    continue;
                }
                for (k, frag) in exposed[si][fi].iter().enumerate() {
                    // The hallway's room is y 0..2.6, z -4.2..-1.8.
                    let in_hallway = frag.iter().all(|p| (-1e-6..=2.6 + 1e-6).contains(&p[1]) && (-4.2 - 1e-6..=-1.8 + 1e-6).contains(&p[2]));
                    let chart = layout.chart_part(hall, si, fi, parts[si][fi][k]).expect("a chart");
                    let at = (chart.x, chart.y);
                    if in_hallway {
                        inside.push(at);
                    } else {
                        outside.push(at);
                    }
                }
            }
        }
        assert!(!inside.is_empty() && !outside.is_empty(), "inside {inside:?} outside {outside:?}");
        for c in &inside {
            assert!(!outside.contains(c), "the hallway end shares chart {c:?} with the outside wall");
        }
    }

    #[test]
    fn the_hole_a_hallway_leaves_in_the_halls_outer_wall_chart_is_uncovered() {
        let path = &crate::test_game_dir().join("scenes/test_room.json");
        let scene = crate::scene::Scene::load(std::path::Path::new(path)).expect("test_room loads");
        let defs: Vec<&BrushDef> = scene.objects.iter().filter_map(|o| o.brush.as_ref()).collect();
        let hall = scene.objects.iter().filter(|o| o.brush.is_some()).position(|o| o.id == "brick_hall").expect("brick_hall");
        let layout = scene_brush_lightmap_layout(&defs);
        let coverage = chart_coverage(&defs, &layout);
        assert_eq!(coverage.len(), layout.charts.len());
        // The outside wall's chart on the x = 10 plane: the one reaching past the hallway.
        let (mut hole, mut hole_covered, mut wall, mut lost) = (0, 0, 0, 0);
        for (c, cov) in layout.charts.iter().zip(&coverage) {
            assert_eq!(cov.len(), (c.w * c.h) as usize);
            let reaches_out = c.texel_world(0, 0)[2].min(c.texel_world(c.w - 1, c.h - 1)[2]) < -5.0;
            if c.object != hall || c.normal[0] > -0.9 || (c.origin[0] - 10.0).abs() > 1e-6 || !reaches_out {
                continue;
            }
            for ty in 0..c.h {
                for tx in 0..c.w {
                    let p = c.texel_world(tx, ty);
                    let covered = cov[(ty * c.w + tx) as usize];
                    // Well inside the hallway's footprint (y -0.3..2.9, z -4.5..-1.5).
                    if p[1] > 0.0 && p[1] < 2.6 && p[2] > -4.2 && p[2] < -1.8 {
                        hole += 1;
                        hole_covered += covered as usize;
                    }
                    // The open wall, well clear of the hallway.
                    if p[2] < -5.0 || p[2] > -1.0 || p[1] > 3.2 {
                        wall += 1;
                        lost += !covered as usize;
                    }
                }
            }
        }
        assert!(hole > 0, "the outer wall's chart holds no texels in the hallway's hole");
        assert_eq!(hole_covered, 0, "{hole_covered} of {hole} texels in the hallway's hole count as the outer wall's surface");
        assert!(wall > 0 && lost == 0, "{lost} of {wall} open-wall texels lost their coverage");
    }

    #[test]
    fn two_brushes_share_one_atlas_without_colliding() {

        let a = brush_at([0.0, 0.0, 0.0], [4.0, 3.0, 0.5]);
        let b = brush_at([10.0, 0.0, 0.0], [12.0, 2.0, 0.5]);
        let layout = scene_brush_lightmap_layout(&[&a, &b]);

        assert!(layout.charts.iter().any(|c| c.object == 0));
        assert!(layout.charts.iter().any(|c| c.object == 1));
        for (i, x) in layout.charts.iter().enumerate() {
            for y in layout.charts.iter().skip(i + 1) {
                let apart_x = x.x + x.w < y.x || y.x + y.w < x.x;
                let apart_y = x.y + x.h < y.y || y.y + y.h < x.y;
                assert!(apart_x || apart_y, "charts of different brushes overlap:\n{x:?}\n{y:?}");
            }
        }
    }

    #[test]
    fn one_brush_packs_the_same_whether_asked_alone_or_as_a_scene() {

        let a = room();
        assert_eq!(brush_lightmap_layout(&a), scene_brush_lightmap_layout(&[&a]));
    }

    #[test]
    fn an_empty_brush_produces_a_usable_atlas_rather_than_a_zero_one() {

        let empty = brush_lightmap_layout(&BrushDef { solids: Vec::new(), subtract: Vec::new() });
        assert!(empty.width >= 1 && empty.height >= 1);
        assert!(empty.charts.is_empty());
    }
}

#[cfg(test)]
mod welding_tests {
    use super::*;
    use crate::brush::{BrushDef, BrushFace, BrushSolid};

    fn face(n: [f64; 3], d: f64) -> BrushFace {
        serde_json::from_str(&format!(
            r#"{{"plane":[{},{},{},{}],"material":"m","scale":[1,1]}}"#,
            n[0], n[1], n[2], d
        ))
        .unwrap()
    }

    fn block(lo: [f64; 3], hi: [f64; 3]) -> BrushSolid {
        BrushSolid {
            faces: vec![
                face([1.0, 0.0, 0.0], hi[0]),
                face([-1.0, 0.0, 0.0], -lo[0]),
                face([0.0, 1.0, 0.0], hi[1]),
                face([0.0, -1.0, 0.0], -lo[1]),
                face([0.0, 0.0, 1.0], hi[2]),
                face([0.0, 0.0, -1.0], -lo[2]),
            ],
        }
    }

    fn wall_with_a_hole_through_it() -> BrushDef {
        BrushDef {
            solids: vec![block([0.0, 0.0, 0.0], [8.0, 3.0, 0.25])],

            subtract: vec![block([3.0, 0.0, -1.0], [5.0, 2.0, 1.0])],
        }
    }

    #[test]
    fn a_subtraction_really_does_split_the_wall() {

        let brush = wall_with_a_hole_through_it();
        assert!(
            brush.evaluate().len() > 1,
            "this subtraction no longer splits the solid, so it cannot test welding",
        );
    }

    #[test]
    fn the_pieces_of_one_wall_share_one_chart() {

        let brush = wall_with_a_hole_through_it();
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();

        let front: Vec<&BrushChart> = layout
            .charts
            .iter()
            .filter(|c| c.normal[2] > 0.9 && solids[c.solid].faces[c.face].plane().d > 0.2)
            .collect();
        assert!(front.len() > 1, "expected the front face to arrive in pieces");
        let first = (front[0].x, front[0].y, front[0].w, front[0].h);
        for c in &front {
            assert_eq!(
                (c.x, c.y, c.w, c.h),
                first,
                "two pieces of one wall were given different atlas rectangles",
            );
        }
    }

    #[test]
    fn a_welded_chart_spans_the_whole_wall() {

        let brush = wall_with_a_hole_through_it();
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        let front = layout
            .charts
            .iter()
            .find(|c| c.normal[2] > 0.9 && solids[c.solid].faces[c.face].plane().d > 0.2)
            .expect("a front face");

        let len = |v: Vec3| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let (du, dv) = (len(front.du), len(front.dv));
        let spanned_w = f64::from(front.w) * du;
        let spanned_h = f64::from(front.h) * dv;

        assert!(
            (spanned_w - 8.0).abs() <= du,
            "the chart does not span the wall's full 8 m width: {spanned_w:.3} m \
             ({} texels of {du:.3} m)",
            front.w,
        );
        assert!(
            (spanned_h - 3.0).abs() <= dv,
            "the chart does not span the wall's full 3 m height: {spanned_h:.3} m \
             ({} texels of {dv:.3} m)",
            front.h,
        );
    }

    #[test]
    fn faces_on_different_planes_are_not_welded() {

        let brush = wall_with_a_hole_through_it();
        let layout = brush_lightmap_layout(&brush);
        let front = layout.charts.iter().find(|c| c.normal[2] > 0.9).unwrap();
        let back = layout.charts.iter().find(|c| c.normal[2] < -0.9).unwrap();
        assert_ne!(
            (front.x, front.y),
            (back.x, back.y),
            "the front and back of a wall were welded into one chart",
        );
    }

    #[test]
    fn welding_frees_atlas_space_rather_than_spending_more() {

        let brush = wall_with_a_hole_through_it();
        let layout = brush_lightmap_layout(&brush);
        let distinct: std::collections::HashSet<(u32, u32)> =
            layout.charts.iter().map(|c| (c.x, c.y)).collect();
        assert!(
            distinct.len() < layout.charts.len(),
            "no charts were shared, so nothing was welded",
        );
    }
}

#[must_use]
pub fn point_on_face(poly: &[Vec3], normal: Vec3, p: Vec3, tolerance: f64) -> bool {
    if poly.len() < 3 {
        return false;
    }

    let mut most_positive: f64 = f64::NEG_INFINITY;
    let mut most_negative: f64 = f64::INFINITY;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let edge = sub(b, a);
        let out = cross(edge, normal);
        let len = dot(out, out).sqrt();
        if len < 1e-12 {
            continue;
        }
        let d = dot(sub(p, a), out) / len;
        most_positive = most_positive.max(d);
        most_negative = most_negative.min(d);
    }
    if most_positive == f64::NEG_INFINITY {
        return false;
    }

    most_positive <= tolerance || most_negative >= -tolerance
}

#[must_use]
pub fn clamp_to_face(poly: &[Vec3], p: Vec3) -> Vec3 {
    if poly.len() < 3 {
        return p;
    }
    let mut best = poly[0];
    let mut best_d = f64::INFINITY;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let ab = sub(b, a);
        let len2 = dot(ab, ab);
        let t = if len2 > 1e-12 {
            (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = add(a, mul(ab, t));
        let d = dot(sub(p, q), sub(p, q));
        if d < best_d {
            best_d = d;
            best = q;
        }
    }
    best
}

#[cfg(test)]
mod face_containment_tests {
    use super::*;

    fn wall() -> (Vec<Vec3>, Vec3) {
        (
            vec![
                [3.0, -0.3, 4.0],
                [3.0, -0.3, -16.0],
                [3.0, 3.4, -16.0],
                [3.0, 3.4, 4.0],
            ],
            [1.0, 0.0, 0.0],
        )
    }

    #[test]
    fn a_point_in_the_middle_of_a_face_is_on_it() {
        let (poly, n) = wall();
        assert!(point_on_face(&poly, n, [3.0, 1.55, -6.125], 0.02));
    }

    #[test]
    fn reversing_the_winding_changes_nothing() {
        let (mut poly, n) = wall();
        let inside = [3.0, 1.55, -6.125];
        let outside = [3.0, 9.0, -6.125];
        assert!(point_on_face(&poly, n, inside, 0.02));
        assert!(!point_on_face(&poly, n, outside, 0.02));
        poly.reverse();
        assert!(point_on_face(&poly, n, inside, 0.02), "reversed winding rejected an inside point");
        assert!(!point_on_face(&poly, n, outside, 0.02), "reversed winding accepted an outside one");
    }

    #[test]
    fn a_texel_just_past_an_edge_is_kept() {
        let (poly, n) = wall();
        assert!(point_on_face(&poly, n, [3.0, 3.41, -6.0], 0.02), "a texel 1cm past the top was dropped");
        assert!(!point_on_face(&poly, n, [3.0, 3.8, -6.0], 0.02), "a texel 40cm past the top was kept");
    }

    #[test]
    fn clamping_lands_on_the_nearest_edge() {
        let (poly, _) = wall();
        let q = clamp_to_face(&poly, [3.0, 9.0, -6.0]);
        assert!((q[1] - 3.4).abs() < 1e-6, "clamped to y={} rather than the top edge", q[1]);
        assert!((q[2] + 6.0).abs() < 1e-6, "clamping moved the point along the edge");

        let inside = [3.0, 1.55, -6.125];
        let r = clamp_to_face(&poly, inside);
        assert!(
            (r[1] - inside[1]).abs() < 2.0,
            "clamping moved an interior point a long way: {r:?}",
        );
    }

    #[test]
    fn too_few_vertices_is_not_a_face() {
        assert!(!point_on_face(&[[0.0; 3], [1.0, 0.0, 0.0]], [0.0, 0.0, 1.0], [0.0; 3], 0.1));
        assert_eq!(clamp_to_face(&[], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    }
}

#[cfg(test)]
mod scaled_layout_tests {
    use super::*;

    #[test]
    fn a_scaled_layout_keeps_every_points_uv2() {
        let solid = crate::brush::block_solid([0.0, 0.0, 0.0], [4.0, 3.0, 0.3], "m");
        let brush = crate::brush::BrushDef { solids: vec![solid], subtract: Vec::new() };
        let base = scene_brush_lightmap_layout(&[&brush]);
        let fine = base.scaled(SUN_MASK_SCALE);
        for (a, b) in base.charts.iter().zip(&fine.charts) {
            for (fu, fv) in [(0.0, 0.0), (0.37, 0.81), (0.999, 0.5)] {
                let p = add(a.texel_world(0, 0), add(mul(a.du, fu * f64::from(a.w)), mul(a.dv, fv * f64::from(a.h))));
                let (ua, ub) = (a.uv2(p, base.width, base.height), b.uv2(p, fine.width, fine.height));
                assert!((ua[0] - ub[0]).abs() < 1e-6 && (ua[1] - ub[1]).abs() < 1e-6, "{ua:?} vs {ub:?}");
            }

            let c0 = a.texel_world(0, 0);
            let f0 = b.texel_world(0, 0);
            let want = add(c0, mul(add(a.du, a.dv), -0.375));
            assert!((0..3).all(|k| (f0[k] - want[k]).abs() < 1e-9), "{f0:?} vs {want:?}");
        }
    }
}
