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

    #[serde(flatten)]
    pub kind: TerrainKind,
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
