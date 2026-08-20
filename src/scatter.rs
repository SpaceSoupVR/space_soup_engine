//! Procedural scatter: paint trees, grass and rocks, then move one of them.
//!
//! The whole design turns on one requirement, and it is the thing that makes
//! Menyr's terrain feel authored rather than generated: you scatter a forest,
//! then you nudge one tree, then you raise the density -- and the tree you
//! nudged must still be the tree you nudged.
//!
//! That rules out storing the output. A stroke is stored as PARAMETERS plus a
//! seed and resolved deterministically, so a density change is a one-line diff
//! rather than five thousand new objects in the scene file. And it means a
//! generated instance cannot carry an authored `uuid`: its identity is DERIVED
//! from (layer, stroke, slot), so an override recorded against slot 4173 still
//! finds slot 4173 after the density moves.
//!
//! The generation order is therefore load-bearing. Slots are produced in a
//! fixed sequence and the first N are taken, so raising density APPENDS rather
//! than reshuffling. Getting that wrong does not error -- it silently relocates
//! every hand-placed edit in the level.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// One thing that can be placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatterPrototype {
    /// Model path, relative to the game directory.
    pub mesh: String,

    /// Relative likelihood of being chosen. Zero means "kept in the palette but
    /// not placed", which is more useful than deleting it while iterating.
    #[serde(default = "one")]
    pub weight: f32,

    /// Uniform scale range, inclusive.
    #[serde(default = "unit_range")]
    pub scale_range: [f32; 2],

    /// Steepest ground this will sit on, in degrees. Trees do not grow on
    /// cliffs, and a forest that ignores slope reads as obviously fake from the
    /// first ridge you look at.
    #[serde(default = "default_max_slope")]
    pub max_slope_deg: f32,
}

fn one() -> f32 { 1.0 }
fn unit_range() -> [f32; 2] { [1.0, 1.0] }
fn default_max_slope() -> f32 { 35.0 }

/// One brush stroke: a disc the author painted over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatterStroke {
    /// Stable within the layer, and never reused. Slot identity hangs off it,
    /// so recycling an id would silently rebind every override on the old one.
    pub id: u32,
    /// World x/z of the disc centre.
    pub center: [f32; 2],
    pub radius: f32,
    /// Instances per square metre.
    pub density: f32,
}

/// Which generated instance an override refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScatterKey {
    pub stroke: u32,
    pub slot: u32,
}

/// A human edit to one generated instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ScatterOverride {
    /// Moved, turned or resized by hand.
    Transform {
        key: ScatterKey,
        #[serde(default)]
        position: Option<[f32; 3]>,
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        #[serde(default)]
        scale: Option<f32>,
    },
    /// Deleted. Kept as a record rather than removed from the output, because
    /// the output is regenerated every load and a deletion has to survive that.
    Removed { key: ScatterKey },
    /// Swapped for a different prototype.
    Prototype { key: ScatterKey, prototype: usize },
}

impl ScatterOverride {
    pub fn key(&self) -> ScatterKey {
        match self {
            Self::Transform { key, .. } | Self::Removed { key } | Self::Prototype { key, .. } => *key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatterLayer {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub seed: u32,
    pub prototypes: Vec<ScatterPrototype>,
    #[serde(default)]
    pub strokes: Vec<ScatterStroke>,
    #[serde(default)]
    pub overrides: Vec<ScatterOverride>,
}

/// One resolved placement. Never stored -- regenerated on every load.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterInstance {
    pub key: ScatterKey,
    pub prototype: usize,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: f32,
}

/// A 32-bit mixer, written out rather than pulled from a crate.
///
/// The editor has to produce byte-identical placements to the runtime or the
/// preview is a lie, and "use the same RNG crate in Rust and JavaScript" is not
/// available. This is `lowbias32`: exact in both languages with wrapping 32-bit
/// multiply, which JS spells `Math.imul`.
#[inline]
pub fn mix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// A stable stream of values for one slot, independent of every other slot.
///
/// Per-slot rather than sequential on purpose: a running generator would make
/// slot N's value depend on how many values slot N-1 happened to consume, so
/// adding a single field to the placement logic would reshuffle the entire
/// level. Hashing the coordinates keeps each slot's draw its own business.
#[inline]
fn draw(seed: u32, stroke: u32, slot: u32, channel: u32) -> f32 {
    let h = mix32(seed ^ mix32(stroke.wrapping_mul(0x9e37_79b9) ^ mix32(slot.wrapping_mul(0x85eb_ca6b) ^ channel)));
    // 24 bits is ample and avoids the top-bit bias of a straight division.
    (h >> 8) as f32 / ((1u32 << 24) as f32)
}

/// How many instances a stroke asks for.
pub fn slot_count(stroke: &ScatterStroke) -> u32 {
    let area = std::f32::consts::PI * stroke.radius * stroke.radius;
    (area * stroke.density.max(0.0)).round().max(0.0) as u32
}

/// Ground query: height and slope at a world x/z.
///
/// A closure rather than a `TerrainSource` so a scene with no terrain can still
/// scatter onto a flat plane, and so the editor can pass its own heightfield
/// without either side depending on the other's representation.
pub trait Ground {
    fn height_at(&self, x: f32, z: f32) -> Option<f32>;

    /// Slope in degrees, sampled by finite difference.
    fn slope_deg_at(&self, x: f32, z: f32, step: f32) -> f32 {
        let (Some(h), Some(hx), Some(hz)) = (
            self.height_at(x, z),
            self.height_at(x + step, z),
            self.height_at(x, z + step),
        ) else {
            return 0.0;
        };
        let gradient = (((hx - h) / step).powi(2) + ((hz - h) / step).powi(2)).sqrt();
        gradient.atan().to_degrees()
    }
}

/// Flat ground at a fixed height, for scenes with no terrain.
pub struct FlatGround(pub f32);

impl Ground for FlatGround {
    fn height_at(&self, _x: f32, _z: f32) -> Option<f32> {
        Some(self.0)
    }
}

/// Resolve a layer into placements.
///
/// Deterministic: same layer and same ground gives the same instances, in the
/// same order, on every machine and every load.
pub fn resolve(layer: &ScatterLayer, ground: &dyn Ground) -> Vec<ScatterInstance> {
    let total_weight: f32 = layer.prototypes.iter().map(|p| p.weight.max(0.0)).sum();
    if layer.prototypes.is_empty() || total_weight <= 0.0 {
        return Vec::new();
    }

    let mut removed = std::collections::HashSet::new();
    let mut edits = std::collections::HashMap::new();
    for over in &layer.overrides {
        match over {
            ScatterOverride::Removed { key } => {
                removed.insert(*key);
            }
            other => {
                edits.insert(other.key(), other);
            }
        }
    }

    let mut out = Vec::new();
    for stroke in &layer.strokes {
        for slot in 0..slot_count(stroke) {
            let key = ScatterKey { stroke: stroke.id, slot };
            if removed.contains(&key) {
                continue;
            }

            // Uniform over the disc: sqrt on the radius, or everything piles
            // into the middle and the edge of every stroke looks bald.
            let r = stroke.radius * draw(layer.seed, stroke.id, slot, 0).sqrt();
            let theta = draw(layer.seed, stroke.id, slot, 1) * std::f32::consts::TAU;
            let x = stroke.center[0] + r * theta.cos();
            let z = stroke.center[1] + r * theta.sin();

            let Some(y) = ground.height_at(x, z) else { continue };

            let mut prototype = pick_prototype(layer, stroke.id, slot, total_weight);
            let mut position = Vec3::new(x, y, z);
            let mut scale = {
                let [lo, hi] = layer.prototypes[prototype].scale_range;
                lo + (hi - lo) * draw(layer.seed, stroke.id, slot, 3)
            };
            let mut rotation =
                Quat::from_rotation_y(draw(layer.seed, stroke.id, slot, 4) * std::f32::consts::TAU);

            // Slope rejection happens BEFORE overrides: a tree the author moved
            // by hand onto a cliff is a decision, not a mistake to undo.
            match edits.get(&key) {
                None => {
                    let slope = ground.slope_deg_at(x, z, 0.5);
                    if slope > layer.prototypes[prototype].max_slope_deg {
                        continue;
                    }
                }
                Some(ScatterOverride::Prototype { prototype: p, .. }) => {
                    if *p < layer.prototypes.len() {
                        prototype = *p;
                    }
                }
                Some(ScatterOverride::Transform { position: p, rotation: r, scale: s, .. }) => {
                    if let Some(p) = p { position = Vec3::from(*p); }
                    if let Some(r) = r { rotation = Quat::from_array(*r); }
                    if let Some(s) = s { scale = *s; }
                }
                Some(ScatterOverride::Removed { .. }) => unreachable!("handled above"),
            }

            out.push(ScatterInstance { key, prototype, position, rotation, scale });
        }
    }
    out
}

fn pick_prototype(layer: &ScatterLayer, stroke: u32, slot: u32, total_weight: f32) -> usize {
    let mut pick = draw(layer.seed, stroke, slot, 2) * total_weight;
    for (i, proto) in layer.prototypes.iter().enumerate() {
        pick -= proto.weight.max(0.0);
        if pick <= 0.0 {
            return i;
        }
    }
    layer.prototypes.len() - 1
}
