use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaterDef {

    pub height: f32,

    pub bounds: [f32; 4],

    #[serde(default = "default_shallow")]
    pub shallow: [u8; 3],

    #[serde(default = "default_deep")]
    pub deep: [u8; 3],

    #[serde(default = "default_depth_scale")]
    pub depth_scale: f32,

    #[serde(default = "default_shore_fade")]
    pub shore_fade: f32,

    #[serde(default = "default_wind_speed")]
    pub wind_speed: f32,

    #[serde(default = "default_wind_direction")]
    pub wind_direction: f32,

    #[serde(default = "default_fetch")]
    pub fetch: f32,

    #[serde(default = "default_choppiness")]
    pub choppiness: f32,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub skirt: f32,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub swash: f32,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub swell: f32,
    #[serde(default = "default_gloss", skip_serializing_if = "is_one")]
    pub gloss: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mud: f32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wade_depth: Option<f32>,
}

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
fn default_gloss() -> f32 { 1.0 }
fn is_one(v: &f32) -> bool { *v == 1.0 }

impl WaterDef {

    pub fn extent(&self) -> (f32, f32, f32, f32) {
        let [a, b, c, d] = self.bounds;
        (a.min(c), b.min(d), a.max(c), b.max(d))
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (x0, z0, x1, z1) = self.extent();
        x >= x0 && x <= x1 && z >= z0 && z <= z1
    }

    pub fn depth_over(&self, ground_y: f32) -> f32 {
        (self.height - ground_y).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterVertex {
    pub position: [f32; 3],

    pub depth: f32,
}

pub const WATER_GRID_STEP: f32 = 0.5;

pub const WATER_GRID_MAX_STEPS: f32 = 200.0;

pub const OPEN_WATER_DEPTH: f32 = 100.0;

const SKIRT_RINGS: usize = 8;

struct Grid {
    step: f32,
    nx: usize,
    nz: usize,
    verts: Vec<WaterVertex>,
    wet: Vec<bool>,
}

fn grid(def: &WaterDef, ground: impl Fn(f32, f32) -> Option<f32>) -> Option<Grid> {
    let (x0, z0, x1, z1) = def.extent();
    let step = WATER_GRID_STEP.max((x1 - x0).max(z1 - z0) / WATER_GRID_MAX_STEPS);
    let nx = (((x1 - x0) / step).ceil() as usize).max(1) + 1;
    let nz = (((z1 - z0) / step).ceil() as usize).max(1) + 1;

    let reach = def.swash.max(0.0);

    let mut verts = Vec::with_capacity(nx * nz);
    let mut any_wet = def.skirt > 0.0;
    for iz in 0..nz {
        for ix in 0..nx {

            let x = if ix + 1 == nx { x1 } else { x0 + ix as f32 * step };
            let z = if iz + 1 == nz { z1 } else { z0 + iz as f32 * step };

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

            for tri in [[a, c, b], [b, c, d]] {
                if tri.iter().any(|&i| wet[i as usize]) {
                    indices.extend_from_slice(&tri);
                }
            }
        }
    }

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

    pub fn wade_depth(&self) -> f32 {
        self.wade
    }

    pub fn surface_at(&self, x: f32, z: f32) -> Option<f32> {
        let (x0, z0, x1, z1) = self.extent;
        if x < x0 || z < z0 || x > x1 || z > z1 {
            let out = (x0 - x).max(x - x1).max(z0 - z).max(z - z1);
            return (out <= self.skirt).then_some(self.height);
        }

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
            gloss: 1.0,
            mud: 0.0,
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
        let w = water();
        let (verts, indices) = build_surface(&w, |_, _| Some(0.0)).expect("all of it is wet");
        assert!(!indices.is_empty() && indices.len() % 3 == 0);
        for v in &verts {
            assert!((v.depth - 2.0).abs() < 1e-6, "flat bed at y=0 under y=2 is 2m deep");
            assert!((v.position[1] - 2.0).abs() < 1e-6, "every vertex sits ON the surface");
        }
    }

    #[test]
    fn a_bed_entirely_above_the_surface_produces_nothing() {

        let w = water();
        assert!(build_surface(&w, |_, _| Some(9.0)).is_none());
    }

    #[test]
    fn dry_quads_are_dropped_but_the_wet_ones_survive() {

        let w = water();
        let bowl = |x: f32, z: f32| Some(if x.abs() < 4.0 && z.abs() < 4.0 { -1.0 } else { 6.0 });
        let (verts, indices) = build_surface(&w, bowl).expect("the middle is wet");

        let full = build_surface(&w, |_, _| Some(0.0)).unwrap().1.len();
        assert!(
            indices.len() < full / 2,
            "a small pond in a big box must drop most of its quads: {} vs {full}",
            indices.len(),
        );

        for tri in indices.chunks_exact(3) {
            assert!(
                tri.iter().any(|&i| verts[i as usize].depth > 0.0),
                "a triangle was kept with all three corners on dry land",
            );
        }
    }

    #[test]
    fn water_running_off_the_heightfield_keeps_going() {

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

        let w = water();
        let (verts, _) = build_surface(&w, |x, _| Some(if x < 0.0 { 0.0 } else { 2.5 })).unwrap();
        let dry = verts.iter().find(|v| v.position[0] > 1.0).unwrap();
        assert!((dry.depth + 0.5).abs() < 1e-6, "ground 0.5 m above the surface is -0.5 m deep, got {}", dry.depth);
    }

    #[test]
    fn a_beach_the_waves_reach_keeps_its_surface() {

        let mut w = water();
        let beach = |x: f32, _: f32| Some(2.0 + 0.05 * x);
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
        let mut w = water();
        w.skirt = 5000.0;
        let (verts, indices) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let reach = verts.iter().map(|v| v.position[0].abs().max(v.position[2].abs())).fold(0.0f32, f32::max);
        assert!(reach >= 5000.0, "the skirt reaches {reach} m, not 5 km past the bounds");
        assert!(verts.iter().all(|v| v.position[1] == 2.0), "a skirt lies at the surface's own height");
        assert!(indices.iter().all(|&i| (i as usize) < verts.len()));

        let edge = verts.iter().filter(|v| v.position[0].abs() == 10.0 && v.position[2].abs() <= 10.0).count();
        let side = (20.0 / WATER_GRID_STEP) as usize + 1;
        assert_eq!(edge, 2 * side, "the grid's edge was duplicated for the skirt");

        assert!(verts.iter().filter(|v| v.position[0].abs() > 10.0).all(|v| v.depth == OPEN_WATER_DEPTH));
    }

    #[test]
    fn a_sea_does_not_fill_a_basin_ringed_by_land() {

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

        w.skirt = 0.0;
        let (verts, indices) = build_surface(&w, island).unwrap();
        assert!(indices.chunks_exact(3).any(|t| t.iter().all(|&i| {
            let [x, _, z] = verts[i as usize].position;
            (x * x + z * z).sqrt() < 3.0
        })));
    }

    #[test]
    fn a_walk_finds_the_water_the_surface_draws() {

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
