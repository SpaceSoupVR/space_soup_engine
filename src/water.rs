//! Standing water: lakes, ponds, flooded rooms.
//!
//! WHY A DEFINITION AND NOT A MESH
//!
//! Water is a surface, a set of optical properties and a rule for how deep it is
//! at each point. Authoring it as geometry would force the author to model the
//! shoreline by hand and re-model it every time the ground under it changes,
//! which is exactly the coupling the heightfield exists to remove: the shore is
//! wherever the ground rises through the water plane, and that is a fact about
//! the terrain, not about the water.
//!
//! So a level says "there is water at this height over this footprint" and the
//! shoreline follows the ground for free.

use serde::{Deserialize, Serialize};

/// One body of standing water.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaterDef {
    /// World Y of the still surface.
    pub height: f32,

    /// Footprint as `[min_x, min_z, max_x, max_z]`.
    ///
    /// A rectangle rather than a polygon: the visible edge of a body of water is
    /// its SHORELINE, which comes from the terrain, and the rectangle only has
    /// to be big enough to contain it. A lake in a valley needs a box around the
    /// valley, not an outline of the lake.
    pub bounds: [f32; 4],

    /// Colour where the water is shallow, and where it is deep.
    ///
    /// Two colours rather than one plus a darkening factor: real water shifts
    /// HUE with depth as well as brightness -- a sandy shallow is green-brown
    /// and the same water deep is blue -- and a single colour cannot.
    #[serde(default = "default_shallow")]
    pub shallow: [u8; 3],
    #[serde(default = "default_deep")]
    pub deep: [u8; 3],

    /// Metres of depth over which `shallow` reaches `deep`.
    #[serde(default = "default_depth_scale")]
    pub depth_scale: f32,

    /// Metres of depth over which the surface fades in at the shoreline.
    ///
    /// Without it the water meets the ground along a hard line that no real
    /// shore has, and which flickers as the camera moves because it is a
    /// geometric intersection sampled per pixel.
    #[serde(default = "default_shore_fade")]
    pub shore_fade: f32,

    /// How fast the wave pattern scrolls, in metres per second.
    #[serde(default = "default_wave_speed")]
    pub wave_speed: f32,

    /// Metres per wave.
    #[serde(default = "default_wave_scale")]
    pub wave_scale: f32,

    /// How far the waves tilt the surface normal, 0..1.
    #[serde(default = "default_wave_strength")]
    pub wave_strength: f32,

    /// Opacity looking straight DOWN into the water.
    ///
    /// Only the head-on value is authored; at grazing angles Fresnel takes the
    /// surface to a mirror on its own, which is why a lake is transparent at
    /// your feet and reflects the far bank.
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

fn default_shallow() -> [u8; 3] { [90, 150, 140] }
fn default_deep() -> [u8; 3] { [10, 40, 70] }
fn default_depth_scale() -> f32 { 4.0 }
fn default_shore_fade() -> f32 { 0.35 }
fn default_wave_speed() -> f32 { 0.35 }
fn default_wave_scale() -> f32 { 2.5 }
fn default_wave_strength() -> f32 { 0.35 }
fn default_opacity() -> f32 { 0.72 }

impl WaterDef {
    /// Footprint extent, `(min_x, min_z, max_x, max_z)`, normalised so min <= max.
    ///
    /// Normalised rather than trusted: an author dragging a rectangle upward and
    /// to the left produces a box with its corners the other way round, and
    /// every consumer would otherwise have to remember to handle it.
    pub fn extent(&self) -> (f32, f32, f32, f32) {
        let [a, b, c, d] = self.bounds;
        (a.min(c), b.min(d), a.max(c), b.max(d))
    }

    /// Whether a world x/z is inside the footprint.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (x0, z0, x1, z1) = self.extent();
        x >= x0 && x <= x1 && z >= z0 && z <= z1
    }

    /// Water depth at a ground height, clamped at zero.
    ///
    /// Zero on dry land, which is what the shore fade keys off.
    pub fn depth_over(&self, ground_y: f32) -> f32 {
        (self.height - ground_y).max(0.0)
    }
}

/// One vertex of a water surface.
///
/// Depth travels with the VERTEX rather than being looked up in the shader.
/// The alternative is sampling the scene depth buffer, which on a tile GPU
/// means resolving and re-reading it -- the single most expensive thing a
/// transparent pass can ask for, and it buys nothing here: the ground under
/// standing water does not move, so its depth can be measured once at load.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterVertex {
    pub position: [f32; 3],
    /// Metres of water above the ground at this point.
    pub depth: f32,
}

/// Metres between vertices of the water grid.
///
/// The grid exists only to carry depth, so its resolution is set by how fast
/// the SHORELINE turns rather than by the waves, which are per-pixel. Half a
/// metre keeps a shore from visibly faceting without making a lake cost more
/// than the terrain under it.
pub const WATER_GRID_STEP: f32 = 0.5;

/// Tessellate a water body, sampling `ground` for depth at each vertex.
///
/// Returns `None` when no part of the footprint is actually underwater -- a
/// body whose surface sits below the ground everywhere is authored, but there
/// is nothing to draw, and returning an empty mesh would have every consumer
/// check for it separately.
pub fn build_surface(
    def: &WaterDef,
    ground: impl Fn(f32, f32) -> Option<f32>,
) -> Option<(Vec<WaterVertex>, Vec<u32>)> {
    let (x0, z0, x1, z1) = def.extent();
    let nx = (((x1 - x0) / WATER_GRID_STEP).ceil() as usize).max(1) + 1;
    let nz = (((z1 - z0) / WATER_GRID_STEP).ceil() as usize).max(1) + 1;

    let mut verts = Vec::with_capacity(nx * nz);
    let mut any_wet = false;
    for iz in 0..nz {
        for ix in 0..nx {
            // The last row and column land exactly on the far edge rather than
            // one step short of it, so a footprint that is not a whole number
            // of steps still reaches its own boundary.
            let x = if ix + 1 == nx { x1 } else { x0 + ix as f32 * WATER_GRID_STEP };
            let z = if iz + 1 == nz { z1 } else { z0 + iz as f32 * WATER_GRID_STEP };
            // Ground outside the terrain is treated as infinitely deep rather
            // than as dry: water running off the edge of the heightfield should
            // carry on, not stop in a straight line.
            let depth = match ground(x, z) {
                Some(g) => def.depth_over(g),
                None => def.depth_scale,
            };
            if depth > 0.0 {
                any_wet = true;
            }
            verts.push(WaterVertex { position: [x, def.height, z], depth });
        }
    }
    if !any_wet {
        return None;
    }

    let mut indices = Vec::with_capacity((nx - 1) * (nz - 1) * 6);
    for iz in 0..nz.saturating_sub(1) {
        for ix in 0..nx.saturating_sub(1) {
            let a = (iz * nx + ix) as u32;
            let b = a + 1;
            let c = a + nx as u32;
            let d = c + 1;
            // Dropped per TRIANGLE, not per quad. A lake in a valley is a small
            // fraction of its bounding box, and the dry part is both invisible
            // and the most expensive kind of nothing -- a transparent surface
            // pays its fill even at zero alpha.
            //
            // Safe to drop a fully-dry triangle because depth is interpolated:
            // three dry corners means the whole triangle is dry, so nothing is
            // lost. The neighbouring triangle that DOES touch water still draws
            // and fades to nothing along the shared edge, so the shore stays
            // continuous.
            //
            // Per-quad culling keeps a triangle whose own three corners are all
            // dry whenever its diagonal partner happens to be wet -- a strip of
            // invisible fill running the length of every shore.
            let wet = |i: u32| verts[i as usize].depth > 0.0;
            for tri in [[a, c, b], [b, c, d]] {
                if tri.iter().any(|&i| wet(i)) {
                    indices.extend_from_slice(&tri);
                }
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    Some((verts, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn water() -> WaterDef {
        WaterDef {
            height: 2.0,
            bounds: [-10.0, -10.0, 10.0, 10.0],
            shallow: default_shallow(),
            deep: default_deep(),
            depth_scale: default_depth_scale(),
            shore_fade: default_shore_fade(),
            wave_speed: default_wave_speed(),
            wave_scale: default_wave_scale(),
            wave_strength: default_wave_strength(),
            opacity: default_opacity(),
        }
    }

    #[test]
    fn a_rectangle_dragged_backwards_still_describes_the_same_box() {
        let mut w = water();
        w.bounds = [10.0, 10.0, -10.0, -10.0];
        assert_eq!(w.extent(), (-10.0, -10.0, 10.0, 10.0));
        assert!(w.contains(0.0, 0.0), "an inverted rectangle must still contain its middle");
    }

    #[test]
    fn ground_above_the_surface_has_no_depth() {
        let w = water();
        assert_eq!(w.depth_over(5.0), 0.0, "a hill standing out of the water is not -3m deep");
        assert_eq!(w.depth_over(2.0), 0.0, "ground exactly at the surface is the shoreline");
        assert!((w.depth_over(-1.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn every_optical_property_has_a_default() {
        // A level should be able to say "water at this height over this box" and
        // get something that looks like water. Requiring eight numbers to place
        // a pond is how a feature goes unused.
        let minimal: WaterDef = serde_json::from_str(
            r#"{"height": 1.5, "bounds": [0, 0, 4, 4]}"#,
        )
        .expect("height and bounds alone must deserialise");
        assert_eq!(minimal.height, 1.5);
        assert_eq!(minimal.opacity, default_opacity());
        assert_eq!(minimal.deep, default_deep());
    }

    #[test]
    fn a_flat_bed_below_the_surface_tessellates_and_carries_depth() {
        let w = water(); // surface at y = 2, bounds 20m square
        let (verts, indices) = build_surface(&w, |_, _| Some(0.0)).expect("all of it is wet");
        assert!(!indices.is_empty() && indices.len() % 3 == 0);
        for v in &verts {
            assert!((v.depth - 2.0).abs() < 1e-6, "flat bed at y=0 under y=2 is 2m deep");
            assert!((v.position[1] - 2.0).abs() < 1e-6, "every vertex sits ON the surface");
        }
    }

    #[test]
    fn a_bed_entirely_above_the_surface_produces_nothing() {
        // Authored water that is nowhere actually water. Drawing an invisible
        // transparent sheet still costs its fill on a tile GPU, which is the
        // most expensive kind of nothing.
        let w = water();
        assert!(build_surface(&w, |_, _| Some(9.0)).is_none());
    }

    #[test]
    fn dry_quads_are_dropped_but_the_wet_ones_survive() {
        // A lake in a valley: only the middle is below the surface. The corners
        // of the bounding box must not be drawn.
        let w = water();
        let bowl = |x: f32, z: f32| Some(if x.abs() < 4.0 && z.abs() < 4.0 { -1.0 } else { 6.0 });
        let (verts, indices) = build_surface(&w, bowl).expect("the middle is wet");

        let full = build_surface(&w, |_, _| Some(0.0)).unwrap().1.len();
        assert!(
            indices.len() < full / 2,
            "a small pond in a big box must drop most of its quads: {} vs {full}",
            indices.len(),
        );
        // Every drawn triangle must touch water somewhere.
        for tri in indices.chunks_exact(3) {
            assert!(
                tri.iter().any(|&i| verts[i as usize].depth > 0.0),
                "a triangle was kept with all three corners on dry land",
            );
        }
    }

    #[test]
    fn water_running_off_the_heightfield_keeps_going() {
        // `None` from the ground sampler means "outside the terrain", not "dry".
        // Treating it as dry ends a lake in a straight line at the map edge.
        let w = water();
        let (verts, _) = build_surface(&w, |x, _| if x < 0.0 { Some(0.0) } else { None })
            .expect("half of it is over known ground");
        assert!(
            verts.iter().filter(|v| v.position[0] > 0.0).all(|v| v.depth > 0.0),
            "water past the edge of the terrain must still be wet",
        );
    }

    #[test]
    fn the_grid_reaches_its_own_far_edge() {
        // A footprint that is not a whole number of steps must still span its
        // bounds exactly, or the water stops short of the shore it was drawn to.
        let mut w = water();
        w.bounds = [0.0, 0.0, 3.3, 3.3];
        let (verts, _) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let max_x = verts.iter().fold(f32::MIN, |m, v| m.max(v.position[0]));
        let max_z = verts.iter().fold(f32::MIN, |m, v| m.max(v.position[2]));
        assert!((max_x - 3.3).abs() < 1e-5, "grid stopped at {max_x}, not 3.3");
        assert!((max_z - 3.3).abs() < 1e-5, "grid stopped at {max_z}, not 3.3");
    }

    #[test]
    fn a_water_def_survives_a_round_trip() {
        let w = water();
        let text = serde_json::to_string(&w).unwrap();
        assert_eq!(serde_json::from_str::<WaterDef>(&text).unwrap(), w);
    }
}
