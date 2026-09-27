//! Where a brush's baked lighting lives in its atlas.
//!
//! ONE LAYOUT, TWO CONSUMERS
//!
//! The baker has to know, for every texel, which point in the world it stands
//! for so it can shoot a ray from there. The renderer has to know, for every
//! vertex, which texel to sample. Those are inverse questions about the same
//! mapping, and if the two are computed by two pieces of code they will
//! eventually disagree -- and the symptom is not a crash or a blank texture, it
//! is a level where the shadows are slightly in the wrong place, which reads as
//! "the baker is buggy" rather than as a layout mismatch.
//!
//! So the layout is computed exactly once, here, and both sides are handed it.
//!
//! WHY NOT THE TEXTURE UVs
//!
//! A face already has `uv`, but it is in TILES and deliberately shared between
//! faces: two faces of the same wall get identical axes so their brickwork lines
//! up across the corner, and the coordinates repeat. Lighting needs the
//! opposite -- every face wants its own patch of the atlas, used once, or two
//! walls would sample each other's shadows.
//!
//! DENSITY AND THE GUTTER
//!
//! Texel size comes from the face's own `lightmap_scale` (metres per texel), so
//! a corridor that needs crisp shadows can be finer than the skybox-facing
//! outside of the same building. Every chart is padded by one texel on each
//! side: the sampler filters bilinearly and without a gutter a wall bleeds its
//! neighbour's lighting along every seam, which looks like light leaking through
//! the corner -- the exact artefact baked lighting is there to remove.

use crate::brush::BrushDef;
use crate::brush::Vec3;

/// Texels of empty space kept around every chart, to stop bilinear bleed.
///
/// FOUR, which is the documented minimum rather than the arithmetic one.
///
/// One is below the floor for plain bilinear on any hardware: the filter kernel
/// of a texel on a chart's edge reaches a full texel outward, so a one-texel
/// gutter is entirely consumed by the bleed it exists to stop. Two satisfies
/// bilinear and was the value here.
///
/// Four is what the engines say. Unreal and Unity both put the minimum at four
/// texels between charts; Unity's guidance goes further and asks for 8 px on a
/// 2K atlas. Their stated reason -- DXT operating on 4x4 blocks -- does not
/// apply to this atlas, which is uncompressed RGBA8. The reason that DOES apply
/// is that bilinear is not the only thing that reaches outside a chart: an MSAA
/// edge pixel is shaded at a centre that can lie outside its polygon, so the
/// interpolated uv2 is EXTRAPOLATED past the chart, by more than one texel and
/// by more the further away the surface is. Two texels bounds the filter; it
/// does not bound that.
///
/// The cost is atlas area, and it is real: four texels of padding on a 64x24
/// chart is about a third of its footprint. That is the trade the engines have
/// already made, and this atlas has room.
pub const GUTTER: u32 = 4;

/// The largest atlas a single brush may claim, per side.
///
/// A cap rather than a promise of quality: one enormous brush should cost a
/// blurry lightmap, not 64MB of texture. Density is scaled down to fit and the
/// layout reports it, so the reason is visible rather than mysterious.
pub const MAX_ATLAS: u32 = 1024;

/// Smallest chart, so a sliver face still gets somewhere to put its light.
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


/// One face, measured in its own plane, before anything is packed.
struct Measured {
    object: usize,
    solid: usize,
    face: usize,
    w: f64,
    h: f64,
    min_u: f64,
    min_v: f64,
    u: Vec3,
    v: Vec3,
    normal: Vec3,
    /// The face plane's own offset: points on it satisfy dot(n, p) == d.
    plane_d: f64,
    texel: f64,
    /// Which WELD GROUP this face belongs to -- see `weld_coplanar`.
    ///
    /// Faces in a group share one atlas rectangle, so the lighting across what
    /// used to be a boundary between them is continuous.
    group: usize,
}

/// One face's patch of the atlas, and how it maps to the world.
///
/// Serialisable because the EDITOR needs it. Its brush geometry is built in
/// JavaScript, and the only alternative to shipping this layout across is
/// porting the packer -- which would be a second implementation of the one
/// thing this module exists to keep single.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrushChart {
    /// Index into the brush list this layout was built from.
    ///
    /// Present because the atlas is SCENE-WIDE rather than per object: every
    /// brush in a level renders from one vertex buffer in one draw, so a
    /// lightmap per object would force a draw call per object -- and on a
    /// tile-based GPU draw calls are exactly the resource this feature is meant
    /// to respect.
    pub object: usize,
    pub solid: usize,
    pub face: usize,
    /// Texel rectangle, gutter excluded.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// World position at the CENTRE of this chart's texel (0, 0).
    ///
    /// Centre rather than corner because that is where a baked sample belongs:
    /// a ray fired from a texel's corner sits exactly on the boundary between
    /// this face and its neighbour, and lands on either depending on rounding.
    pub origin: Vec3,
    /// World step between horizontally and vertically adjacent texels.
    pub du: Vec3,
    pub dv: Vec3,
    pub normal: Vec3,
}

impl BrushChart {
    /// The world point a texel of this chart samples.
    pub fn texel_world(&self, tx: u32, ty: u32) -> Vec3 {
        add(
            self.origin,
            add(mul(self.du, tx as f64), mul(self.dv, ty as f64)),
        )
    }

    /// Where a world point on this face lands in the atlas, in 0..1.
    pub fn uv2(&self, p: Vec3, atlas_w: u32, atlas_h: u32) -> [f32; 2] {
        let rel = sub(p, self.origin);
        let lu = dot(self.du, self.du);
        let lv = dot(self.dv, self.dv);
        let tx = if lu > 0.0 { dot(rel, self.du) / lu } else { 0.0 };
        let ty = if lv > 0.0 { dot(rel, self.dv) / lv } else { 0.0 };
        // +0.5 puts a texel's centre at its centre. Without it every chart is
        // sampled half a texel off, which on a 2-texel sliver is a quarter of
        // the whole face.
        [
            ((self.x as f64 + tx + 0.5) / atlas_w as f64) as f32,
            ((self.y as f64 + ty + 0.5) / atlas_h as f64) as f32,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrushLightmapLayout {
    pub width: u32,
    pub height: u32,
    pub charts: Vec<BrushChart>,
    /// Multiplier applied to every face's requested density to fit `MAX_ATLAS`.
    /// 1.0 when nothing had to be given up.
    pub density_scale: f64,
}

/// How much finer the sun-visibility mask is than the lightmap it rides with.
///
/// The sun is a disc half a degree across, so the soft edge of a shadow it
/// casts is only about 1 cm wide per metre from the edge that casts it -- 2 to
/// 4 cm for a doorway onto a floor. A lightmap texel is 12-25 cm, and a sun
/// baked into it came out as a staircase of whole texels (headset, 2026-09-23).
/// Four times finer puts a mask texel at 3-6 cm: the width of the real edge.
///
/// Only the MASK is finer. Bounce varies over metres and gains nothing from it.
pub const SUN_MASK_SCALE: u32 = 4;

/// How far the sun mask's signed distance to the shadow's edge reaches, in
/// mask texels, either side. Red 0..255 is -this..+this; blue is the sun's
/// penumbra half-width, 0..this. The renderer's
/// `brush_pipeline::SUN_MASK_DISTANCE_TEXELS` must equal it.
///
/// Four texels (12-24 cm) is far more than the sun's penumbra here -- about
/// 1 cm per metre from the edge that casts it -- and far enough that a
/// bilinear tap never reads a saturated value where an edge is close.
pub const SUN_MASK_DISTANCE_TEXELS: f32 = 4.0;

/// How much denser than the lightmap the STATIONARY lamps' shadow masks are,
/// on the same charts. Half the sun mask's: a lamp's mask is two bytes in an
/// RGBA layer shared by two lamps, and at the sun's density four layers would
/// cost 134 MB on test_room alone. The distance field keeps the edge straight
/// at any magnification; the density only sets how small a shadow's features
/// can be.
pub const STATIONARY_MASK_SCALE: u32 = 2;

/// How far a stationary mask's signed distance reaches, in mask texels, and
/// its penumbra: the same encoding as the sun mask's red and blue.
pub const STATIONARY_MASK_DISTANCE_TEXELS: f32 = 4.0;

impl BrushLightmapLayout {
    /// The same charts at `s` times the density, in an atlas `s` times the size.
    ///
    /// Every point keeps its uv2: texel (0, 0) of a scaled chart is the first
    /// of the `s x s` sub-texels of the original, so its centre moves back by
    /// half a texel less half a sub-texel. A mesh built against this layout
    /// samples both atlases with the same coordinates. The gutter scales with
    /// the charts, to `GUTTER * s` texels.
    pub fn scaled(&self, s: u32) -> Self {
        let f = s as f64;
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

    pub fn chart(&self, object: usize, solid: usize, face: usize) -> Option<&BrushChart> {
        self.charts
            .iter()
            .find(|c| c.object == object && c.solid == solid && c.face == face)
    }
}

/// Pack every face of a brush into one atlas.
///
/// Faces are visited in evaluation order and shelf-packed without sorting.
/// Sorting by height packs tighter, and would make the layout depend on the
/// relative sizes of unrelated faces -- so adding a small face somewhere could
/// move every other chart, invalidating a bake that nothing else had changed.
pub fn brush_lightmap_layout(brush: &BrushDef) -> BrushLightmapLayout {
    scene_brush_lightmap_layout(&[brush])
}

/// Pack every face of every brush in a level into a single atlas.
///
/// One atlas for the whole level rather than one per object, because the
/// renderer draws every brush from one vertex buffer. Per-object atlases would
/// mean per-object draw calls, which costs more on a Quest than the texture
/// memory it would save.
pub fn scene_brush_lightmap_layout(brushes: &[&BrushDef]) -> BrushLightmapLayout {
    // First pass: measure every face in its own plane, at its own density.
    let mut measured = Vec::new();
    for (oi, brush) in brushes.iter().enumerate() {
    let solids = brush.evaluate();
    // Measured from the EXPOSED fragments, not the whole face: a chart then
    // covers only surface someone can see, with no texels baked inside a wall,
    // and a face with nothing exposed gets no chart at all. The render mesh is
    // built from the same fragments (`brush_polygons_in_atlas`), so the two
    // cannot disagree about where a face ends.
    let exposed = crate::brush::exposed_fragments(&solids);
    for (si, solid) in solids.iter().enumerate() {
        for (fi, frags) in exposed[si].iter().enumerate() {
            let poly: Vec<crate::brush::Vec3> = frags.iter().flatten().copied().collect();
            if frags.is_empty() || poly.len() < 3 {
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
                w: (max_u - min_u).max(0.0),
                h: (max_v - min_v).max(0.0),
                min_u,
                min_v,
                u,
                v,
                normal: face.plane().n,
                plane_d: face.plane().d,
                texel,
                // Filled in by `weld_coplanar` once every face is measured.
                group: 0,
            });
        }
    }
    }
    weld_coplanar(&mut measured);
    if measured.is_empty() {
        return BrushLightmapLayout { width: 1, height: 1, charts: Vec::new(), density_scale: 1.0 };
    }

    // Second pass: choose a density that fits, then pack.
    //
    // Tried at full density first and only reduced if the result overflows, so
    // the common case -- a room, a few dozen faces -- keeps exactly the density
    // its faces asked for.
    let mut density_scale = 1.0_f64;
    for _ in 0..12 {
        if let Some(layout) = try_pack(&measured, density_scale) {
            return layout;
        }
        density_scale *= 0.5;
    }
    // Twelve halvings is a factor of four thousand; anything still not fitting
    // is degenerate rather than large. One texel per face keeps it renderable.
    try_pack(&measured, 0.0).unwrap_or(BrushLightmapLayout {
        width: 1,
        height: 1,
        charts: Vec::new(),
        density_scale: 0.0,
    })
}

/// How many texels a face of `metres` needs at `per_metre` density.
///
/// A plain `ceil` allocates a whole extra ROW on any face whose size divides
/// exactly, because the product lands a few ulps above the integer: a 3 m wall
/// at 4 texels/m measures 12.000000000000002 and rounds to 13.
///
/// This is a SPACE fix, not a correctness one -- `span_per_texel` sizes the
/// texels to whatever count comes out of here, so an extra row would still tile
/// the face, just more finely than asked. It is worth doing anyway because the
/// atlas is the scarce resource: a wasted row on every exactly-sized face in a
/// level is real, and levels are built out of whole-metre walls.
///
/// The epsilon is subtracted rather than added: the failure to prevent is
/// rounding a whole extra texel UP, and a face that genuinely needs 12.4 texels
/// still gets 13.
fn texels_across(metres: f64, per_metre: f64) -> u32 {
    let exact = metres * per_metre;
    // Relative, so it holds for a 0.25 m sliver and a 40 m hall alike.
    let eps = exact.abs() * 1e-9 + 1e-9;
    (exact - eps).ceil().max(1.0) as u32
}

/// Metres per texel along one axis, so `count` texels exactly span `metres`.
fn span_per_texel(metres: f64, count: u32) -> f64 {
    if count == 0 || metres <= 0.0 {
        return metres.max(1.0);
    }
    metres / count as f64
}

/// Give every set of coplanar faces on one brush a SHARED chart.
///
/// WHY A WALL ARRIVES HERE IN PIECES
///
/// CSG subtraction does not cut a hole in a face; it splits the whole solid
/// into convex pieces. The test room's shell -- one box with a room and a
/// doorway taken out of it -- evaluates to SEVEN solids, and each face of each
/// piece asked for a chart of its own. What is visibly one flat wall was
/// therefore lit as a dozen separate surfaces, every one with its own gutter.
///
/// Lighting is smooth inside a chart and discontinuous across the boundary
/// between two, so those boundaries showed up on the headset as a grid of hard
/// rectangular cells -- reported as "circular lights reflecting as squares".
/// Many of the charts were 2 texels wide, where a one-texel gutter on each side
/// leaves almost nothing that is not edge, so their values came mostly from
/// dilation rather than from a real gather.
///
/// The fix is to stop treating a subtraction artefact as a lighting boundary.
/// Faces sharing a plane, an object, a pair of texture axes and a texel size
/// are one surface as far as light is concerned, so they get one rectangle
/// spanning all of them. Texels that fall between two pieces are then baked as
/// ordinary points on that plane, and there is no seam left to be discontinuous
/// across.
///
/// Grouped on the AXES as well as the plane: two coplanar faces with different
/// authored `u_axis`/`v_axis` are deliberately different surfaces, and welding
/// them would silently re-project one of them.
/// A chart for a face whose weld group already has its rectangle.
///
/// The geometry is derived the same way as the first member's -- same plane,
/// same axes, same union extent -- so every member of a group agrees about
/// where each texel is in the world.
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
    // Quantised so that float noise from splitting a solid does not separate
    // two halves of what was one face. A tenth of a millimetre is far below any
    // authored geometry and far above the error a split introduces.
    const Q: f64 = 1e4;
    let q = |x: f64| (x * Q).round() as i64;
    let qv = |v: Vec3| [q(v[0]), q(v[1]), q(v[2])];

    let mut keys: Vec<([i64; 3], i64, [i64; 3], [i64; 3], i64, usize)> = Vec::new();
    for (i, m) in measured.iter().enumerate() {
        keys.push((qv(m.normal), q(m.plane_d), qv(m.u), qv(m.v), q(m.texel), i));
    }
    let mut groups: std::collections::HashMap<
        ([i64; 3], i64, [i64; 3], [i64; 3], i64, usize),
        usize,
    > = std::collections::HashMap::new();
    for (k0, k1, k2, k3, k4, i) in keys {
        // The OBJECT is part of the key: two brushes that happen to share a
        // plane are still two objects, and welding across them would make one
        // brush's lighting depend on another's geometry.
        let key = (k0, k1, k2, k3, k4, measured[i].object);
        let next = groups.len();
        let g = *groups.entry(key).or_insert(next);
        measured[i].group = g;
    }

    // Each group's union extent, in its own shared u/v axes.
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

    // A square-ish atlas: start from the total padded area and round up to a
    // power of two, which is what a GPU wants for mips and wrapping anyway.
    let area: u64 = sized
        .iter()
        .map(|(w, h)| ((w + 2 * GUTTER) as u64) * ((h + 2 * GUTTER) as u64))
        .sum();
    let mut width = (area as f64).sqrt().ceil().max(4.0) as u32;
    width = width.next_power_of_two().min(MAX_ATLAS);

    let mut charts = Vec::with_capacity(measured.len());
    let (mut shelf_y, mut shelf_h, mut cursor_x) = (0u32, 0u32, 0u32);
    // ONE RECTANGLE PER WELD GROUP. Coplanar faces share a surface, so they
    // share the atlas space that surface occupies -- which is what removes the
    // seam between them. Every face still gets its own `BrushChart`, so looking
    // one up by (object, solid, face) is unchanged; the members of a group just
    // all point at the same rectangle.
    let group_count = measured.iter().map(|m| m.group + 1).max().unwrap_or(0);
    let mut placed: Vec<Option<(u32, u32, u32, u32)>> = vec![None; group_count];
    for (m, (w, h)) in measured.iter().zip(sized.iter()) {
        if let Some((px, py, pw2, ph2)) = placed[m.group] {
            // Already allocated by an earlier member of this group.
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
        // Texel size from the face's OWN extent divided by the texels it got,
        // rather than from the nominal density.
        //
        // The two disagree whenever the count was rounded -- always upward, by
        // `ceil` or by the `MIN_CHART` clamp -- and using the nominal size then
        // makes the chart cover MORE than the face. The overshoot is a strip of
        // texel centres hanging off the edge in open air, shaded as if they were
        // surface: they see light the face cannot, and bilinear filtering pulls
        // it back into the visible texel beside them as a bright rim.
        //
        // Dividing instead makes the invariant true by construction -- every
        // texel centre lies on its own face -- for an exact fit, a rounded one
        // and a sliver clamped up to `MIN_CHART` alike.
        let du = mul(m.u, span_per_texel(m.w, *w));
        let dv = mul(m.v, span_per_texel(m.h, *h));
        // The chart's texel (0,0) centre, half a texel in from the face's
        // own minimum corner in its plane.
        let origin = add(
            add(mul(m.u, m.min_u), mul(m.v, m.min_v)),
            add(mul(du, 0.5), mul(dv, 0.5)),
        );
        // `u` and `v` span the face's DIRECTION but say nothing about how
        // far along the normal it sits, so the point built from them lies on
        // the parallel plane through the world origin. Lifting it onto the
        // face's own plane is what makes texel_world return points that are
        // actually on the surface -- without it every baked ray starts
        // somewhere else entirely, and for a wall at the origin it looks
        // perfectly correct.
        let along = dot(origin, m.normal);
        let origin = add(origin, mul(m.normal, m.plane_d - along));

        let (cx, cy) = (cursor_x + GUTTER, shelf_y + GUTTER);
        placed[m.group] = Some((cx, cy, *w, *h));
        charts.push(BrushChart {
            object: m.object,
            solid: m.solid,
            face: m.face,
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
        // Without the gutter a wall bleeds its neighbour's lighting along every
        // seam under bilinear filtering, which looks exactly like light leaking
        // through the corner -- the artefact baked lighting exists to remove.
        //
        // The separation is spelled as a literal 1 rather than as GUTTER on
        // purpose: written against the constant, this test relaxes in lockstep
        // with the code and setting GUTTER to 0 passes it -- checked by doing
        // exactly that, and it did. One empty texel is the guarantee the
        // sampler needs, whatever the constant is later set to.
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
        // THE property the baker depends on. If a texel's world point is not on
        // its own face, every ray starts in the wrong place -- and for a brush
        // sitting at the world origin it would still look right, which is how
        // this would survive a casual test.
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
        // Anything that genuinely needs a partial texel still rounds up.
        assert_eq!(texels_across(3.1, 4.0), 13);
        assert_eq!(texels_across(0.05, 4.0), 1, "a sliver still gets one texel");
    }

    #[test]
    fn every_texel_centre_lies_within_its_own_face_not_merely_on_its_plane() {
        // The stronger half of `a_texel_lands_on_the_face_it_belongs_to`, and
        // the half that was missing. A texel hanging a quarter of a metre below
        // the bottom of a wall is still exactly on that wall's PLANE, so the
        // plane check passes while the point sits in open air off the edge.
        //
        // It got there by rounding: `ceil(3.0 * 4.0)` on a face measuring
        // 3.0000000000000004 metres allocated 13 rows where 12 tile the wall,
        // and the `MIN_CHART` clamp does the same to a sliver. Those texels are
        // then shaded like surface -- in a sealed room the bottom row of every
        // inner wall gathered bounce from the sunlit UNDERSIDE of the floor,
        // through the floor -- and bilinear filtering carries it back into the
        // visible texel beside them as a bright rim along the seam.
        //
        // Deliberately includes a face whose size divides EXACTLY (3 m at 4
        // texels/m), because that is the case the floating-point ceil broke and
        // an awkward size would have hidden it.
        let brush = brush_at([-4.0, 0.0, -4.0], [4.0, 3.0, -3.75]);
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        for c in &layout.charts {
            let polys = crate::brush::solid_polygons(&solids[c.solid]);
            let poly = polys[c.face].as_ref().expect("charted face must have a polygon");
            // Extent of the real polygon along the chart's own axes.
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
        // The renderer asks one direction and the baker the other. If they are
        // not inverses the lighting is subtly displaced, which reads as a buggy
        // baker rather than a layout mismatch.
        let layout = brush_lightmap_layout(&room());
        for c in &layout.charts {
            for (tx, ty) in [(0, 0), (1, 1), (c.w - 1, c.h - 1)] {
                let world = c.texel_world(tx, ty);
                let uv = c.uv2(world, layout.width, layout.height);
                let back_x = uv[0] as f64 * layout.width as f64 - 0.5 - c.x as f64;
                let back_y = uv[1] as f64 * layout.height as f64 - 0.5 - c.y as f64;
                assert!((back_x - tx as f64).abs() < 1e-3, "u round trip {back_x} vs {tx}");
                assert!((back_y - ty as f64).abs() < 1e-3, "v round trip {back_y} vs {ty}");
            }
        }
    }

        #[test]
    fn uv2_of_a_faces_own_points_stays_inside_its_own_chart() {
        // Two walls sampling each other's texels is the failure this prevents,
        // and it is a one-texel error at the edges -- so the check is on the
        // real polygon corners, which are exactly where it would happen.
        let brush = room();
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        for c in &layout.charts {
            let polys = crate::brush::solid_polygons(&solids[c.solid]);
            let poly = polys[c.face].as_ref().expect("charted face must have a polygon");
            for p in poly {
                let uv = c.uv2(*p, layout.width, layout.height);
                let tx = uv[0] as f64 * layout.width as f64;
                let ty = uv[1] as f64 * layout.height as f64;
                assert!(
                    tx >= c.x as f64 - 1e-6 && tx <= (c.x + c.w) as f64 + 1e-6,
                    "corner sampled at {tx}, chart spans {}..{}", c.x, c.x + c.w,
                );
                assert!(
                    ty >= c.y as f64 - 1e-6 && ty <= (c.y + c.h) as f64 + 1e-6,
                    "corner sampled at {ty}, chart spans {}..{}", c.y, c.y + c.h,
                );
            }
        }
    }

    #[test]
    fn a_bigger_wall_gets_more_texels() {
        // Density is per metre, so resolution has to follow area rather than
        // face count -- otherwise a corridor and a cathedral wall get the same
        // budget and one of them is wasted.
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
        // A cap, not a promise: one huge brush should cost a blurry lightmap
        // rather than tens of megabytes of texture.
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
        // The bake is keyed to this layout. If it wandered, a rebake would be
        // required after edits that changed nothing about the geometry.
        assert_eq!(brush_lightmap_layout(&room()), brush_lightmap_layout(&room()));
    }

    #[test]
    fn two_brushes_share_one_atlas_without_colliding() {
        // The whole level packs into a single atlas so it can be drawn in one
        // call. Two brushes writing the same texels would have each lit by the
        // other's shadows, which looks like a baker bug rather than a packing
        // one.
        let a = brush_at([0.0, 0.0, 0.0], [4.0, 3.0, 0.5]);
        let b = brush_at([10.0, 0.0, 0.0], [12.0, 2.0, 0.5]);
        let layout = scene_brush_lightmap_layout(&[&a, &b]);

        assert!(layout.charts.iter().any(|c| c.object == 0));
        assert!(layout.charts.iter().any(|c| c.object == 1));
        for (i, x) in layout.charts.iter().enumerate() {
            for y in layout.charts.iter().skip(i + 1) {
                let apart_x = x.x + x.w + 1 <= y.x || y.x + y.w + 1 <= x.x;
                let apart_y = x.y + x.h + 1 <= y.y || y.y + y.h + 1 <= x.y;
                assert!(apart_x || apart_y, "charts of different brushes overlap:\n{x:?}\n{y:?}");
            }
        }
    }

    #[test]
    fn one_brush_packs_the_same_whether_asked_alone_or_as_a_scene() {
        // brush_lightmap_layout is the scene function with one brush. If those
        // ever diverged, the baker and the renderer could disagree simply
        // because one of them went through the convenience wrapper.
        let a = room();
        assert_eq!(brush_lightmap_layout(&a), scene_brush_lightmap_layout(&[&a]));
    }

    #[test]
    fn an_empty_brush_produces_a_usable_atlas_rather_than_a_zero_one() {
        // A 0x0 texture is not creatable; the renderer would fail at bind time
        // rather than simply having nothing to show.
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

    /// A wall split into pieces by a subtraction is still ONE wall.
    fn wall_with_a_hole_through_it() -> BrushDef {
        BrushDef {
            solids: vec![block([0.0, 0.0, 0.0], [8.0, 3.0, 0.25])],
            // A doorway, which splits the slab into several convex pieces.
            subtract: vec![block([3.0, 0.0, -1.0], [5.0, 2.0, 1.0])],
        }
    }

    #[test]
    fn a_subtraction_really_does_split_the_wall() {
        // The premise. If CSG stopped fragmenting, the tests below would pass
        // for the wrong reason.
        let brush = wall_with_a_hole_through_it();
        assert!(
            brush.evaluate().len() > 1,
            "this subtraction no longer splits the solid, so it cannot test welding",
        );
    }

    #[test]
    fn the_pieces_of_one_wall_share_one_chart() {
        // THE fix. Every piece's front face lies on the same plane, so they are
        // one surface for lighting and must land on one rectangle -- otherwise
        // each boundary between pieces is a lighting discontinuity, which is
        // what showed up on the headset as a grid of hard rectangles.
        let brush = wall_with_a_hole_through_it();
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();

        // Charts whose faces face +Z: the wall's front, in however many pieces.
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
        // The rectangle has to cover the UNION, not just one piece -- otherwise
        // the texels between two pieces have nowhere to live.
        let brush = wall_with_a_hole_through_it();
        let layout = brush_lightmap_layout(&brush);
        let solids = brush.evaluate();
        let front = layout
            .charts
            .iter()
            .find(|c| c.normal[2] > 0.9 && solids[c.solid].faces[c.face].plane().d > 0.2)
            .expect("a front face");
        // MEASURED IN METRES, not in texels. Spelling the answer as a texel
        // count pinned the default density into a test about welding, so
        // changing the density broke it while nothing about welding had moved.
        // What "spans the whole wall" means is that the rectangle covers the
        // surface, at whatever density the level is baked.
        let len = |v: Vec3| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let (du, dv) = (len(front.du), len(front.dv));
        let spanned_w = front.w as f64 * du;
        let spanned_h = front.h as f64 * dv;
        // Rounded UP to a whole texel, so at most one texel of slack per side.
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
        // The guard. Welding by plane must not merge a wall with its own back,
        // or with the floor -- they are different surfaces and share nothing.
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
        // Slivers are the expensive part: a 2-texel chart is almost entirely
        // gutter. Welding should therefore need no more room than before, and
        // in practice much less.
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

/// Is a point on this face's plane actually ON the face?
///
/// A chart spans the face's BOUNDING BOX in its own plane, which is the face
/// itself only when the face is a rectangle. Cut a doorway through a wall and
/// the box still covers the hole -- so texels land in open air in the opening,
/// get shaded as though they were wall, and see whatever is outside. Bilinear
/// filtering then pulls that into the wall beside them, which is a light leak
/// around every opening and in the corners next to one.
///
/// `tolerance` is in world units and should be about half a texel: a texel
/// whose centre sits just outside a true edge is still mostly on the surface,
/// and rejecting it would eat the face's own border.
pub fn point_on_face(poly: &[Vec3], normal: Vec3, p: Vec3, tolerance: f64) -> bool {
    if poly.len() < 3 {
        return false;
    }
    // SAME SIDE OF EVERY EDGE, without assuming which way the face is wound.
    //
    // Brush faces are convex by construction -- they are an intersection of
    // half spaces -- so "inside" is "on one side of all of them". Testing for
    // a NEGATIVE side instead would depend on the winding, and `solid_polygons`
    // does not promise one: an earlier version of this assumed it and reported
    // two thirds of the level's texels outside their own face, including chart
    // CENTRES, which is how the assumption was caught.
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
    // Inside means every signed distance shares a sign, allowing `tolerance`
    // of slop on the far side for a texel centre sitting just past an edge.
    most_positive <= tolerance || most_negative >= -tolerance
}

/// The nearest point on the face to `p`, for a texel that fell outside it.
///
/// Shading the clamped point rather than skipping the texel is what fills the
/// area around an opening with the EDGE's value -- which is exactly the
/// dilation a lightmap needs, obtained without a second pass over the atlas.
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

    /// A wall: x = 3, spanning y -0.3..3.4 and z -16..4. Taken from the real
    /// scene rather than invented, so the winding is whatever
    /// `solid_polygons` actually produces rather than what is convenient.
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

    /// WINDING MUST NOT MATTER. `solid_polygons` makes no promise about it,
    /// and an earlier version of this assumed one -- which reported chart
    /// centres as being off their own face.
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

    /// A texel centre just past an edge is still mostly on the surface, so the
    /// tolerance keeps it -- otherwise the face loses its own border.
    #[test]
    fn a_texel_just_past_an_edge_is_kept() {
        let (poly, n) = wall();
        assert!(point_on_face(&poly, n, [3.0, 3.41, -6.0], 0.02), "a texel 1cm past the top was dropped");
        assert!(!point_on_face(&poly, n, [3.0, 3.8, -6.0], 0.02), "a texel 40cm past the top was kept");
    }

    /// Clamping puts an outside point back on the face, at the nearest edge.
    #[test]
    fn clamping_lands_on_the_nearest_edge() {
        let (poly, _) = wall();
        let q = clamp_to_face(&poly, [3.0, 9.0, -6.0]);
        assert!((q[1] - 3.4).abs() < 1e-6, "clamped to y={} rather than the top edge", q[1]);
        assert!((q[2] + 6.0).abs() < 1e-6, "clamping moved the point along the edge");
        // A point already inside is left where it is, near enough.
        let inside = [3.0, 1.55, -6.125];
        let r = clamp_to_face(&poly, inside);
        assert!(
            (r[1] - inside[1]).abs() < 2.0,
            "clamping moved an interior point a long way: {r:?}",
        );
    }

    /// A degenerate face is not a crash.
    #[test]
    fn too_few_vertices_is_not_a_face() {
        assert!(!point_on_face(&[[0.0; 3], [1.0, 0.0, 0.0]], [0.0, 0.0, 1.0], [0.0; 3], 0.1));
        assert_eq!(clamp_to_face(&[], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    }
}

#[cfg(test)]
mod scaled_layout_tests {
    use super::*;

    /// A point on a face has the SAME uv2 in the lightmap and in the mask, so
    /// one set of mesh coordinates samples both.
    #[test]
    fn a_scaled_layout_keeps_every_points_uv2() {
        let solid = crate::brush::block_solid([0.0, 0.0, 0.0], [4.0, 3.0, 0.3], "m");
        let brush = crate::brush::BrushDef { solids: vec![solid], subtract: Vec::new() };
        let base = scene_brush_lightmap_layout(&[&brush]);
        let fine = base.scaled(SUN_MASK_SCALE);
        for (a, b) in base.charts.iter().zip(&fine.charts) {
            for (fu, fv) in [(0.0, 0.0), (0.37, 0.81), (0.999, 0.5)] {
                let p = add(a.texel_world(0, 0), add(mul(a.du, fu * a.w as f64), mul(a.dv, fv * a.h as f64)));
                let (ua, ub) = (a.uv2(p, base.width, base.height), b.uv2(p, fine.width, fine.height));
                assert!((ua[0] - ub[0]).abs() < 1e-6 && (ua[1] - ub[1]).abs() < 1e-6, "{ua:?} vs {ub:?}");
            }
            // And the fine texels tile the coarse one: sub-texel centres at
            // +-1/8 and +-3/8 of a coarse texel around its centre.
            let c0 = a.texel_world(0, 0);
            let f0 = b.texel_world(0, 0);
            let want = add(c0, mul(add(a.du, a.dv), -0.375));
            assert!((0..3).all(|k| (f0[k] - want[k]).abs() < 1e-9), "{f0:?} vs {want:?}");
        }
    }
}
