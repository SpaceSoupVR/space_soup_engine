use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::brush::{self, BrushSolid};
use crate::physics::point_in_obb;
use crate::scene::GameObject;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VolumeAction {

    SetVar { name: String, value: String },
    AddVar { name: String, delta: f64 },

    SetObjectVisible { id: String, visible: bool },

    SetObjectSolid { id: String, solid: bool },
    PlaySound { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerVolumeDef {

    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_enter: Vec<VolumeAction>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_exit: Vec<VolumeAction>,

    #[serde(default)]
    pub once: bool,
}

impl Default for TriggerVolumeDef {
    fn default() -> Self {
        Self {
            enabled: true,
            var: None,
            on_enter: Vec::new(),
            on_exit: Vec::new(),
            once: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum VolumeShape {

    Obb {
        center: Vec3,
        half_size: Vec3,
        rotation: glam::Quat,
    },

    Solids(Vec<BrushSolid>),
}

impl VolumeShape {

    #[must_use]
    pub fn of(object: &GameObject) -> Self {
        if let Some(def) = &object.brush {
            let solids = def.evaluate();
            if !solids.is_empty() {
                return Self::Solids(solids);
            }
        }
        Self::Obb {
            center: object.cuboid.position,
            half_size: object.cuboid.half_size,
            rotation: object.cuboid.rotation,
        }
    }

    #[must_use]
    pub fn contains(&self, p: Vec3) -> bool {
        match self {
            Self::Obb {
                center,
                half_size,
                rotation,
            } => point_in_obb(p, *center, *half_size, *rotation),

            Self::Solids(solids) => {
                let at: brush::Vec3 = [f64::from(p.x), f64::from(p.y), f64::from(p.z)];
                solids.iter().any(|s| brush::contains_point(s, at, 0.0))
            }
        }
    }
}
