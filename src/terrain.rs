use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::physics::Aabb;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerrainKind {

    Heightfield {

        path: String,

        resolution: [u32; 2],

        size: [f32; 2],

        height_range: [f32; 2],
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainDef {

    #[serde(default)]
    pub origin: [f32; 3],

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splat: Option<SplatDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holes: Option<HoleDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub textures: Option<String>,

    #[serde(flatten)]
    pub kind: TerrainKind,
}

pub const DEFAULT_TEXTURE_DIR: &str = "textures/terrain";

impl TerrainDef {

    pub fn texture_dir(&self, game_dir: &std::path::Path) -> std::path::PathBuf {
        game_dir.join(self.textures.as_deref().unwrap_or(DEFAULT_TEXTURE_DIR))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplatDef {

    pub path: String,

    pub resolution: [u32; 2],

    #[serde(default = "default_splat_layers")]
    pub layers: u8,
}

fn default_splat_layers() -> u8 {
    SPLAT_LAYERS as u8
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainPatch {
    pub positions: Vec<Vec3>,

    pub indices: Vec<u32>,
}

pub trait TerrainSource: Send + Sync {

    fn bounds(&self) -> Aabb;

    fn height_at(&self, x: f32, z: f32) -> Option<f32>;

    fn patch(&self, region: Aabb, step: u32) -> TerrainPatch;
}

#[derive(Debug)]
pub struct Heightfield {
    samples: Vec<u16>,
    resolution: [u32; 2],
    size: [f32; 2],
    height_range: [f32; 2],
    origin: Vec3,

    holes: Option<HoleMask>,
}

impl Heightfield {

    pub fn new(
        samples: Vec<u16>,
        resolution: [u32; 2],
        size: [f32; 2],
        height_range: [f32; 2],
        origin: Vec3,
    ) -> Result<Self, String> {
        if resolution[0] < 2 || resolution[1] < 2 {
            return Err(format!(
                "heightfield resolution {resolution:?} must be at least 2x2"
            ));
        }
        let expected = resolution[0] as usize * resolution[1] as usize;
        if samples.len() != expected {
            return Err(format!(
                "heightfield has {} samples but resolution {resolution:?} needs {expected}",
                samples.len()
            ));
        }
        if !(size[0] > 0.0 && size[1] > 0.0) {
            return Err(format!("heightfield size {size:?} must be positive"));
        }
        Ok(Self { samples, resolution, size, height_range, origin, holes: None })
    }

    pub fn from_raw_le(
        bytes: &[u8],
        resolution: [u32; 2],
        size: [f32; 2],
        height_range: [f32; 2],
        origin: Vec3,
    ) -> Result<Self, String> {
        if bytes.len() % 2 != 0 {
            return Err(format!(
                "heightfield data is {} bytes, which is not a whole number of u16 samples",
                bytes.len()
            ));
        }
        let samples = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Self::new(samples, resolution, size, height_range, origin)
    }

    fn step_metres(&self) -> (f32, f32) {
        (
            self.size[0] / (self.resolution[0] - 1) as f32,
            self.size[1] / (self.resolution[1] - 1) as f32,
        )
    }

    fn sample(&self, ix: u32, iz: u32) -> f32 {
        let ix = ix.min(self.resolution[0] - 1);
        let iz = iz.min(self.resolution[1] - 1);
        let raw = self.samples[(iz * self.resolution[0] + ix) as usize] as f32 / u16::MAX as f32;
        let [lo, hi] = self.height_range;
        self.origin.y + lo + raw * (hi - lo)
    }

    fn world_position(&self, ix: u32, iz: u32) -> Vec3 {
        let (dx, dz) = self.step_metres();
        Vec3::new(
            self.origin.x + ix as f32 * dx,
            self.sample(ix, iz),
            self.origin.z + iz as f32 * dz,
        )
    }
}

impl TerrainSource for Heightfield {
    fn bounds(&self) -> Aabb {
        let [lo, hi] = self.height_range;
        Aabb {
            min: Vec3::new(self.origin.x, self.origin.y + lo, self.origin.z),
            max: Vec3::new(
                self.origin.x + self.size[0],
                self.origin.y + hi,
                self.origin.z + self.size[1],
            ),
        }
    }

    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let local_x = x - self.origin.x;
        let local_z = z - self.origin.z;
        if local_x < 0.0 || local_z < 0.0 || local_x > self.size[0] || local_z > self.size[1] {
            return None;
        }

        let (dx, dz) = self.step_metres();
        let fx = local_x / dx;
        let fz = local_z / dz;
        let ix = (fx.floor() as u32).min(self.resolution[0] - 2);
        let iz = (fz.floor() as u32).min(self.resolution[1] - 2);
        let tx = fx - ix as f32;
        let tz = fz - iz as f32;

        let h00 = self.sample(ix, iz);
        let h10 = self.sample(ix + 1, iz);
        let h01 = self.sample(ix, iz + 1);
        let h11 = self.sample(ix + 1, iz + 1);
        let top = h00 + (h10 - h00) * tx;
        let bottom = h01 + (h11 - h01) * tx;
        Some(top + (bottom - top) * tz)
    }

    fn patch(&self, region: Aabb, step: u32) -> TerrainPatch {
        let step = step.max(1);
        let (dx, dz) = self.step_metres();

        let to_index = |world: f32, origin: f32, spacing: f32, max: u32| -> u32 {
            let i = ((world - origin) / spacing).floor();
            if i < 0.0 { 0 } else { (i as u32).min(max) }
        };
        let x0 = to_index(region.min.x, self.origin.x, dx, self.resolution[0] - 1);
        let x1 = to_index(region.max.x, self.origin.x, dx, self.resolution[0] - 1);
        let z0 = to_index(region.min.z, self.origin.z, dz, self.resolution[1] - 1);
        let z1 = to_index(region.max.z, self.origin.z, dz, self.resolution[1] - 1);

        let mut patch = TerrainPatch::default();
        if x1 <= x0 || z1 <= z0 {
            return patch;
        }

        let xs: Vec<u32> = (x0..=x1).step_by(step as usize).chain(Some(x1)).collect();
        let zs: Vec<u32> = (z0..=z1).step_by(step as usize).chain(Some(z1)).collect();
        let xs = dedup_sorted(xs);
        let zs = dedup_sorted(zs);

        for &iz in &zs {
            for &ix in &xs {
                patch.positions.push(self.world_position(ix, iz));
            }
        }

        let stride = xs.len() as u32;
        for row in 0..(zs.len() as u32 - 1) {
            for col in 0..(stride - 1) {
                let a = row * stride + col;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;

                if self.holes.is_some() {
                    let cx = (self.world_position(xs[col as usize], iz_at(&zs, row)).x
                        + self.world_position(xs[col as usize + 1], iz_at(&zs, row + 1)).x)
                        * 0.5;
                    let cz = (self.world_position(xs[col as usize], iz_at(&zs, row)).z
                        + self.world_position(xs[col as usize + 1], iz_at(&zs, row + 1)).z)
                        * 0.5;
                    if self.is_hole_at(cx, cz) {
                        continue;
                    }
                }

                let ha = patch.positions[a as usize].y;
                let hb = patch.positions[b as usize].y;
                let hc = patch.positions[c as usize].y;
                let hd = patch.positions[d as usize].y;

                if (hb - hc).abs() <= (ha - hd).abs() {
                    patch.indices.extend_from_slice(&[a, c, b, b, c, d]);
                } else {
                    patch.indices.extend_from_slice(&[a, c, d, a, d, b]);
                }
            }
        }
        patch
    }
}

fn dedup_sorted(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v.dedup();
    v
}

pub fn load(def: &TerrainDef, game_dir: &std::path::Path) -> Result<Box<dyn TerrainSource>, String> {
    match &def.kind {
        TerrainKind::Heightfield { path, resolution, size, height_range } => {
            let full = game_dir.join(path);
            let bytes = std::fs::read(&full)
                .map_err(|e| format!("failed to read terrain {}: {e}", full.display()))?;
            let field = Heightfield::from_raw_le(
                &bytes,
                *resolution,
                *size,
                *height_range,
                Vec3::from(def.origin),
            )?;

            let holes = match &def.holes {
                Some(h) => {
                    let hole_path = game_dir.join(&h.path);
                    let hole_bytes = std::fs::read(&hole_path).map_err(|e| {
                        format!("failed to read hole mask {}: {e}", hole_path.display())
                    })?;
                    Some(HoleMask::from_bytes(&hole_bytes, h.resolution)?)
                }
                None => None,
            };
            Ok(Box::new(field.with_holes(holes)))
        }
    }
}

impl crate::scatter::Ground for Heightfield {
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        TerrainSource::height_at(self, x, z)
    }
}

pub const SPLAT_LAYERS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct SplatMap {
    weights: Vec<u8>,
    resolution: [u32; 2],
}

impl SplatMap {

    pub fn new(weights: Vec<u8>, resolution: [u32; 2]) -> Result<Self, String> {
        if resolution[0] < 1 || resolution[1] < 1 {
            return Err(format!("splat resolution {resolution:?} must be at least 1x1"));
        }
        let expected = resolution[0] as usize * resolution[1] as usize * SPLAT_LAYERS;
        if weights.len() != expected {
            return Err(format!(
                "splat map has {} bytes but resolution {resolution:?} needs {expected}",
                weights.len(),
            ));
        }
        Ok(Self { weights, resolution })
    }

    pub fn solid(resolution: [u32; 2], layer: usize) -> Self {
        let count = resolution[0] as usize * resolution[1] as usize;
        let mut weights = vec![0u8; count * SPLAT_LAYERS];
        for texel in 0..count {
            weights[texel * SPLAT_LAYERS + layer.min(SPLAT_LAYERS - 1)] = 255;
        }
        Self { weights, resolution }
    }

    pub fn resolution(&self) -> [u32; 2] {
        self.resolution
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.weights
    }

    pub fn at_index(&self, ix: u32, iz: u32) -> [u8; SPLAT_LAYERS] {
        let ix = ix.min(self.resolution[0] - 1) as usize;
        let iz = iz.min(self.resolution[1] - 1) as usize;
        let base = (iz * self.resolution[0] as usize + ix) * SPLAT_LAYERS;
        let mut out = [0u8; SPLAT_LAYERS];
        out.copy_from_slice(&self.weights[base..base + SPLAT_LAYERS]);
        out
    }

    pub fn sample_uv(&self, u: f32, v: f32) -> [u8; SPLAT_LAYERS] {
        let fx = (u.clamp(0.0, 1.0) * (self.resolution[0] - 1) as f32).round();
        let fz = (v.clamp(0.0, 1.0) * (self.resolution[1] - 1) as f32).round();
        self.at_index(fx as u32, fz as u32)
    }

    pub fn dominant_uv(&self, u: f32, v: f32) -> (usize, u8) {
        let w = self.sample_uv(u, v);
        let mut best = 0usize;
        for (i, weight) in w.iter().enumerate() {
            if *weight > w[best] {
                best = i;
            }
        }
        (best, w[best])
    }
}

pub fn load_splat(
    def: &TerrainDef,
    game_dir: &std::path::Path,
) -> Result<Option<SplatMap>, String> {
    let Some(splat) = &def.splat else {
        return Ok(None);
    };
    let full = game_dir.join(&splat.path);
    if splat.layers as usize != SPLAT_LAYERS {

        return Err(format!(
            "splat map {} declares {} layers but this build supports {SPLAT_LAYERS}",
            full.display(),
            splat.layers,
        ));
    }
    let bytes = std::fs::read(&full)
        .map_err(|e| format!("failed to read splat map {}: {e}", full.display()))?;
    Ok(Some(SplatMap::new(bytes, splat.resolution)?))
}

#[cfg(test)]
mod splat_tests {
    use super::*;

    #[test]
    fn solid_puts_all_weight_on_one_layer() {
        let m = SplatMap::solid([4, 3], 1);
        assert_eq!(m.resolution(), [4, 3]);
        assert_eq!(m.as_bytes().len(), 4 * 3 * SPLAT_LAYERS);
        assert_eq!(m.at_index(0, 0), [0, 255, 0, 0]);
        assert_eq!(m.at_index(3, 2), [0, 255, 0, 0]);
        assert_eq!(m.dominant_uv(0.5, 0.5), (1, 255));
    }

    #[test]
    fn a_byte_count_that_does_not_match_the_resolution_is_rejected() {
        assert!(SplatMap::new(vec![0; 4 * 4], [2, 2]).is_ok());
        let err = SplatMap::new(vec![0; 4 * 3], [2, 2]).unwrap_err();
        assert!(err.contains("12 bytes"), "error should name the actual count: {err}");
        assert!(err.contains("16"), "error should name the expected count: {err}");
    }

    #[test]
    fn indexing_is_row_major_with_x_fastest() {

        let mut bytes = vec![0u8; 3 * 2 * SPLAT_LAYERS];
        let last = (1 * 3 + 2) * SPLAT_LAYERS;
        bytes[last + 3] = 200;
        let m = SplatMap::new(bytes, [3, 2]).unwrap();
        assert_eq!(m.at_index(2, 1), [0, 0, 0, 200]);
        assert_eq!(m.at_index(1, 1), [0, 0, 0, 0]);
        assert_eq!(m.dominant_uv(1.0, 1.0), (3, 200));
    }

    #[test]
    fn indices_and_uvs_clamp_to_the_grid() {
        let m = SplatMap::solid([2, 2], 0);
        assert_eq!(m.at_index(99, 99), [255, 0, 0, 0]);
        assert_eq!(m.sample_uv(-5.0, 5.0), [255, 0, 0, 0]);
    }

    #[test]
    fn sampling_is_nearest_so_a_boundary_stays_a_boundary() {

        let bytes = vec![255, 0, 0, 0, 0, 255, 0, 0];
        let m = SplatMap::new(bytes, [2, 1]).unwrap();
        assert_eq!(m.dominant_uv(0.0, 0.0).0, 0);
        assert_eq!(m.dominant_uv(1.0, 0.0).0, 1);

        let (layer, weight) = m.dominant_uv(0.5, 0.0);
        assert!(layer == 0 || layer == 1, "midpoint resolved to layer {layer}");
        assert_eq!(weight, 255, "a nearest sample must return an authored value, not a mix");
    }

    #[test]
    fn a_scene_with_no_splat_block_loads_as_none() {
        let def = TerrainDef {
            origin: [0.0; 3],
            splat: None,
            holes: None,
            textures: None,
            kind: TerrainKind::Heightfield {
                path: "unused".into(),
                resolution: [2, 2],
                size: [1.0, 1.0],
                height_range: [0.0, 1.0],
            },
        };
        assert_eq!(load_splat(&def, std::path::Path::new("/nonexistent")), Ok(None));
    }
}

#[cfg(test)]
mod splat_layers_tests {
    use super::*;

    fn def_with(json: &str) -> TerrainDef {
        serde_json::from_str(json).expect("parse terrain def")
    }

    #[test]
    fn a_splat_block_without_a_layer_count_defaults_to_four() {
        let def = def_with(
            r#"{"origin":[0,0,0],"splat":{"path":"t.splat","resolution":[4,4]},
                "kind":"heightfield","path":"h.r16","resolution":[2,2],
                "size":[1,1],"height_range":[0,1]}"#,
        );
        assert_eq!(def.splat.as_ref().unwrap().layers, 4);
    }

    #[test]
    fn a_foreign_layer_count_is_refused_with_a_message_naming_both() {
        let def = def_with(
            r#"{"origin":[0,0,0],"splat":{"path":"t.splat","resolution":[4,4],"layers":8},
                "kind":"heightfield","path":"h.r16","resolution":[2,2],
                "size":[1,1],"height_range":[0,1]}"#,
        );
        let err = load_splat(&def, std::path::Path::new("/nonexistent")).unwrap_err();
        assert!(err.contains('8'), "error should name what the file claims: {err}");
        assert!(err.contains('4'), "error should name what this build supports: {err}");
    }

    #[test]
    fn the_layer_count_round_trips() {
        let def = def_with(
            r#"{"origin":[0,0,0],"splat":{"path":"t.splat","resolution":[8,8]},
                "kind":"heightfield","path":"h.r16","resolution":[2,2],
                "size":[1,1],"height_range":[0,1]}"#,
        );
        let text = serde_json::to_string(&def).expect("serialize");
        assert!(text.contains("\"layers\":4"), "layer count must be written: {text}");
        let back: TerrainDef = serde_json::from_str(&text).expect("round trip");
        assert_eq!(back.splat.unwrap().layers, 4);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoleMask {

    open: Vec<bool>,
    resolution: [u32; 2],
}

impl HoleMask {
    pub fn new(open: Vec<bool>, resolution: [u32; 2]) -> Result<Self, String> {
        if resolution[0] < 1 || resolution[1] < 1 {
            return Err(format!("hole resolution {resolution:?} must be at least 1x1"));
        }
        let expected = resolution[0] as usize * resolution[1] as usize;
        if open.len() != expected {
            return Err(format!(
                "hole mask has {} cells but resolution {resolution:?} needs {expected}",
                open.len(),
            ));
        }
        Ok(Self { open, resolution })
    }

    pub fn from_bytes(bytes: &[u8], resolution: [u32; 2]) -> Result<Self, String> {
        Self::new(bytes.iter().map(|b| *b != 0).collect(), resolution)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.open.iter().map(|o| u8::from(*o)).collect()
    }

    pub fn solid(resolution: [u32; 2]) -> Self {
        let count = resolution[0] as usize * resolution[1] as usize;
        Self { open: vec![false; count], resolution }
    }

    pub fn resolution(&self) -> [u32; 2] {
        self.resolution
    }

    pub fn any_open(&self) -> bool {
        self.open.iter().any(|o| *o)
    }

    pub fn is_open_uv(&self, u: f32, v: f32) -> bool {
        let ix = (u.clamp(0.0, 1.0) * self.resolution[0] as f32).floor() as u32;
        let iz = (v.clamp(0.0, 1.0) * self.resolution[1] as f32).floor() as u32;
        let ix = ix.min(self.resolution[0] - 1) as usize;
        let iz = iz.min(self.resolution[1] - 1) as usize;
        self.open[iz * self.resolution[0] as usize + ix]
    }

    pub fn set_open(&mut self, ix: u32, iz: u32, open: bool) {
        if ix >= self.resolution[0] || iz >= self.resolution[1] {
            return;
        }
        let at = iz as usize * self.resolution[0] as usize + ix as usize;
        self.open[at] = open;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleDef {

    pub path: String,

    pub resolution: [u32; 2],
}

fn iz_at(zs: &[u32], row: u32) -> u32 {
    zs[(row as usize).min(zs.len() - 1)]
}

impl Heightfield {

    pub fn with_holes(mut self, holes: Option<HoleMask>) -> Self {
        self.holes = holes;
        self
    }

    pub fn holes(&self) -> Option<&HoleMask> {
        self.holes.as_ref()
    }

    pub fn is_hole_at(&self, x: f32, z: f32) -> bool {
        let Some(mask) = &self.holes else { return false };
        let u = (x - self.origin.x) / self.size[0].max(1e-6);
        let v = (z - self.origin.z) / self.size[1].max(1e-6);
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return false;
        }
        mask.is_open_uv(u, v)
    }
}

#[cfg(test)]
mod hole_tests {
    use super::*;

    fn flat(res: [u32; 2], size: [f32; 2]) -> Heightfield {
        let count = res[0] as usize * res[1] as usize;
        Heightfield::new(vec![0u16; count], res, size, [0.0, 10.0], Vec3::ZERO)
            .expect("build heightfield")
    }

    #[test]
    fn a_solid_mask_changes_nothing() {
        let field = flat([5, 5], [40.0, 40.0]);
        let before = field.patch(field.bounds(), 1).indices.len();

        let holed = flat([5, 5], [40.0, 40.0]).with_holes(Some(HoleMask::solid([4, 4])));
        assert_eq!(holed.patch(holed.bounds(), 1).indices.len(), before);
    }

    #[test]
    fn an_open_cell_removes_its_two_triangles() {
        let mut mask = HoleMask::solid([4, 4]);
        mask.set_open(1, 1, true);
        let field = flat([5, 5], [40.0, 40.0]).with_holes(Some(mask));

        let solid = flat([5, 5], [40.0, 40.0]);
        let before = solid.patch(solid.bounds(), 1).indices.len();
        let after = field.patch(field.bounds(), 1).indices.len();

        assert_eq!(before - after, 6, "one open cell should drop exactly two triangles");
    }

    #[test]
    fn render_and_physics_see_the_same_holes() {
        let mut mask = HoleMask::solid([8, 8]);
        for ix in 2..5 {
            for iz in 3..6 {
                mask.set_open(ix, iz, true);
            }
        }
        let field = flat([9, 9], [80.0, 80.0]).with_holes(Some(mask));

        let physics = field.patch(field.bounds(), 1);
        let render = field.patch(field.bounds(), 1);
        assert_eq!(physics.indices, render.indices);
        assert_eq!(physics.positions.len(), render.positions.len());

        let solid = flat([9, 9], [80.0, 80.0]);
        assert!(
            physics.indices.len() < solid.patch(solid.bounds(), 1).indices.len(),
            "the mask should have removed geometry",
        );
    }

    #[test]
    fn a_fully_open_mask_produces_no_triangles() {
        let mut mask = HoleMask::solid([4, 4]);
        for ix in 0..4 {
            for iz in 0..4 {
                mask.set_open(ix, iz, true);
            }
        }
        let field = flat([5, 5], [40.0, 40.0]).with_holes(Some(mask));
        assert!(field.patch(field.bounds(), 1).indices.is_empty());
    }

    #[test]
    fn gameplay_can_ask_whether_a_point_is_over_a_hole() {
        let mut mask = HoleMask::solid([4, 4]);
        mask.set_open(0, 0, true);
        let field = flat([5, 5], [40.0, 40.0]).with_holes(Some(mask));

        assert!(field.is_hole_at(5.0, 5.0), "inside the open cell");
        assert!(!field.is_hole_at(35.0, 35.0), "far from it");
        assert!(!field.is_hole_at(-100.0, 0.0), "outside the terrain is not a hole");
    }

    #[test]
    fn a_cell_count_that_does_not_match_the_resolution_is_rejected() {
        assert!(HoleMask::from_bytes(&[0, 1, 0, 1], [2, 2]).is_ok());
        let err = HoleMask::from_bytes(&[0, 1, 0], [2, 2]).unwrap_err();
        assert!(err.contains('3') && err.contains('4'), "error should name both counts: {err}");
    }

    #[test]
    fn the_mask_round_trips_through_bytes() {
        let mut mask = HoleMask::solid([3, 2]);
        mask.set_open(2, 1, true);
        let back = HoleMask::from_bytes(&mask.to_bytes(), [3, 2]).expect("round trip");
        assert_eq!(back, mask);
        assert!(back.any_open());
        assert!(!HoleMask::solid([3, 2]).any_open());
    }
}

#[cfg(test)]
mod diagonal_tests {
    use super::*;

    fn one_quad(h: [u16; 4]) -> Heightfield {
        Heightfield::new(h.to_vec(), [2, 2], [10.0, 10.0], [0.0, 100.0], Vec3::ZERO)
            .expect("build quad")
    }

    fn tris(field: &Heightfield) -> Vec<[u32; 3]> {
        let p = field.patch(field.bounds(), 1);
        p.indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
    }

    fn signed_area_xz(p: &TerrainPatch, t: [u32; 3]) -> f32 {
        let (a, b, c) = (
            p.positions[t[0] as usize],
            p.positions[t[1] as usize],
            p.positions[t[2] as usize],
        );
        (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
    }

    #[test]
    fn a_single_raised_corner_keeps_the_opposite_half_flat() {

        let field = one_quad([0, 0, 0, 40000]);
        let patch = field.patch(field.bounds(), 1);
        let flat: Vec<_> = tris(&field)
            .into_iter()
            .filter(|t| t.iter().all(|i| patch.positions[*i as usize].y == 0.0))
            .collect();
        assert_eq!(flat.len(), 1, "exactly one triangle should stay entirely at zero");

        let field = one_quad([40000, 0, 0, 0]);
        let patch = field.patch(field.bounds(), 1);
        let flat: Vec<_> = tris(&field)
            .into_iter()
            .filter(|t| t.iter().all(|i| patch.positions[*i as usize].y == 0.0))
            .collect();
        assert_eq!(flat.len(), 1, "the mirrored case must also keep a flat half");
    }

    #[test]
    fn a_diagonal_ridge_is_split_along_its_length() {

        let field = one_quad([50000, 0, 20000, 50000]);
        let t = tris(&field);

        assert!(
            t.iter().all(|tri| tri.contains(&0) && tri.contains(&3)),
            "both triangles should share the a-d edge, got {t:?}",
        );
    }

    #[test]
    fn a_tie_is_deterministic() {
        let a = tris(&one_quad([1000, 1000, 1000, 1000]));
        let b = tris(&one_quad([1000, 1000, 1000, 1000]));
        assert_eq!(a, b);
        assert_eq!(a, tris(&one_quad([0, 0, 0, 0])), "any flat quad splits alike");
    }

    #[test]
    fn both_diagonals_wind_the_same_way() {
        for heights in [
            [0, 0, 0, 40000],
            [50000, 0, 20000, 50000],
            [0, 0, 0, 0],
            [10000, 40000, 5000, 60000],
        ] {
            let field = one_quad(heights);
            let patch = field.patch(field.bounds(), 1);
            let signs: Vec<f32> = patch
                .indices
                .chunks(3)
                .map(|c| signed_area_xz(&patch, [c[0], c[1], c[2]]).signum())
                .collect();
            assert!(
                signs.windows(2).all(|w| w[0] == w[1]),
                "mixed winding for heights {heights:?}: {signs:?}",
            );
        }
    }

    #[test]
    fn every_split_covers_the_quad_with_all_four_corners() {
        for heights in [[0, 0, 0, 40000], [50000, 0, 20000, 50000], [0, 0, 0, 0]] {
            let field = one_quad(heights);
            let patch = field.patch(field.bounds(), 1);
            assert_eq!(patch.indices.len(), 6, "two triangles for {heights:?}");
            let used: std::collections::HashSet<u32> = patch.indices.iter().copied().collect();
            assert_eq!(used.len(), 4, "all four corners used for {heights:?}");
        }
    }

    #[test]
    fn a_coarse_step_still_produces_whole_quads() {
        let mut samples = vec![0u16; 81];
        samples[40] = 60000;
        let field = Heightfield::new(samples, [9, 9], [80.0, 80.0], [0.0, 100.0], Vec3::ZERO)
            .expect("build field");
        for step in [1, 2, 4] {
            let patch = field.patch(field.bounds(), step);
            assert_eq!(patch.indices.len() % 6, 0, "whole quads at step {step}");
            assert!(!patch.indices.is_empty(), "geometry at step {step}");
        }
    }
}

#[cfg(test)]
mod diagonal_parity {
    use super::*;

    pub fn reference_field() -> Heightfield {
        let (nx, nz) = (9u32, 9u32);
        let mut samples = Vec::with_capacity((nx * nz) as usize);
        for iz in 0..nz {
            for ix in 0..nx {
                samples.push(((ix * 7919 + iz * 104729) % 65536) as u16);
            }
        }
        Heightfield::new(samples, [nx, nz], [80.0, 80.0], [0.0, 100.0], Vec3::ZERO)
            .expect("reference field")
    }

    pub fn index_checksum(indices: &[u32]) -> u32 {
        let mut h: u32 = 2166136261;
        for (i, v) in indices.iter().enumerate() {
            h ^= v.wrapping_mul(i as u32 + 1);
            h = h.wrapping_mul(16777619);
        }
        h
    }

    #[test]
    fn the_reference_triangulation_is_pinned() {
        let field = reference_field();
        let patch = field.patch(field.bounds(), 1);
        assert_eq!(patch.indices.len(), 8 * 8 * 6, "64 quads, two triangles each");
        println!("REFERENCE_INDEX_CHECKSUM = {}", index_checksum(&patch.indices));
        assert_eq!(index_checksum(&patch.indices), REFERENCE_INDEX_CHECKSUM);
    }

    #[test]
    fn the_reference_uses_both_diagonals() {
        let field = reference_field();
        let patch = field.patch(field.bounds(), 1);
        let mut bc = 0;
        let mut ad = 0;
        for quad in patch.indices.chunks(6) {

            if quad[3] == quad[0] { ad += 1 } else { bc += 1 }
        }
        assert!(bc > 0 && ad > 0, "expected both splits, got bc={bc} ad={ad}");
    }

    pub const REFERENCE_INDEX_CHECKSUM: u32 = 2993084145;
}
