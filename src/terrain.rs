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
    /// A normal the ground was AUTHORED with at a vertex, entry for entry with
    /// `positions`; zero where there is none and a consumer computes its own
    /// from the triangles. Empty when no vertex has one (every heightfield
    /// without a [`GroundPatch`]).
    pub normals: Vec<Vec3>,
}

pub trait TerrainSource: Send + Sync {

    fn bounds(&self) -> Aabb;

    fn height_at(&self, x: f32, z: f32) -> Option<f32>;

    fn patch(&self, region: Aabb, step: u32) -> TerrainPatch;

    /// The lowest the ground gets, world y. What physics takes as the level's
    /// floor (`PhysicsWorld::kill_height`): anything under it has fallen out
    /// of the world.
    ///
    /// The bottom of `bounds` unless a source knows better. That is never too
    /// high -- no ground lies under its own bounds -- and a source that knows
    /// its real lowest point should say so, since a floor set too low only
    /// makes a fallen prop wait longer to be put back.
    fn lowest_height(&self) -> f32 {
        self.bounds().min.y
    }

    /// This source as the regular grid of heights it is, if it is one.
    ///
    /// For a consumer that can hold a grid far more cheaply than its
    /// triangles: physics gives `PhysX` a height field, four bytes a sample,
    /// instead of cooking `patch(bounds, 1)` into a triangle mesh
    /// (`PhysicsWorld::spawn_scene_terrain`). `None` for any other kind of
    /// source, which is then asked for triangles as before.
    fn as_heightfield(&self) -> Option<&Heightfield> {
        None
    }
}

/// WHICH WAY A QUAD OF THE GROUND IS SPLIT, from its four corner heights
/// (`a` at the low x and z, `b` one step along x, `c` one step along z, `d`
/// opposite `a`): `true` for the diagonal b-c, `false` for a-d.
///
/// ONE rule, asked by the mesher ([`TerrainSource::patch`]) and by physics'
/// height field (`Heightfield::cell_splits_through_first_sample`), so a ray
/// lands on the triangle that is drawn. See the mesher for why the shorter
/// diagonal.
fn splits_b_to_c(ha: f32, hb: f32, hc: f32, hd: f32) -> bool {
    (hb - hc).abs() <= (ha - hd).abs()
}

#[derive(Debug)]
pub struct Heightfield {
    samples: Vec<u16>,
    resolution: [u32; 2],
    size: [f32; 2],
    height_range: [f32; 2],
    origin: Vec3,

    holes: Option<HoleMask>,
    /// Ground a cave bake put back inside the cells it holed. On the source
    /// for the reason the holes are: see [`GroundPatch`].
    ground_patch: Option<GroundPatch>,
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
        Ok(Self { samples, resolution, size, height_range, origin, holes: None, ground_patch: None })
    }

    pub fn from_raw_le(
        bytes: &[u8],
        resolution: [u32; 2],
        size: [f32; 2],
        height_range: [f32; 2],
        origin: Vec3,
    ) -> Result<Self, String> {
        if !bytes.len().is_multiple_of(2) {
            return Err(format!(
                "heightfield data is {} bytes, which is not a whole number of u16 samples",
                bytes.len()
            ));
        }
        let samples = bytes
            .as_chunks::<2>().0.iter()
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
        let raw = f32::from(self.samples[(iz * self.resolution[0] + ix) as usize]) / f32::from(u16::MAX);
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

    /// The lowest sample, and the lowest vertex of the ground the caves put
    /// back: one pass over the samples, asked once a level load.
    fn lowest_height(&self) -> f32 {
        let (mut least, mut most) = (u16::MAX, 0u16);
        for &raw in &self.samples {
            least = least.min(raw);
            most = most.max(raw);
        }
        let [lo, hi] = self.height_range;
        let at = |raw: u16| self.origin.y + lo + (f32::from(raw) / f32::from(u16::MAX)) * (hi - lo);
        let ground = at(least).min(at(most));
        self.ground_patch.iter().flat_map(|p| p.positions.iter()).fold(ground, |low, p| low.min(p.y))
    }

    fn as_heightfield(&self) -> Option<&Heightfield> {
        Some(self)
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
        let (x0, x1, z0, z1) = self.sample_span(region);

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

                if self.quad_is_open(xs[col as usize], xs[col as usize + 1], iz_at(&zs, row), iz_at(&zs, row + 1)) {
                    continue;
                }

                let ha = patch.positions[a as usize].y;
                let hb = patch.positions[b as usize].y;
                let hc = patch.positions[c as usize].y;
                let hd = patch.positions[d as usize].y;

                if splits_b_to_c(ha, hb, hc, hd) {
                    patch.indices.extend_from_slice(&[a, c, b, b, c, d]);
                } else {
                    patch.indices.extend_from_slice(&[a, c, d, a, d, b]);
                }
            }
        }

        if let Some(extra) = &self.ground_patch {
            let mut mapped: Vec<Option<u32>> = vec![None; extra.positions.len()];
            for tri in extra.triangles_in(region) {
                for &i in tri {
                    let i = i as usize;
                    let at = if let Some(at) = mapped[i] { at } else {
                        let p = extra.positions[i];
                        let on_grid = self.sample_index_at(p).and_then(|(ix, iz)| {
                            let col = xs.binary_search(&ix).ok()?;
                            let row = zs.binary_search(&iz).ok()?;
                            Some(row as u32 * stride + col as u32)
                        });
                        let at = on_grid.unwrap_or_else(|| {
                            patch.positions.push(p);
                            (patch.positions.len() - 1) as u32
                        });
                        let n = extra.normals.get(i).copied().unwrap_or(Vec3::ZERO);
                        if on_grid.is_none() && n != Vec3::ZERO {
                            patch.normals.resize(patch.positions.len(), Vec3::ZERO);
                            patch.normals[at as usize] = n;
                        }
                        mapped[i] = Some(at);
                        at
                    };
                    patch.indices.push(at);
                }
            }
            if !patch.normals.is_empty() {
                patch.normals.resize(patch.positions.len(), Vec3::ZERO);
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
            let ground_patch = match &def.holes {
                Some(h) => {
                    let patch_path = game_dir.join(ground_patch_path(&h.path));
                    match std::fs::read(&patch_path) {
                        Ok(bytes) => Some(
                            GroundPatch::from_bytes(&bytes)
                                .map_err(|e| format!("terrain patch {}: {e}", patch_path.display()))?,
                        ),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                        Err(e) => return Err(format!("failed to read terrain patch {}: {e}", patch_path.display())),
                    }
                }
                None => None,
            };
            Ok(Box::new(field.with_holes(holes).with_ground_patch(ground_patch)))
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

/// How many texels beyond a sheer face its bare rock takes to fade back to
/// what was painted: 0.86 m on the test level's 0.21 m texels.
pub const SHEER_SKIRT_TEXELS: u32 = 4;

/// The sheer share of each texel (0..1, from the faces' own footprints) as a
/// byte, spread outward as a cone: full where it was, then falling in a
/// straight line to nothing `SHEER_SKIRT_TEXELS` away. Two sweeps of a 3-4
/// chamfer, greatest value wins -- whole numbers only, so it comes out the
/// same in any language.
fn feather_sheer(sheer: &[f32], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as i32, h as i32);
    let mut m: Vec<i32> = sheer.iter().map(|&s| (s.clamp(0.0, 1.0) * 255.0).round() as i32).collect();
    let step = 255 / SHEER_SKIRT_TEXELS as i32;
    let diagonal = step * 4 / 3;
    let at = |x: i32, z: i32| (z * w + x) as usize;
    let sweep = |m: &mut Vec<i32>, forward: bool| {
        let rows: Box<dyn Iterator<Item = i32>> = if forward { Box::new(0..h) } else { Box::new((0..h).rev()) };
        for z in rows {
            let cols: Box<dyn Iterator<Item = i32>> = if forward { Box::new(0..w) } else { Box::new((0..w).rev()) };
            for x in cols {
                let d = if forward { -1 } else { 1 };
                let mut best = m[at(x, z)];
                for (nx, nz, cost) in [(x + d, z, step), (x, z + d, step), (x + d, z + d, diagonal), (x - d, z + d, diagonal)] {
                    if nx >= 0 && nz >= 0 && nx < w && nz < h {
                        best = best.max(m[at(nx, nz)] - cost);
                    }
                }
                m[at(x, z)] = best;
            }
        }
    };
    sweep(&mut m, true);
    sweep(&mut m, false);
    m.into_iter().map(|v| v.clamp(0, 255) as u8).collect()
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

    /// SOIL DOES NOT HOLD ON A CLIFF: the soil layers' weight (ground, layer
    /// 0, and sediment, layer 3) moved to rock (layer 1) wherever a texel's
    /// footprint touches drawn ground steeper than about 60 degrees, fully
    /// past 70. The `high` layer is left as painted. Returns how many texels
    /// changed.
    ///
    /// Why the map has to say it: the map is read straight down, so a
    /// near-vertical face of the heightfield -- a cliff step of metres inside
    /// one 0.43 m cell -- has a plan footprint of a few centimetres, and takes
    /// whatever its one or two texels hold up its whole height. Where the
    /// grass at a cliff's foot or rim shared those texels, the face was drawn
    /// as grass in vertical stripes: the dark slits on the ridge by the cave's
    /// back exit (2026-10-08). No paint could separate the foot from the face
    /// in a top-down map, and grass does not grow on a 70-degree face, so the
    /// texels a sheer face reads become bare rock -- the face and a bilinear
    /// texel round it -- fading back to the paint over `SHEER_SKIRT_TEXELS`
    /// at its foot and rim, where a cliff is bare or scree too.
    /// The cave does the same on its walls (`rockOnSheerFaces`, editor).
    pub fn bare_rock_on_sheer_ground(&mut self, ground: &dyn TerrainSource) -> usize {
        const ROCK: usize = 1;
        const SOIL: [usize; 2] = [0, 3];
        const HOLDS: f32 = 0.5;
        const BARE: f32 = 0.34;
        let bounds = ground.bounds();
        let extent = bounds.max - bounds.min;
        if !(extent.x > 0.0 && extent.z > 0.0) {
            return 0;
        }
        let [w, h] = self.resolution;
        let mut sheer = vec![0.0f32; (w * h) as usize];
        let patch = ground.patch(bounds, 1);
        for tri in patch.indices.as_chunks::<3>().0 {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| patch.positions[i as usize]);
            let n = (b - a).cross(c - a).normalize_or_zero();
            let ny = n.y.abs();
            if ny >= HOLDS || n == Vec3::ZERO {
                continue;
            }
            let t = ((HOLDS - ny) / (HOLDS - BARE)).clamp(0.0, 1.0);
            let s = t * t * (3.0 - 2.0 * t);
            let snap = |t: f32| (t * 1024.0).round() / 1024.0;
            let tx = |x: f32| snap((x - bounds.min.x) / extent.x * w as f32);
            let tz = |z: f32| snap((z - bounds.min.z) / extent.z * h as f32);
            let (x0, x1) = (tx(a.x.min(b.x).min(c.x)), tx(a.x.max(b.x).max(c.x)));
            let (z0, z1) = (tz(a.z.min(b.z).min(c.z)), tz(a.z.max(b.z).max(c.z)));
            let span = |lo: f32, hi: f32, n: u32| {
                let first = (lo.floor() as i64 - 1).max(0) as u32;
                let last = ((hi.floor() as i64 + 1).max(0) as u32).min(n - 1);
                first..=last
            };
            for iz in span(z0, z1, h) {
                for ix in span(x0, x1, w) {
                    let k = (iz * w + ix) as usize;
                    sheer[k] = sheer[k].max(s);
                }
            }
        }
        let share = feather_sheer(&sheer, w, h);
        let mut changed = 0;
        for (k, &m) in share.iter().enumerate() {
            if m == 0 {
                continue;
            }
            let texel = &mut self.weights[k * SPLAT_LAYERS..(k + 1) * SPLAT_LAYERS];
            let before = [texel[0], texel[1], texel[2], texel[3]];
            let mut moved = 0u32;
            for layer in SOIL {
                let take = (u32::from(texel[layer]) * u32::from(m) + 127) / 255;
                texel[layer] -= take as u8;
                moved += take;
            }
            texel[ROCK] = (u32::from(texel[ROCK]) + moved).min(255) as u8;
            if texel != before {
                changed += 1;
            }
        }
        changed
    }

    /// The layer with the most weight at a normalised position, and its weight.
    ///
    /// The query gameplay actually wants: "what am I standing on".
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
        let last = (3 + 2) * SPLAT_LAYERS;
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

    /// Grass painted over a cliff step of metres inside one cell is read
    /// straight down, so its face took the grass up its whole height. The
    /// soil layers on sheer ground go to rock; level ground and the `high`
    /// layer keep what was painted.
    #[test]
    fn soil_painted_on_a_cliff_becomes_bare_rock_and_level_ground_keeps_its_paint() {
        let res = 9u32;
        let samples: Vec<u16> =
            (0..res * res).map(|i| if (i % res) as f32 * 0.5 >= 2.25 { u16::MAX } else { 0 }).collect();
        let field = Heightfield::new(samples, [res, res], [4.0, 4.0], [0.0, 3.0], Vec3::ZERO).unwrap();
        let texels = 16u32;
        let mut map = SplatMap::new([200u8, 0, 30, 25].repeat((texels * texels) as usize), [texels, texels]).unwrap();
        let changed = map.bare_rock_on_sheer_ground(&field);
        assert!(changed > 0);
        let on_step = map.at_index(9, 8);
        assert_eq!(on_step, [0, 225, 30, 0], "the cliff's soil should be rock, its `high` kept");
        let level = map.at_index(1, 8);
        assert_eq!(level, [200, 0, 30, 25], "level ground keeps its paint");
        let fnv = map.as_bytes().iter().fold(0x811c9dc5u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(0x01000193));
        assert_eq!(fnv, SHEER_FIXTURE_CHECKSUM, "the engine and the editor bare different texels");
        let flat = Heightfield::new(vec![0; 81], [res, res], [4.0, 4.0], [0.0, 3.0], Vec3::ZERO).unwrap();
        let mut untouched = SplatMap::new([200u8, 0, 30, 25].repeat((texels * texels) as usize), [texels, texels]).unwrap();
        assert_eq!(untouched.bare_rock_on_sheer_ground(&flat), 0);
    }

    const SHEER_FIXTURE_CHECKSUM: u32 = 1847300485;

    /// STEEP SHAPES, PAINTED ACROSS (2026-10-12): a 60-degree bank, an
    /// 85-degree face, a vertical step, a narrow ridge and a pit, each laid at
    /// an angle to the grid on the level's own 0.43 m cells, with all four
    /// layers painted over them in stripes that cross every face.
    ///
    /// `drawn` lays a vertical edge the way the editor's flat-topped brush
    /// now does -- each sample holding the share of it the shape covers, the
    /// edge 1.5 cells wide (`RIM_CELLS`, heightfield.js) -- and `!drawn` lays
    /// it on whole samples, as the brush did before and a generator that
    /// samples a shape at points still does.
    mod steep_shapes {
        use super::*;
        use glam::Vec2;

        pub const CELL: f32 = 0.43;
        const SAMPLES: u32 = 65;
        const SIZE: f32 = CELL * (SAMPLES - 1) as f32;
        const TEXELS: u32 = 128;
        /// Across every shape's faces, at a slant: no face runs along the grid.
        const ACROSS: Vec2 = Vec2::new(-0.574, 0.819);

        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum Shape {
            Bank60,
            Face85,
            Step,
            Ridge,
            Pit,
        }
        pub const ALL: [Shape; 5] = [Shape::Bank60, Shape::Face85, Shape::Step, Shape::Ridge, Shape::Pit];

        /// How much of a sample a vertical edge `inside` metres behind it covers.
        fn cover(inside: f32, drawn: bool) -> f32 {
            if drawn {
                (0.5 + inside / (1.5 * CELL)).clamp(0.0, 1.0)
            } else if inside >= 0.0 {
                1.0
            } else {
                0.0
            }
        }

        pub fn height(shape: Shape, drawn: bool, x: f32, z: f32) -> f32 {
            let p = Vec2::new(x, z) - Vec2::splat(SIZE * 0.5);
            let across = p.dot(ACROSS);
            let ramp = |deg: f32| 6.0 * (across / (6.0 / deg.to_radians().tan()) + 0.5).clamp(0.0, 1.0);
            match shape {
                Shape::Bank60 => ramp(60.0),
                Shape::Face85 => ramp(85.0),
                Shape::Step => 6.0 * cover(across, drawn),
                Shape::Ridge => 4.0 * cover(0.8 - across.abs(), drawn),
                Shape::Pit => -4.0 * cover(3.5 - p.length(), drawn),
            }
        }

        pub fn field(shape: Shape, drawn: bool) -> Heightfield {
            let range = [-10.0f32, 30.0];
            let samples = (0..SAMPLES * SAMPLES)
                .map(|i| {
                    let y = height(shape, drawn, (i % SAMPLES) as f32 * CELL, (i / SAMPLES) as f32 * CELL);
                    ((y - range[0]) / (range[1] - range[0]) * f32::from(u16::MAX)).round() as u16
                })
                .collect();
            Heightfield::new(samples, [SAMPLES, SAMPLES], [SIZE, SIZE], range, Vec3::ZERO).unwrap()
        }

        /// Stripes a metre and a half wide, 30 degrees off the grid: ground,
        /// sediment, high ground, rock. They cross every face of every shape.
        pub fn painted() -> SplatMap {
            let order = [0usize, 3, 2, 1];
            let mut bytes = vec![0u8; (TEXELS * TEXELS) as usize * SPLAT_LAYERS];
            for t in 0..TEXELS * TEXELS {
                let x = ((t % TEXELS) as f32 + 0.5) / TEXELS as f32 * SIZE;
                let z = ((t / TEXELS) as f32 + 0.5) / TEXELS as f32 * SIZE;
                let band = ((x * 0.866 + z * 0.5) / 1.5).floor() as usize % 4;
                bytes[t as usize * SPLAT_LAYERS + order[band]] = 255;
            }
            SplatMap::new(bytes, [TEXELS, TEXELS]).unwrap()
        }

        /// Each drawn triangle: its corners, its unit normal, its thinnest
        /// corner in degrees.
        pub fn triangles(field: &Heightfield) -> Vec<([Vec3; 3], Vec3, f32)> {
            let patch = field.patch(field.bounds(), 1);
            patch
                .indices
                .as_chunks::<3>().0.iter()
                .map(|t| {
                    let c = [t[0], t[1], t[2]].map(|i| patch.positions[i as usize]);
                    let n = (c[1] - c[0]).cross(c[2] - c[0]);
                    let corner = |a: Vec3, b: Vec3, d: Vec3| (b - a).angle_between(d - a).to_degrees();
                    let thin = corner(c[0], c[1], c[2]).min(corner(c[1], c[2], c[0])).min(corner(c[2], c[0], c[1]));
                    (c, n.normalize_or_zero(), thin)
                })
                .collect()
        }

        /// The texels a triangle's plan footprint touches.
        pub fn texels_under(c: &[Vec3; 3]) -> impl Iterator<Item = (u32, u32)> {
            let to = |v: f32| ((((v / SIZE * TEXELS as f32) * 1024.0).round() / 1024.0).floor().max(0.0) as u32).min(TEXELS - 1);
            let (x0, x1) = (to(c[0].x.min(c[1].x).min(c[2].x)), to(c[0].x.max(c[1].x).max(c[2].x)));
            let (z0, z1) = (to(c[0].z.min(c[1].z).min(c[2].z)), to(c[0].z.max(c[1].z).max(c[2].z)));
            (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| (x, z)))
        }

        /// GEOMETRY. Whatever is drawn, the mesh has no triangle of no area
        /// and none folded under, and no triangle is thinner than the slope
        /// it lies on makes it. A cell's triangle on a face is one cell wide
        /// and the face's rise long, so its thinnest corner is `atan(cos a)`
        /// at worst on ground of slope `a` -- 27 degrees at 60, 19 at 70, 10
        /// at 80, 5 at 85 -- and the shorter-diagonal cut does no worse than
        /// that anywhere. So what keeps slivers out is how steep a drawn edge
        /// is allowed to stand: across 1.5 cells a 6 m step is 84 degrees and
        /// its thinnest corner 13; on whole samples it is 87 and 4 (`census`).
        #[test]
        fn no_shape_meshes_with_a_folded_or_needlessly_thin_triangle() {
            for shape in ALL {
                for drawn in [false, true] {
                    let field = field(shape, drawn);
                    for (c, n, thin) in triangles(&field) {
                        let area = (c[1] - c[0]).cross(c[2] - c[0]).length() * 0.5;
                        assert!(area > 1e-4 && n.y > 0.0, "{shape:?} drawn={drawn}: a triangle with no area or folded under: {c:?}");
                        let slope_makes = n.y.atan().to_degrees();
                        assert!(
                            thin >= 0.95 * slope_makes,
                            "{shape:?} drawn={drawn}: a {thin:.1} degree corner on ground whose slope allows {slope_makes:.1}: {c:?}"
                        );
                    }
                }
            }
        }

        /// SKYLINE AND FACE. A straight face drawn at a slant must face one
        /// way. Laid on whole samples it is a staircase: steps facing along
        /// the grid's two axes in turn, teeth on the skyline and slits down
        /// the face. Drawn with its edge across the grid it faces where it
        /// was drawn to, to within a few degrees, all along.
        #[test]
        fn a_slanted_face_faces_one_way_when_its_edge_is_drawn_across_the_grid() {
            let astray = |shape: Shape, drawn: bool| {
                let (mut off, mut all) = (0.0f32, 0.0f32);
                for (c, n, _) in triangles(&field(shape, drawn)) {
                    if n.y >= 0.5 {
                        continue;
                    }
                    let area = (c[1] - c[0]).cross(c[2] - c[0]).length() * 0.5;
                    let plan = Vec2::new(n.x, n.z).normalize_or_zero();
                    let turn = plan.dot(-ACROSS).abs().clamp(0.0, 1.0).acos().to_degrees();
                    all += area;
                    if turn > 20.0 {
                        off += area;
                    }
                }
                off / all.max(1e-6)
            };
            for shape in [Shape::Face85, Shape::Step, Shape::Ridge] {
                assert!(astray(shape, true) < 0.06, "{shape:?}: {:.0}% of the drawn face is turned away", astray(shape, true) * 100.0);
            }
            for shape in [Shape::Step, Shape::Ridge] {
                assert!(astray(shape, false) > 0.15, "{shape:?}: the staircase this guards against: {:.2}", astray(shape, false));
            }
            for (shape, rise) in [(Shape::Step, 6.0f32), (Shape::Ridge, 4.0), (Shape::Pit, 4.0)] {
                let allowed = (rise / (1.5 * CELL)).atan();
                for (c, n, thin) in triangles(&field(shape, true)).into_iter().filter(|t| t.1.y < 0.5) {
                    assert!(n.y.acos() <= allowed + 0.01, "{shape:?}: a drawn face at {:.1} degrees: {c:?}", n.y.acos().to_degrees());
                    assert!(thin >= 0.95 * allowed.cos().atan().to_degrees(), "{shape:?}: a {thin:.1} degree sliver on a drawn face");
                }
            }
        }

        /// The numbers behind the three tests, for a write-up:
        /// `cargo test steep_shapes::census -- --ignored --nocapture`.
        #[test]
        #[ignore]
        fn census() {
            for shape in ALL {
                for drawn in [false, true] {
                    let field = field(shape, drawn);
                    let (mut steep, mut slivers, mut thinnest, mut ratio, mut steepest) = (0, 0, 90.0f32, 9.0f32, 0.0f32);
                    let (mut off, mut all) = (0.0f32, 0.0f32);
                    for (c, n, thin) in triangles(&field) {
                        ratio = ratio.min(thin / n.y.atan().to_degrees());
                        if n.y >= 0.5 {
                            continue;
                        }
                        steep += 1;
                        slivers += u32::from(thin < 8.0);
                        thinnest = thinnest.min(thin);
                        steepest = steepest.max(n.y.acos().to_degrees());
                        let area = (c[1] - c[0]).cross(c[2] - c[0]).length() * 0.5;
                        let turn = Vec2::new(n.x, n.z).normalize_or_zero().dot(-ACROSS).abs().clamp(0.0, 1.0).acos().to_degrees();
                        all += area;
                        off += if turn > 20.0 { area } else { 0.0 };
                    }
                    let mut bared = painted();
                    let changed = bared.bare_rock_on_sheer_ground(&field);
                    println!(
                        "{shape:?} drawn={drawn}: {steep} triangles past 60 deg, steepest {steepest:.1}, {slivers} under 8 deg, thinnest {thinnest:.1}, thinnest/slope-allows {ratio:.2}, turned>20deg {:.0}%, texels bared {changed}",
                        100.0 * off / all.max(1e-6)
                    );
                }
            }
        }

        /// PAINT. The map is read straight down, so a texel is spread up a
        /// face by 1/cos of its slope: twice at 60 degrees, three times at
        /// 70, six at 80, and a vertical face takes one texel up its whole
        /// height. Soil layers painted across a face that steep would be
        /// drawn as stripes metres tall, so they go to rock there; ground no
        /// steeper than 60 degrees keeps exactly what was painted, and the
        /// `high` and rock stripes are as painted everywhere.
        #[test]
        fn paint_crossing_a_sheer_face_is_never_stretched_up_it() {
            for shape in ALL {
                for drawn in [false, true] {
                    let field = field(shape, drawn);
                    let painted = painted();
                    let mut bared = painted.clone();
                    bared.bare_rock_on_sheer_ground(&field);
                    let tris = triangles(&field);
                    let mut near_steep = vec![false; (TEXELS * TEXELS) as usize];
                    for (c, n, _) in &tris {
                        if n.y < 0.5 {
                            for (x, z) in texels_under(c) {
                                let reach = 1 + SHEER_SKIRT_TEXELS as i32;
                                for dz in -reach..=reach {
                                    for dx in -reach..=reach {
                                        let (x, z) = (x as i32 + dx, z as i32 + dz);
                                        if x >= 0 && z >= 0 && (x as u32) < TEXELS && (z as u32) < TEXELS {
                                            near_steep[(z as u32 * TEXELS + x as u32) as usize] = true;
                                        }
                                    }
                                }
                            }
                        }
                        if n.y < 0.34 {
                            for (x, z) in texels_under(c) {
                                let w = bared.at_index(x, z);
                                assert_eq!((w[0], w[3]), (0, 0), "{shape:?} drawn={drawn}: soil left on a {:.0} degree face at texel ({x}, {z})", n.y.acos().to_degrees());
                            }
                        }
                    }
                    for z in 0..TEXELS {
                        for x in 0..TEXELS {
                            let (was, now) = (painted.at_index(x, z), bared.at_index(x, z));
                            assert_eq!(now.iter().map(|&b| u32::from(b)).sum::<u32>(), 255, "weights no longer sum to one");
                            assert_eq!(now[2], was[2], "the high layer is as painted");
                            assert!(now[1] >= was[1], "rock is never taken away");
                            if !near_steep[(z * TEXELS + x) as usize] {
                                assert_eq!(now, was, "{shape:?} drawn={drawn}: paint changed at ({x}, {z}), away from any face past 60 degrees");
                            }
                        }
                    }
                }
            }
            for shape in ALL {
                let field = field(shape, true);
                let mut bared = SplatMap::new([255u8, 0, 0, 0].repeat((TEXELS * TEXELS) as usize), [TEXELS, TEXELS]).unwrap();
                bared.bare_rock_on_sheer_ground(&field);
                for z in 0..TEXELS - 1 {
                    for x in 0..TEXELS - 1 {
                        let here = i32::from(bared.at_index(x, z)[0]);
                        for there in [i32::from(bared.at_index(x + 1, z)[0]), i32::from(bared.at_index(x, z + 1)[0])] {
                            assert!((here - there).abs() <= 85, "{shape:?}: the soil steps from {here} to {there} at texel ({x}, {z})");
                        }
                    }
                }
            }
            let (field, painted) = (field(Shape::Bank60, true), painted());
            let mut bared = painted.clone();
            bared.bare_rock_on_sheer_ground(&field);
            let soil = |m: &SplatMap| m.as_bytes().as_chunks::<SPLAT_LAYERS>().0.iter().map(|t| u64::from(t[0]) + u64::from(t[3])).sum::<u64>();
            assert!(soil(&bared) as f64 >= 0.97 * soil(&painted) as f64, "a 60 degree bank lost its paint: {} of {}", soil(&bared), soil(&painted));
        }
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

/// Where a scene's terrain patch lives, given its hole mask's path: beside it.
pub fn ground_patch_path(holes_path: &str) -> std::path::PathBuf {
    std::path::Path::new(holes_path).with_extension("patch")
}

/// GROUND A CAVE PUT BACK inside the terrain cells it took.
///
/// A hole in a heightfield is whole cells, so a mouth cut by a sculpt leaves a
/// stepped outline with nothing in it but the cave's own rock. The cave bake
/// (`scene_editor_web/frontend/src/lib/sdfBake.js`) therefore OWNS the cells
/// round a mouth: it opens them in the mask and hands back, here, the part of
/// their ground the sculpt did not touch -- meshed with the cave, so it meets
/// the carved rock along the carve's own line, and ending on the cells'
/// outline at the heightfield's own samples, bit for bit.
///
/// It is TERRAIN: `patch` appends it to the grid's triangles, so it is drawn
/// by the terrain's shader with the terrain's maps and cooked into the
/// terrain's collider. Nothing joins two materials at the stepped outline;
/// the join with the cave's rock is at the carve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroundPatch {
    /// World positions.
    pub positions: Vec<Vec3>,
    /// A normal per position: the sculpt's gradient, or ZERO at a vertex that
    /// is one of the heightfield's samples (its normal is the terrain's).
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl GroundPatch {
    const MAGIC: &'static [u8; 4] = b"SSTP";
    const VERSION: u32 = 1;

    /// Every section of a `.patch` file as one patch: a section a cave, each
    /// `name, vertex count, index count, positions, normals, indices`
    /// (`sdfBakeGlb.js::serializeTerrainPatch`).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let u32_at = |o: usize| -> Result<u32, String> {
            bytes.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| "truncated".to_string())
        };
        if bytes.len() < 12 || &bytes[0..4] != Self::MAGIC {
            return Err("not a terrain patch (bad magic)".into());
        }
        if u32_at(4)? != Self::VERSION {
            return Err(format!("version {} is not {}", u32_at(4)?, Self::VERSION));
        }
        let sections = u32_at(8)?;
        let mut out = Self::default();
        let mut o = 12usize;
        for _ in 0..sections {
            let name = bytes.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or("truncated")?;
            o += 2 + name;
            o += (4 - o % 4) % 4;
            let (nv, ni) = (u32_at(o)? as usize, u32_at(o + 4)? as usize);
            o += 8;
            let need = nv.checked_mul(24).and_then(|v| ni.checked_mul(4).map(|i| v + i)).ok_or("section too large")?;
            if bytes.len() < o + need {
                return Err("truncated".into());
            }
            let f32_at = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
            let vec_at = |o: usize| Vec3::new(f32_at(o), f32_at(o + 4), f32_at(o + 8));
            let base = out.positions.len() as u32;
            out.positions.extend((0..nv).map(|i| vec_at(o + i * 12)));
            out.normals.extend((0..nv).map(|i| vec_at(o + nv * 12 + i * 12)));
            o += nv * 24;
            for i in 0..ni {
                let idx = u32_at(o + i * 4)?;
                if idx as usize >= nv {
                    return Err(format!("index {idx} past {nv} vertices"));
                }
                out.indices.push(base + idx);
            }
            o += ni * 4;
        }
        out.indices.truncate(out.indices.len() / 3 * 3);
        if out.positions.iter().chain(&out.normals).any(|p| !p.is_finite()) {
            return Err("a position or normal is not finite".into());
        }
        Ok(out)
    }

    /// The triangles of the patch a region takes, three indices each: those
    /// whose centre is over it. ONE rule for the mesher, which appends them
    /// to the grid's triangles ([`TerrainSource::patch`]), and for physics,
    /// which cooks them beside its height field -- so the two never disagree
    /// about a triangle at the region's edge.
    #[allow(clippy::chunks_exact_to_as_chunks, reason = "public signature yields slices, not arrays")]
    pub fn triangles_in(&self, region: Aabb) -> impl Iterator<Item = &[u32]> + '_ {
        self.indices.chunks_exact(3).filter(move |tri| {
            let corners = [0, 1, 2].map(|k| self.positions[tri[k] as usize]);
            let p = (corners[0] + corners[1] + corners[2]) / 3.0;
            p.x >= region.min.x && p.x <= region.max.x && p.z >= region.min.z && p.z <= region.max.z
        })
    }
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

    /// Attach the ground a cave put back in its holes. Consumed by `patch`, so
    /// it is drawn and collided with together.
    pub fn with_ground_patch(mut self, patch: Option<GroundPatch>) -> Self {
        self.ground_patch = patch.filter(|p| !p.indices.is_empty());
        self
    }

    pub fn ground_patch(&self) -> Option<&GroundPatch> {
        self.ground_patch.as_ref()
    }

    /// The sample `p` IS, if it is one: its x, z and height the grid's own, to
    /// the bit. A patch's vertices on the outline of its cells are written
    /// from the same arithmetic as [`Self::world_position`].
    fn sample_index_at(&self, p: Vec3) -> Option<(u32, u32)> {
        let (dx, dz) = self.step_metres();
        let ix = ((p.x - self.origin.x) / dx).round();
        let iz = ((p.z - self.origin.z) / dz).round();
        if ix < 0.0 || iz < 0.0 || ix > (self.resolution[0] - 1) as f32 || iz > (self.resolution[1] - 1) as f32 {
            return None;
        }
        let (ix, iz) = (ix as u32, iz as u32);
        (self.world_position(ix, iz) == p).then_some((ix, iz))
    }

    /// Samples along x and z.
    pub fn resolution(&self) -> [u32; 2] {
        self.resolution
    }

    /// Metres covered along x and z.
    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    /// The heights the lowest and the highest sample value stand for, above
    /// `origin().y`.
    pub fn height_range(&self) -> [f32; 2] {
        self.height_range
    }

    /// The world position of sample (0, 0) at height 0.
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// Metres between samples along x and z.
    pub fn cell_size(&self) -> [f32; 2] {
        let (dx, dz) = self.step_metres();
        [dx, dz]
    }

    /// The samples as stored: row-major, x fastest, `resolution()[0]` a row.
    pub fn raw_samples(&self) -> &[u16] {
        &self.samples
    }

    /// The samples a region reaches, as `(x0, x1, z0, z1)`: what the mesher
    /// covers for it.
    fn sample_span(&self, region: Aabb) -> (u32, u32, u32, u32) {
        let (dx, dz) = self.step_metres();

        let to_index = |world: f32, origin: f32, spacing: f32, max: u32| -> u32 {
            let i = ((world - origin) / spacing).floor();
            if i < 0.0 { 0 } else { (i as u32).min(max) }
        };
        (
            to_index(region.min.x, self.origin.x, dx, self.resolution[0] - 1),
            to_index(region.max.x, self.origin.x, dx, self.resolution[0] - 1),
            to_index(region.min.z, self.origin.z, dz, self.resolution[1] - 1),
            to_index(region.max.z, self.origin.z, dz, self.resolution[1] - 1),
        )
    }

    /// How many cells along x and z `patch(bounds(), 1)` meshes: every one,
    /// `resolution - 1`, unless the far edge's own arithmetic falls a hair
    /// short of the last sample -- an origin and size whose sum rounds down --
    /// and the last row or column is left out. Physics' height field leaves
    /// out the same cells, so it is the ground that is drawn (and that was
    /// cooked) to the last cell rather than nearly.
    pub fn meshed_cells(&self) -> [u32; 2] {
        let (_, x1, _, z1) = self.sample_span(self.bounds());
        [x1, z1]
    }

    /// Whether the quad between samples `x0..x1` and `z0..z1` has no ground:
    /// the hole mask read at the quad's centre. What the mesher skips, at any
    /// step.
    fn quad_is_open(&self, x0: u32, x1: u32, z0: u32, z1: u32) -> bool {
        if self.holes.is_none() {
            return false;
        }
        let (near, far) = (self.world_position(x0, z0), self.world_position(x1, z1));
        self.is_hole_at(f32::midpoint(near.x, far.x), f32::midpoint(near.z, far.z))
    }

    /// Whether cell (`ix`, `iz`) -- the quad whose lowest corner is that
    /// sample -- has no ground: exactly the cells `patch(_, 1)` leaves out,
    /// by the mesher's own test. For physics' height field, whose holes have
    /// to be the drawn ones.
    pub fn cell_is_open(&self, ix: u32, iz: u32) -> bool {
        self.quad_is_open(ix, ix + 1, iz, iz + 1)
    }

    /// Whether cell (`ix`, `iz`) is split along the diagonal that starts at
    /// its own sample (and runs to sample `ix + 1, iz + 1`), rather than the
    /// one between the other two corners: exactly the split `patch(_, 1)`
    /// draws there, by the mesher's own rule.
    pub fn cell_splits_through_first_sample(&self, ix: u32, iz: u32) -> bool {
        !splits_b_to_c(self.sample(ix, iz), self.sample(ix + 1, iz), self.sample(ix, iz + 1), self.sample(ix + 1, iz + 1))
    }

    /// Whether a world x/z sits over an open cell.
    ///
    /// Public because gameplay needs it for the same reason the mesher does: a
    /// spawn point, a foot placement or a grenade bounce resolved against ground
    /// that is not there would drop an actor into a cave ceiling.
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
mod ground_patch_tests {
    use super::*;

    /// A field of 5 x 5 samples a metre apart, sloping in x and z so no two
    /// samples share a height, with its middle cell open.
    fn field() -> Heightfield {
        let mut samples = vec![0u16; 25];
        for iz in 0..5u32 {
            for ix in 0..5u32 {
                samples[(iz * 5 + ix) as usize] = (ix * 3000 + iz * 5000 + (ix * iz) * 700) as u16;
            }
        }
        let mut mask = HoleMask::solid([4, 4]);
        mask.set_open(1, 1, true);
        Heightfield::new(samples, [5, 5], [4.0, 4.0], [-1.0, 9.0], Vec3::new(-2.0, 0.5, 3.0)).unwrap().with_holes(Some(mask))
    }

    /// The open cell filled by a fan round a vertex in its middle: four
    /// corners written as the grid writes them, and one of the patch's own.
    fn fan(field: &Heightfield) -> GroundPatch {
        let c = [field.world_position(1, 1), field.world_position(2, 1), field.world_position(2, 2), field.world_position(1, 2)];
        let mid = (c[0] + c[1] + c[2] + c[3]) / 4.0 + Vec3::new(0.0, -0.2, 0.0);
        GroundPatch {
            positions: vec![c[0], c[1], c[2], c[3], mid],
            normals: vec![Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)],
            indices: vec![0, 3, 4, 3, 2, 4, 2, 1, 4, 1, 0, 4],
        }
    }

    fn open_edges(indices: &[u32]) -> usize {
        let mut count: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        for t in indices.as_chunks::<3>().0 {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        count.values().filter(|c| **c == 1).count()
    }

    #[test]
    fn the_patch_closes_the_hole_on_the_grids_own_vertices() {
        let holed = field();
        let before = holed.patch(holed.bounds(), 1);
        assert_eq!(before.positions.len(), 25);
        assert_eq!(open_edges(&before.indices), 16 + 4);
        assert!(before.normals.is_empty(), "no patch: nothing is authored");

        let patched = { let f = field(); let p = fan(&f); f.with_ground_patch(Some(p)) };
        let after = patched.patch(patched.bounds(), 1);
        assert_eq!(after.positions.len(), 26, "four corners are the grid's own; one vertex is new");
        assert_eq!(after.indices.len(), before.indices.len() + 12);
        assert_eq!(open_edges(&after.indices), 16, "the hole's four edges are shared: the step across them is nothing");
        assert_eq!(after.normals.len(), 26);
        assert_eq!(after.normals.iter().filter(|n| **n != Vec3::ZERO).count(), 1, "only the patch's own vertex has an authored normal");
        assert_eq!(after.normals[25], Vec3::Y);
        for t in after.indices.as_chunks::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|k| after.positions[t[k] as usize]);
            assert!((b - a).cross(c - a).y > 0.0);
        }
    }

    #[test]
    fn a_corner_that_is_not_the_grids_to_the_bit_is_not_welded() {
        let f = field();
        let mut p = fan(&f);
        p.positions[0].y += 1e-5;
        let patched = f.with_ground_patch(Some(p));
        let after = patched.patch(patched.bounds(), 1);
        assert_eq!(after.positions.len(), 27, "a near miss is a second vertex, and a crack a test can count");
        assert_eq!(open_edges(&after.indices), 16 + 4);
    }

    #[test]
    fn a_region_takes_only_the_patch_triangles_inside_it() {
        let f = field();
        let p = fan(&f);
        let patched = f.with_ground_patch(Some(p));
        let b = patched.bounds();
        let far = Aabb { min: Vec3::new(b.max.x - 1.0, b.min.y, b.max.z - 1.0), max: b.max };
        let corner = patched.patch(far, 1);
        assert_eq!(corner.indices.len(), 6, "one cell of grid, none of the patch");
        assert!(corner.normals.is_empty());
        let coarse = patched.patch(b, 2);
        assert_eq!(coarse.indices.len() % 3, 0);
        assert!(coarse.indices.iter().all(|i| (*i as usize) < coarse.positions.len()));
    }

    fn bytes(sections: &[(&str, &GroundPatch)]) -> Vec<u8> {
        let mut out = b"SSTP".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend((sections.len() as u32).to_le_bytes());
        for (name, p) in sections {
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(name.as_bytes());
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            out.extend((p.positions.len() as u32).to_le_bytes());
            out.extend((p.indices.len() as u32).to_le_bytes());
            for v in p.positions.iter().chain(&p.normals) {
                for c in v.to_array() {
                    out.extend(c.to_le_bytes());
                }
            }
            for i in &p.indices {
                out.extend(i.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn the_file_reads_back_every_section_as_one_patch() {
        let f = field();
        let p = fan(&f);
        assert_eq!(GroundPatch::from_bytes(&bytes(&[("cave", &p)])).unwrap(), p);
        let two = GroundPatch::from_bytes(&bytes(&[("a", &p), ("another_cave", &p)])).unwrap();
        assert_eq!(two.positions.len(), 10);
        assert_eq!(two.indices[12..], p.indices.iter().map(|i| i + 5).collect::<Vec<_>>()[..]);
        assert!(GroundPatch::from_bytes(b"nope").is_err());
        let whole = bytes(&[("cave", &p)]);
        assert!(GroundPatch::from_bytes(&whole[..whole.len() - 3]).is_err(), "a truncated file is refused, not half read");
        let mut bad = p.clone();
        bad.indices[0] = 99;
        assert!(GroundPatch::from_bytes(&bytes(&[("cave", &bad)])).is_err());
    }

    #[test]
    fn a_scene_loads_the_patch_beside_its_hole_mask_and_none_when_there_is_none() {
        let dir = std::env::temp_dir().join(format!("ss_ground_patch_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("terrain")).unwrap();
        let f = field();
        let p = fan(&f);
        std::fs::write(dir.join("terrain/t.r16"), f.samples.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        std::fs::write(dir.join("terrain/t.holes"), f.holes().unwrap().to_bytes()).unwrap();
        let def: TerrainDef = serde_json::from_str(
            r#"{"origin":[-2.0,0.5,3.0],"holes":{"path":"terrain/t.holes","resolution":[4,4]},
                "kind":"heightfield","path":"terrain/t.r16","resolution":[5,5],"size":[4.0,4.0],"height_range":[-1.0,9.0]}"#,
        )
        .expect("a terrain def");
        let without = load(&def, &dir).unwrap();
        assert_eq!(without.patch(without.bounds(), 1).positions.len(), 25);
        std::fs::write(dir.join("terrain/t.patch"), bytes(&[("cave", &p)])).unwrap();
        let with = load(&def, &dir).unwrap();
        let patch = with.patch(with.bounds(), 1);
        assert_eq!(patch.positions.len(), 26);
        assert_eq!(open_edges(&patch.indices), 16);
        std::fs::write(dir.join("terrain/t.patch"), b"SSTPxx").unwrap();
        assert!(load(&def, &dir).err().unwrap().contains("t.patch"));
        let _ = std::fs::remove_dir_all(dir);
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
