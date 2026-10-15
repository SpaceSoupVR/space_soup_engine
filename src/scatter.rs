use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatterPrototype {

    pub mesh: String,

    #[serde(default = "one")]
    pub weight: f32,

    #[serde(default = "unit_range")]
    pub scale_range: [f32; 2],

    #[serde(default = "default_max_slope")]
    pub max_slope_deg: f32,

    #[serde(default = "full_turn")]
    pub yaw_range: [f32; 2],

    #[serde(default)]
    pub tilt_deg: f32,
}

fn one() -> f32 { 1.0 }
fn unit_range() -> [f32; 2] { [1.0, 1.0] }
fn default_max_slope() -> f32 { 35.0 }
fn full_turn() -> [f32; 2] { [0.0, 360.0] }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatterStroke {

    pub id: u32,

    pub center: [f32; 2],
    pub radius: f32,

    pub density: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScatterKey {
    pub stroke: u32,
    pub slot: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ScatterOverride {

    Transform {
        key: ScatterKey,
        #[serde(default)]
        position: Option<[f32; 3]>,
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        #[serde(default)]
        scale: Option<f32>,
    },

    Removed { key: ScatterKey },

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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScatterInstance {
    pub key: ScatterKey,
    pub prototype: usize,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: f32,
}

#[inline]
pub fn mix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

#[inline]
fn draw(seed: u32, stroke: u32, slot: u32, channel: u32) -> f32 {
    let h = mix32(seed ^ mix32(stroke.wrapping_mul(0x9e37_79b9) ^ mix32(slot.wrapping_mul(0x85eb_ca6b) ^ channel)));

    (h >> 8) as f32 / ((1u32 << 24) as f32)
}

pub fn slot_count(stroke: &ScatterStroke) -> u32 {
    let area = std::f32::consts::PI * stroke.radius * stroke.radius;
    (area * stroke.density.max(0.0)).round().max(0.0) as u32
}

pub trait Ground {
    fn height_at(&self, x: f32, z: f32) -> Option<f32>;

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

pub struct FlatGround(pub f32);

impl Ground for FlatGround {
    fn height_at(&self, _x: f32, _z: f32) -> Option<f32> {
        Some(self.0)
    }
}

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
            let mut rotation = {
                let proto = &layer.prototypes[prototype];
                let [lo, hi] = proto.yaw_range;

                let (lo, hi) = (lo / 360.0, hi / 360.0);
                let turns = lo + (hi - lo) * draw(layer.seed, stroke.id, slot, 4);
                let yaw = Quat::from_rotation_y(turns * std::f32::consts::TAU);
                if proto.tilt_deg <= 0.0 {
                    yaw
                } else {

                    let lean = (draw(layer.seed, stroke.id, slot, 5) * proto.tilt_deg).to_radians();
                    let az = draw(layer.seed, stroke.id, slot, 6) * std::f32::consts::TAU;
                    let axis = Vec3::new(az.cos(), 0.0, az.sin());
                    yaw * Quat::from_axis_angle(axis, lean)
                }
            };

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
