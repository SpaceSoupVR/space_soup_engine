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

    /// What a white bed looks like seen straight down through `depth_scale`
    /// metres of this water: how clear the water is, and the hue depth gives
    /// it -- a sandy shallow green-blue, a peat pond brown.
    ///
    /// A colour rather than three absorption coefficients because a colour is
    /// what an author can see and pick. The renderer turns it into them: each
    /// channel of light falls off as exp(-k d), on its way down to the bed
    /// and back up, and k is whatever takes white to this colour.
    #[serde(default = "default_shallow")]
    pub shallow: [u8; 3],
    /// The colour of water too deep to see the bottom of: the light the water
    /// itself scatters back up, lit by the sky and the sun.
    #[serde(default = "default_deep")]
    pub deep: [u8; 3],

    /// Metres of water through which a white bed looks `shallow`.
    #[serde(default = "default_depth_scale")]
    pub depth_scale: f32,

    /// Metres of depth over which the surface fades in at the shoreline.
    ///
    /// Without it the water meets the ground along a hard line that no real
    /// shore has, and which flickers as the camera moves because it is a
    /// geometric intersection sampled per pixel.
    #[serde(default = "default_shore_fade")]
    pub shore_fade: f32,

    /// Wind speed ten metres up, m/s. With `fetch`, how big the waves are: the
    /// renderer builds the sea a wind like this raises (a JONSWAP spectrum --
    /// see `space_soup::renderer::water_waves`). 2 ruffles a pond, 7 is a
    /// fresh breeze, 15 a gale.
    #[serde(default = "default_wind_speed")]
    pub wind_speed: f32,

    /// The way the wind blows, degrees from +x toward +z.
    #[serde(default = "default_wind_direction")]
    pub wind_direction: f32,

    /// Metres of open water the wind has blown across. A pond's waves stay
    /// small in any wind because there is no room for them to grow.
    #[serde(default = "default_fetch")]
    pub fetch: f32,

    /// How sharp the crests are: 0 rolls, 1 peaks them.
    #[serde(default = "default_choppiness")]
    pub choppiness: f32,

    /// Metres the surface carries on past `bounds`, all round, at its own
    /// height: 0 for a lake held in its box, kilometres for a sea that runs
    /// out to the horizon. The ground under the skirt is taken as deep.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skirt: f32,

    /// How high, in metres, waves run up a beach: breakers roll in toward the
    /// shore, and their wash climbs the sand and drains back. 0 for a still
    /// shore.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub swash: f32,

    /// Metres the whole surface heaves in long, slow undulations, beyond what
    /// its wind raises -- the gentle rise and fall a sheltered pond keeps
    /// from passing gusts and its own basin's slosh, where a light breeze over
    /// a few metres raises only ripples too short to lift it. 0 for none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub swell: f32,

    /// The author's name for the body, for the editor's lists; the renderer
    /// ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The deepest of this water a walk goes into, metres, where it is not
    /// [`DEFAULT_WADE_DEPTH`]: a pond to walk into on its bed, head under and
    /// all, where the sea stops a player thigh deep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wade_depth: Option<f32>,
}

/// The deepest water a walk goes into, metres, unless the water says
/// otherwise ([`WaterDef::max_wade_depth`]): knee to thigh deep.
pub const DEFAULT_WADE_DEPTH: f32 = 0.6;

fn default_shallow() -> [u8; 3] { [90, 150, 140] }
fn default_deep() -> [u8; 3] { [10, 40, 70] }
fn default_depth_scale() -> f32 { 4.0 }
fn default_shore_fade() -> f32 { 0.35 }
fn default_wind_speed() -> f32 { 3.0 }
fn default_wind_direction() -> f32 { 30.0 }
fn default_fetch() -> f32 { 100.0 }
fn default_choppiness() -> f32 { 0.6 }
fn is_zero(v: &f32) -> bool { *v == 0.0 }

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
    /// Metres from the still surface down to the ground at this point:
    /// negative where the ground stands above the surface -- a beach, where
    /// the shore's waves run up. [`OPEN_WATER_DEPTH`] where the ground is
    /// unknown.
    pub depth: f32,
}

/// Metres between vertices of the water grid, at the finest.
///
/// The grid carries depth and the long waves' rise and fall. The shoreline is
/// drawn per pixel, where the surface passes into the ground, and the ripples
/// are per-pixel normals, so neither needs vertices. Half a metre keeps a
/// small pond's surface from faceting as its waves move it.
pub const WATER_GRID_STEP: f32 = 0.5;

/// The most grid steps along a side. A sea's bounds are a hundred metres and
/// more across, a quarter of a million vertices at half a metre, so a big
/// body's grid is coarser -- the waves big water has are longer anyway.
pub const WATER_GRID_MAX_STEPS: f32 = 200.0;

/// Metres of water taken to lie under the surface where the ground is not
/// known -- past the edge of the terrain, and under a skirt: deep enough that
/// nothing shows through, which is how open water looks.
pub const OPEN_WATER_DEPTH: f32 = 100.0;

/// Rings in a skirt, from the bounds out to its far edge, each about twice as
/// far out as the last: as fine as the grid where it meets it, kilometres
/// coarse at the horizon, where a ring is a pixel or two tall.
const SKIRT_RINGS: usize = 8;

/// A body's grid over its footprint: each vertex's still depth, and which of
/// them the water reaches.
///
/// One function for the drawn surface ([`build_surface`]) and for
/// [`WaterFootprint`], so the water a walk is stopped by is the water drawn.
struct Grid {
    step: f32,
    nx: usize,
    nz: usize,
    verts: Vec<WaterVertex>,
    wet: Vec<bool>,
}

/// `None` when no part of the footprint is underwater.
fn grid(def: &WaterDef, ground: impl Fn(f32, f32) -> Option<f32>) -> Option<Grid> {
    let (x0, z0, x1, z1) = def.extent();
    let step = WATER_GRID_STEP.max((x1 - x0).max(z1 - z0) / WATER_GRID_MAX_STEPS);
    let nx = (((x1 - x0) / step).ceil() as usize).max(1) + 1;
    let nz = (((z1 - z0) / step).ceil() as usize).max(1) + 1;
    // Ground this far above the still surface is where the shore's waves
    // reach, so it keeps its surface: a wave needs something to run up. While
    // the water is down, that surface is under the sand and the ground hides it.
    let reach = def.swash.max(0.0);

    let mut verts = Vec::with_capacity(nx * nz);
    let mut any_wet = def.skirt > 0.0;
    for iz in 0..nz {
        for ix in 0..nx {
            // The last row and column land exactly on the far edge rather than
            // one step short of it, so a footprint that is not a whole number
            // of steps still reaches its own boundary.
            let x = if ix + 1 == nx { x1 } else { x0 + ix as f32 * step };
            let z = if iz + 1 == nz { z1 } else { z0 + iz as f32 * step };
            // Ground outside the terrain is treated as deep rather than as
            // dry: water running off the edge of the heightfield should carry
            // on, not stop in a straight line.
            let depth = match ground(x, z) {
                Some(g) => def.height - g,
                None => OPEN_WATER_DEPTH,
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

    // A SEA IS THE WATER JOINED TO ITS SKIRT. Ground below its surface but
    // ringed by land -- a pond's basin, a cellar -- is not the sea, however
    // low it lies: flooded from the open edge, it stays dry. A lake has no
    // skirt and is the water in its box.
    let wet: Vec<bool> = if def.skirt > 0.0 {
        let mut reached = vec![false; verts.len()];
        let mut frontier: Vec<usize> = (0..verts.len())
            .filter(|&i| {
                let (ix, iz) = (i % nx, i / nx);
                (ix == 0 || iz == 0 || ix + 1 == nx || iz + 1 == nz) && verts[i].depth > -reach
            })
            .collect();
        for &i in &frontier {
            reached[i] = true;
        }
        while let Some(i) = frontier.pop() {
            let (ix, iz) = (i % nx, i / nx);
            let mut visit = |j: usize| {
                if !reached[j] && verts[j].depth > -reach {
                    reached[j] = true;
                    frontier.push(j);
                }
            };
            if ix > 0 { visit(i - 1); }
            if ix + 1 < nx { visit(i + 1); }
            if iz > 0 { visit(i - nx); }
            if iz + 1 < nz { visit(i + nx); }
        }
        reached
    } else {
        verts.iter().map(|v| v.depth > -reach).collect()
    };
    Some(Grid { step, nx, nz, verts, wet })
}

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
    let Grid { step, nx, nz, mut verts, wet } = grid(def, ground)?;
    let (x0, z0, x1, z1) = def.extent();

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
            for tri in [[a, c, b], [b, c, d]] {
                if tri.iter().any(|&i| wet[i as usize]) {
                    indices.extend_from_slice(&tri);
                }
            }
        }
    }

    // THE SKIRT: rings round the grid, out to the horizon. The first is the
    // grid's own edge -- the same vertices, so nothing can open between them
    // however the waves move it -- and each further ring is the last scaled
    // out from the middle.
    if def.skirt > 0.0 {
        let mut ring: Vec<u32> = (0..nx).map(|ix| ix as u32).collect();
        ring.extend((1..nz).map(|iz| (iz * nx + nx - 1) as u32));
        ring.extend((0..nx - 1).rev().map(|ix| ((nz - 1) * nx + ix) as u32));
        ring.extend((1..nz - 1).rev().map(|iz| (iz * nx) as u32));
        let (cx, cz) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
        let half = ((x1 - x0).min(z1 - z0) * 0.5).max(step);
        let far = 1.0 + def.skirt / half;
        let mut inner = ring.clone();
        for k in 1..=SKIRT_RINGS {
            let s = far.powf(k as f32 / SKIRT_RINGS as f32);
            let base = verts.len() as u32;
            for &i in &ring {
                let [x, _, z] = verts[i as usize].position;
                verts.push(WaterVertex {
                    position: [cx + (x - cx) * s, def.height, cz + (z - cz) * s],
                    depth: OPEN_WATER_DEPTH,
                });
            }
            for j in 0..ring.len() {
                let jn = (j + 1) % ring.len();
                let (a, b) = (inner[j], inner[jn]);
                let (c, d) = (base + j as u32, base + jn as u32);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
            inner = (base..base + ring.len() as u32).collect();
        }
    }
    if indices.is_empty() {
        return None;
    }
    Some((verts, indices))
}

/// Where a body's water stands, kept to ask how deep it is under someone's
/// feet.
///
/// The drawn surface's own grid and flood fill rather than a second rule, so
/// a walk is stopped by the water on screen -- and a basin a sea leaves dry
/// is dry to walk in too.
#[derive(Debug, Clone)]
pub struct WaterFootprint {
    height: f32,
    wade: f32,
    extent: (f32, f32, f32, f32),
    step: f32,
    nx: usize,
    nz: usize,
    wet: Vec<bool>,
    skirt: f32,
}

impl WaterFootprint {
    /// `None` where the body has no water at all, as for [`build_surface`].
    pub fn new(def: &WaterDef, ground: impl Fn(f32, f32) -> Option<f32>) -> Option<Self> {
        let Grid { step, nx, nz, wet, .. } = grid(def, ground)?;
        Some(Self {
            height: def.height,
            wade: def.max_wade_depth.unwrap_or(DEFAULT_WADE_DEPTH),
            extent: def.extent(),
            step,
            nx,
            nz,
            wet,
            skirt: def.skirt.max(0.0),
        })
    }

    /// The deepest of this water a walk goes into, metres.
    pub fn wade_depth(&self) -> f32 {
        self.wade
    }

    /// The still surface's height over a world x/z, where this water stands
    /// there: in a grid cell the surface covers, or out on its skirt.
    pub fn surface_at(&self, x: f32, z: f32) -> Option<f32> {
        let (x0, z0, x1, z1) = self.extent;
        if x < x0 || z < z0 || x > x1 || z > z1 {
            let out = (x0 - x).max(x - x1).max(z0 - z).max(z - z1);
            return (out <= self.skirt).then_some(self.height);
        }
        // The cell holding the point. The last row and column sit on the far
        // edge, as in the grid, so the last cell may be narrower than a step.
        let ix = (((x - x0) / self.step) as usize).min(self.nx - 2);
        let iz = (((z - z0) / self.step) as usize).min(self.nz - 2);
        let a = iz * self.nx + ix;
        [a, a + 1, a + self.nx, a + self.nx + 1].iter().any(|&i| self.wet[i]).then_some(self.height)
    }
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
            wind_speed: default_wind_speed(),
            wind_direction: default_wind_direction(),
            fetch: default_fetch(),
            choppiness: default_choppiness(),
            skirt: 0.0,
            swash: 0.0,
        swell: 0.0,
        name: None,
        max_wade_depth: None,
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
        assert_eq!(minimal.wind_speed, default_wind_speed());
        assert_eq!((minimal.skirt, minimal.swash), (0.0, 0.0), "a lake has no skirt and no surf");
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
    fn depth_below_the_surface_is_signed() {
        // A beach: ground above the still surface carries a NEGATIVE depth,
        // which is how far a wave must rise to reach it.
        let w = water(); // surface at y = 2
        let (verts, _) = build_surface(&w, |x, _| Some(if x < 0.0 { 0.0 } else { 2.5 })).unwrap();
        let dry = verts.iter().find(|v| v.position[0] > 1.0).unwrap();
        assert!((dry.depth + 0.5).abs() < 1e-6, "ground 0.5 m above the surface is -0.5 m deep, got {}", dry.depth);
    }

    #[test]
    fn a_beach_the_waves_reach_keeps_its_surface() {
        // Sand rising out of the water: with no swash its triangles are all
        // dry and dropped; with 0.3 m of swash the first 0.3 m of it is kept,
        // or the waves would have nothing to run up.
        let mut w = water();
        let beach = |x: f32, _: f32| Some(2.0 + 0.05 * x); // rises 5 cm a metre
        let still = build_surface(&w, beach).unwrap().1.len();
        w.swash = 0.3;
        let (verts, indices) = build_surface(&w, beach).unwrap();
        assert!(indices.len() > still, "swash must keep more surface: {} vs {still}", indices.len());
        for tri in indices.chunks_exact(3) {
            assert!(
                tri.iter().any(|&i| verts[i as usize].depth > -0.3),
                "a triangle was kept above where the waves reach",
            );
        }
    }

    #[test]
    fn a_skirt_carries_the_surface_out_to_the_horizon() {
        let mut w = water(); // 20 m square at y = 2
        w.skirt = 5000.0;
        let (verts, indices) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let reach = verts.iter().map(|v| v.position[0].abs().max(v.position[2].abs())).fold(0.0f32, f32::max);
        assert!(reach >= 5000.0, "the skirt reaches {reach} m, not 5 km past the bounds");
        assert!(verts.iter().all(|v| v.position[1] == 2.0), "a skirt lies at the surface's own height");
        assert!(indices.iter().all(|&i| (i as usize) < verts.len()));
        // The first ring IS the grid's edge: no second vertex at any edge point,
        // so no crack can open between them.
        let edge = verts.iter().filter(|v| v.position[0].abs() == 10.0 && v.position[2].abs() <= 10.0).count();
        let side = (20.0 / WATER_GRID_STEP) as usize + 1;
        assert_eq!(edge, 2 * side, "the grid's edge was duplicated for the skirt");
        // Past the bounds the ground is unknown, so deep.
        assert!(verts.iter().filter(|v| v.position[0].abs() > 10.0).all(|v| v.depth == OPEN_WATER_DEPTH));
    }

    #[test]
    fn a_sea_does_not_fill_a_basin_ringed_by_land() {
        // An island in a sea at y = 2, with a pond's basin in its middle that
        // dips below the sea's surface. The basin is not the sea.
        let mut w = water();
        w.bounds = [-20.0, -20.0, 20.0, 20.0];
        w.skirt = 100.0;
        let island = |x: f32, z: f32| {
            let r = (x * x + z * z).sqrt();
            Some(if r < 3.0 { 0.0 } else if r < 12.0 { 4.0 } else { 0.0 })
        };
        let (verts, indices) = build_surface(&w, island).unwrap();
        let in_basin = indices.chunks_exact(3).filter(|t| {
            t.iter().all(|&i| {
                let [x, _, z] = verts[i as usize].position;
                (x * x + z * z).sqrt() < 3.0
            })
        });
        assert_eq!(in_basin.count(), 0, "the sea flooded a basin it cannot reach");
        // A lake in the same box, with no skirt, is the water in its box: the
        // basin is its water.
        w.skirt = 0.0;
        let (verts, indices) = build_surface(&w, island).unwrap();
        assert!(indices.chunks_exact(3).any(|t| t.iter().all(|&i| {
            let [x, _, z] = verts[i as usize].position;
            (x * x + z * z).sqrt() < 3.0
        })));
    }

    #[test]
    fn a_walk_finds_the_water_the_surface_draws() {
        // The island above: the sea stands off its shore and on out to the
        // skirt's edge -- not on the island, and not in the basin.
        let mut w = water();
        w.bounds = [-20.0, -20.0, 20.0, 20.0];
        w.skirt = 100.0;
        let island = |x: f32, z: f32| {
            let r = (x * x + z * z).sqrt();
            Some(if r < 3.0 { 0.0 } else if r < 12.0 { 4.0 } else { 0.0 })
        };
        let sea = WaterFootprint::new(&w, island).unwrap();
        assert_eq!(sea.surface_at(16.0, 0.0), Some(2.0), "off the shore");
        assert_eq!(sea.surface_at(20.0, 20.0), Some(2.0), "on the grid's far corner");
        assert_eq!(sea.surface_at(60.0, 0.0), Some(2.0), "out on the skirt");
        assert_eq!(sea.surface_at(200.0, 0.0), None, "past the skirt's edge");
        assert_eq!(sea.surface_at(8.0, 0.0), None, "on the island");
        assert_eq!(sea.surface_at(0.5, 0.5), None, "in the basin the sea cannot reach");
        w.skirt = 0.0;
        let lake = WaterFootprint::new(&w, island).unwrap();
        assert_eq!(lake.surface_at(0.5, 0.5), Some(2.0), "a lake is the water in its box");
        assert_eq!(lake.surface_at(60.0, 0.0), None, "and ends at its edge");
    }

    #[test]
    fn a_big_body_gets_a_coarser_grid() {
        let mut w = water();
        w.bounds = [-200.0, -200.0, 200.0, 200.0];
        let (verts, _) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let side = WATER_GRID_MAX_STEPS as usize + 1;
        assert_eq!(verts.len(), side * side, "a 400 m sea must not be tessellated at half a metre");
    }

    #[test]
    fn a_water_def_survives_a_round_trip() {
        let w = water();
        let text = serde_json::to_string(&w).unwrap();
        assert_eq!(serde_json::from_str::<WaterDef>(&text).unwrap(), w);
    }
}
