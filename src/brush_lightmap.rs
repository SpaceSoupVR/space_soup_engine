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

use crate::brush::{solid_polygons, BrushDef};
use crate::brush::Vec3;

/// Texels of empty space kept around every chart, to stop bilinear bleed.
pub const GUTTER: u32 = 1;

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
}

/// One face's patch of the atlas, and how it maps to the world.
#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrushLightmapLayout {
    pub width: u32,
    pub height: u32,
    pub charts: Vec<BrushChart>,
    /// Multiplier applied to every face's requested density to fit `MAX_ATLAS`.
    /// 1.0 when nothing had to be given up.
    pub density_scale: f64,
}

impl BrushLightmapLayout {
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
    for (si, solid) in solids.iter().enumerate() {
        for (fi, poly) in solid_polygons(solid).into_iter().enumerate() {
            let Some(poly) = poly else { continue };
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
                w: (max_u - min_u).max(0.0),
                h: (max_v - min_v).max(0.0),
                min_u,
                min_v,
                u,
                v,
                normal: face.plane().n,
                plane_d: face.plane().d,
                texel,
            });
        }
    }
    }
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

fn try_pack(measured: &[Measured], density_scale: f64) -> Option<BrushLightmapLayout> {
    let sized: Vec<(u32, u32)> = measured
        .iter()
        .map(|m| {
            let per_metre = if m.texel > 0.0 { density_scale / m.texel } else { 0.0 };
            (
                ((m.w * per_metre).ceil() as u32).clamp(MIN_CHART, MAX_ATLAS),
                ((m.h * per_metre).ceil() as u32).clamp(MIN_CHART, MAX_ATLAS),
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
    for (m, (w, h)) in measured.iter().zip(sized.iter()) {
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
        let per_metre = if m.texel > 0.0 { density_scale / m.texel } else { 0.0 };
        let metres_per_texel = if per_metre > 0.0 { 1.0 / per_metre } else { m.w.max(m.h).max(1.0) };
        let du = mul(m.u, metres_per_texel);
        let dv = mul(m.v, metres_per_texel);
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

        charts.push(BrushChart {
            object: m.object,
            solid: m.solid,
            face: m.face,
            x: cursor_x + GUTTER,
            y: shelf_y + GUTTER,
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
