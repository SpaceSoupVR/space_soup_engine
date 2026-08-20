//! Terrain as a source of geometry, not as a format.
//!
//! There was no terrain representation before this. `TerrainColliderDef` marks
//! nodes of an authored glTF as collidable, which is Blender-made mesh terrain
//! and gives an editor nothing to sculpt against.
//!
//! The decision this file exists to make is that the engine asks a
//! `TerrainSource` for the geometry over a region, rather than knowing what
//! terrain *is*. A heightfield implements it; the existing mesh path can
//! implement it; a sparse voxel or SDF store can implement it in two years
//! without touching a line above the trait. That is what keeps runtime
//! destruction of arbitrary topology possible without paying for it now --
//! see the terrain section of AAA_LEVEL_EDITOR_RESEARCH_AND_PLAN.
//!
//! Region-at-a-time rather than all-at-once for the same reason streaming and
//! 64-player relevance filtering both need a spatial index: they are the same
//! query, and asking for the whole world is the one shape that cannot be made
//! to scale later.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::physics::Aabb;

/// How a scene's terrain is stored.
///
/// Heights live in a separate asset, never inline: a 512x512 field is 262,144
/// samples, which would dwarf every scene file and make every terrain edit an
/// unreadable diff -- exactly the problem canonical serialization was added to
/// solve for object data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerrainKind {
    /// A regular grid of heights, stored as little-endian `u16` samples in
    /// row-major order (x varies fastest), mapped linearly onto `height_range`.
    ///
    /// u16 rather than f32: it halves the file, it is what every terrain tool
    /// exports, and over a 200m vertical range it still resolves to 3mm --
    /// finer than anything a player can stand on and notice.
    Heightfield {
        /// Path to the raw sample file, relative to the game directory.
        path: String,
        /// Samples along x and z. Both must be at least 2.
        resolution: [u32; 2],
        /// World-space extent covered, in metres.
        size: [f32; 2],
        /// World Y that sample values 0 and 65535 map to.
        height_range: [f32; 2],
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainDef {
    /// World position of the terrain's minimum corner (its x/z origin, and the
    /// Y that `height_range` is measured against).
    #[serde(default)]
    pub origin: [f32; 3],

    /// Authored material blend weights over the same footprint, if any.
    ///
    /// Optional because slope- and height-driven blending already gives a scene
    /// plausible ground with nothing authored, and a level that never needs
    /// hand-painted material should not carry an empty megabyte to say so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splat: Option<SplatDef>,

    #[serde(flatten)]
    pub kind: TerrainKind,
}

/// Where a scene's splat map lives and how finely it is sampled.
///
/// No `size` or `origin`: the map covers exactly the terrain's footprint. See
/// `SplatMap` for why that is locked rather than configurable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplatDef {
    /// Path to the raw RGBA8 file, relative to the game directory.
    pub path: String,
    /// Texels along x and z.
    pub resolution: [u32; 2],
    /// How many material layers each texel carries.
    ///
    /// Always 4 today, and recorded anyway. The file is headerless raw bytes,
    /// so without this a 4-layer and an 8-layer map of different resolutions
    /// can have identical byte counts and nothing can tell them apart. Writing
    /// it now makes a future second texture an ADDITIVE change; leaving it out
    /// would make it a migration with a heuristic in the middle. It defaults on
    /// read, so maps written before this field keep loading.
    #[serde(default = "default_splat_layers")]
    pub layers: u8,
}

fn default_splat_layers() -> u8 {
    SPLAT_LAYERS as u8
}

/// A patch of terrain geometry, ready for collision or rendering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainPatch {
    pub positions: Vec<Vec3>,
    /// Triangle indices into `positions`.
    pub indices: Vec<u32>,
}

/// Where terrain geometry comes from.
///
/// Deliberately small. Everything above it -- physics, rendering, the editor's
/// preview -- should be expressible in these three questions, so that a new
/// implementation is a new file rather than a new set of special cases.
pub trait TerrainSource: Send + Sync {
    /// The whole extent this source can answer for.
    fn bounds(&self) -> Aabb;

    /// Ground height at a world x/z, or `None` outside `bounds`.
    ///
    /// The hot query: locomotion, foot placement, spawn resolution and grenade
    /// bounces all ask it far more often than they ask for geometry.
    fn height_at(&self, x: f32, z: f32) -> Option<f32>;

    /// Triangles covering `region`, decimated by `step` samples.
    ///
    /// `step` is the LOD knob: 1 is every sample, 4 is every fourth. Distant
    /// chunks get a coarse step, which is the whole reason this is a region
    /// query rather than a whole-world one.
    fn patch(&self, region: Aabb, step: u32) -> TerrainPatch;
}

/// A regular grid of heights.
#[derive(Debug)]
pub struct Heightfield {
    samples: Vec<u16>,
    resolution: [u32; 2],
    size: [f32; 2],
    height_range: [f32; 2],
    origin: Vec3,
}

impl Heightfield {
    /// Build from raw samples. Fails rather than guessing when the sample count
    /// does not match the stated resolution -- a truncated read would otherwise
    /// produce terrain that is silently flat at one edge.
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
        Ok(Self { samples, resolution, size, height_range, origin })
    }

    /// Decode little-endian u16 pairs, as written by every terrain tool and by
    /// the editor's own sculpt export.
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

    /// Spacing between samples, in metres.
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

        // Bilinear rather than nearest: a player walking across a coarse field
        // should climb a slope, not a staircase of sample-sized steps.
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

        // Sample indices covering the region, clamped to the field and widened
        // by one so adjacent patches share an edge instead of leaving a crack.
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
                // Counter-clockwise, matching the renderer's front_face(Ccw) --
                // cuboids were once wound the other way and every one of them
                // lit inside-out without erroring.
                patch.indices.extend_from_slice(&[a, c, b, b, c, d]);
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

/// Build a live source from a scene's `TerrainDef`, reading the sample asset.
///
/// Both the server (for collision) and a client (for rendering) call this
/// against their own copy of the game directory. The samples are static scene
/// data that is already on every machine, so terrain costs nothing on the wire
/// -- which matters rather a lot when the target is 64 players and the snapshot
/// budget is the binding constraint.
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
            Ok(Box::new(field))
        }
    }
}

/// Lets a heightfield answer scatter's ground queries.
///
/// Kept as a concrete impl rather than a blanket one over `TerrainSource`:
/// coherence would then make `FlatGround` -- which is deliberately not terrain
/// at all -- awkward to implement, and scatter only ever needs a height at an
/// x/z. A future voxel or mesh source adds its own two-line impl.
impl crate::scatter::Ground for Heightfield {
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        TerrainSource::height_at(self, x, z)
    }
}

/// Number of splat layers. Four because that is what one RGBA8 texel holds and
/// what one texture fetch returns; a fifth layer costs a second texture and a
/// second fetch per fragment, which is a real budget decision rather than a
/// constant to bump casually.
pub const SPLAT_LAYERS: usize = 4;

/// Authored per-texel blend weights across the terrain's material layers.
///
/// Stored as RGBA8 in row-major order with x varying fastest -- deliberately
/// the same convention as the heightfield, because two bulk grids over the same
/// ground disagreeing about row order is a bug that looks like an art mistake.
///
/// The grid is INDEPENDENT of the heightfield's resolution but covers exactly
/// the same footprint. Independent because the two want different densities: a
/// 513x513 heightfield resolves 3mm of elevation, while blend weights that fine
/// are invisible and cost four times the bytes. Locked to the same footprint
/// because a splat map free to have its own origin and size is a permanent
/// source of misalignment that renders as terrain painted slightly off from
/// where the artist painted it.
#[derive(Clone, Debug, PartialEq)]
pub struct SplatMap {
    weights: Vec<u8>,
    resolution: [u32; 2],
}

impl SplatMap {
    /// Build from raw RGBA8 bytes.
    ///
    /// Rejects a byte count that does not match the resolution rather than
    /// padding: a truncated splat read would otherwise paint one edge of the
    /// level with layer 0 and never report anything.
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

    /// A map with every texel fully on `layer`.
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

    /// Weights at a texel, clamped to the grid.
    pub fn at_index(&self, ix: u32, iz: u32) -> [u8; SPLAT_LAYERS] {
        let ix = ix.min(self.resolution[0] - 1) as usize;
        let iz = iz.min(self.resolution[1] - 1) as usize;
        let base = (iz * self.resolution[0] as usize + ix) * SPLAT_LAYERS;
        let mut out = [0u8; SPLAT_LAYERS];
        out.copy_from_slice(&self.weights[base..base + SPLAT_LAYERS]);
        out
    }

    /// Weights at a normalised position, `u`/`v` in 0..=1 across the footprint.
    ///
    /// Nearest-sample, not bilinear. The GPU filters these when it samples the
    /// texture; this exists for gameplay queries -- footstep sounds, surface
    /// friction, whether a grenade landed on rock -- which want the authored
    /// value at a point, and where interpolating across a material boundary
    /// would invent a half-gravel-half-water surface that is neither.
    pub fn sample_uv(&self, u: f32, v: f32) -> [u8; SPLAT_LAYERS] {
        let fx = (u.clamp(0.0, 1.0) * (self.resolution[0] - 1) as f32).round();
        let fz = (v.clamp(0.0, 1.0) * (self.resolution[1] - 1) as f32).round();
        self.at_index(fx as u32, fz as u32)
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

/// Read a scene's splat map from disk, if it declares one.
///
/// Separate from `load` rather than folded into `TerrainSource` on purpose: the
/// heightfield answers geometry questions and gets swapped wholesale when a
/// scene moves to voxels or heightfield chunks, while blend weights are a
/// material concern that outlives that choice. Bolting them onto the geometry
/// trait would make every future terrain representation reimplement them.
pub fn load_splat(
    def: &TerrainDef,
    game_dir: &std::path::Path,
) -> Result<Option<SplatMap>, String> {
    let Some(splat) = &def.splat else {
        return Ok(None);
    };
    let full = game_dir.join(&splat.path);
    if splat.layers as usize != SPLAT_LAYERS {
        // Refused rather than best-effort: a map with a different layer count
        // is not slightly wrong, it is a different pixel layout, and reading it
        // as four would shear every weight across the wrong materials.
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

    /// The byte count must match the resolution exactly. A short read that got
    /// padded would paint one edge of a level with layer 0 and report nothing,
    /// which is the same failure mode the heightfield guards against.
    #[test]
    fn a_byte_count_that_does_not_match_the_resolution_is_rejected() {
        assert!(SplatMap::new(vec![0; 4 * 4], [2, 2]).is_ok());
        let err = SplatMap::new(vec![0; 4 * 3], [2, 2]).unwrap_err();
        assert!(err.contains("12 bytes"), "error should name the actual count: {err}");
        assert!(err.contains("16"), "error should name the expected count: {err}");
    }

    /// Row-major with x varying fastest, matching the heightfield. Two bulk
    /// grids over the same ground disagreeing about row order reads as an art
    /// mistake and is nearly impossible to spot from a screenshot.
    #[test]
    fn indexing_is_row_major_with_x_fastest() {
        // 3x2 grid; mark texel (2, 1) -- the last one -- with layer 3.
        let mut bytes = vec![0u8; 3 * 2 * SPLAT_LAYERS];
        let last = (1 * 3 + 2) * SPLAT_LAYERS;
        bytes[last + 3] = 200;
        let m = SplatMap::new(bytes, [3, 2]).unwrap();
        assert_eq!(m.at_index(2, 1), [0, 0, 0, 200]);
        assert_eq!(m.at_index(1, 1), [0, 0, 0, 0]);
        assert_eq!(m.dominant_uv(1.0, 1.0), (3, 200));
    }

    /// Out-of-range indices clamp instead of panicking. Gameplay queries arrive
    /// from positions that can sit a hair outside the footprint after physics,
    /// and a panic there is far worse than a repeated edge texel.
    #[test]
    fn indices_and_uvs_clamp_to_the_grid() {
        let m = SplatMap::solid([2, 2], 0);
        assert_eq!(m.at_index(99, 99), [255, 0, 0, 0]);
        assert_eq!(m.sample_uv(-5.0, 5.0), [255, 0, 0, 0]);
    }

    /// sample_uv is nearest, NOT bilinear: interpolating across a material
    /// boundary invents a surface that is half rock and half water and is
    /// neither, which is wrong for the footstep/friction queries this serves.
    #[test]
    fn sampling_is_nearest_so_a_boundary_stays_a_boundary() {
        // 2x1: left texel all layer 0, right texel all layer 1.
        let bytes = vec![255, 0, 0, 0, /**/ 0, 255, 0, 0];
        let m = SplatMap::new(bytes, [2, 1]).unwrap();
        assert_eq!(m.dominant_uv(0.0, 0.0).0, 0);
        assert_eq!(m.dominant_uv(1.0, 0.0).0, 1);
        // Exactly halfway must resolve to one side or the other, never a blend.
        let (layer, weight) = m.dominant_uv(0.5, 0.0);
        assert!(layer == 0 || layer == 1, "midpoint resolved to layer {layer}");
        assert_eq!(weight, 255, "a nearest sample must return an authored value, not a mix");
    }

    #[test]
    fn a_scene_with_no_splat_block_loads_as_none() {
        let def = TerrainDef {
            origin: [0.0; 3],
            splat: None,
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

    /// Maps written before the field existed must keep loading.
    #[test]
    fn a_splat_block_without_a_layer_count_defaults_to_four() {
        let def = def_with(
            r#"{"origin":[0,0,0],"splat":{"path":"t.splat","resolution":[4,4]},
                "kind":"heightfield","path":"h.r16","resolution":[2,2],
                "size":[1,1],"height_range":[0,1]}"#,
        );
        assert_eq!(def.splat.as_ref().unwrap().layers, 4);
    }

    /// A map declaring a layout this build cannot read is refused, not
    /// best-effort decoded -- reading 8-layer bytes as 4 would shear every
    /// weight across the wrong materials and look like a painting mistake.
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

    /// The count survives a write/read cycle, which is the whole point of
    /// recording it.
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
