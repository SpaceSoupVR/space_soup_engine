use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene_cuboid::Color3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}


impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: f32,
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
    pub color: Option<Color3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<Easing>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub name: String,
    pub keyframes: Vec<Keyframe>,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default)]
    pub looping: bool,
}

impl Animation {
    pub fn duration(&self) -> f32 {
        self.keyframes.iter().map(|k| k.t).fold(0.0_f32, f32::max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum PartDriver {
    HoldTrigger,
    HoldGrip,
    HandPull,
    #[default]
    Manual,

    Cyclic { cycles_per_second: f32 },
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartAnimationDef {
    pub clip: String,
    #[serde(default)]
    pub driver: PartDriver,
    #[serde(default)]
    pub easing: Easing,

    #[serde(default)]
    pub triggers: Vec<PartTrigger>,

    #[serde(default)]
    pub blend_mode: ClipBlendMode,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_when: Option<Condition>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grip_point: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Not { not: Box<Condition> },

    Compare { var: String, op: CompareOp, value: f64 },

    Equals {
        var: String,
        equals: String,
        #[serde(default)]
        negate: bool,
    },
}

impl Condition {

    pub fn holds(&self, get: &dyn Fn(&str) -> Option<String>) -> bool {
        match self {
            Condition::All { all } => all.iter().all(|c| c.holds(get)),
            Condition::Any { any } => any.iter().any(|c| c.holds(get)),
            Condition::Not { not } => !not.holds(get),
            Condition::Equals { var, equals, negate } => {
                (get(var).as_deref().unwrap_or("") == equals) != *negate
            }
            Condition::Compare { var, op, value } => {
                let v = get(var).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
                match op {
                    CompareOp::Lt => v < *value,
                    CompareOp::Le => v <= *value,
                    CompareOp::Gt => v > *value,
                    CompareOp::Ge => v >= *value,
                    CompareOp::Eq => v == *value,
                    CompareOp::Ne => v != *value,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClipBlendMode {
    #[default]
    Override,
    Additive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartTrigger {

    pub at: f32,

    #[serde(default = "default_true")]
    pub rising: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,

    pub action: PartTriggerAction,
}

fn default_true() -> bool {
    true
}

pub const TRIGGER_HYSTERESIS: f32 = 0.05;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PartTriggerAction {

    DetachPart {
        part: String,
        template: String,
        #[serde(default)]
        impulse: [f32; 3],
    },

    SetPartVisible { part: String, visible: bool },

    SetObjectVisible {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        visible: bool,
    },

    SetObjectSolid {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        solid: bool,
    },

    SetVar { name: String, value: String },

    AddVar { name: String, delta: f64 },

    PlayClip { clip: String },
    PlaySound { id: String },
    SpawnParticleBurst { id: String, count: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum PlayMode {
    #[default]
    Simultaneous,
    Sequential,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum BindingScope {
    #[default]
    ContextualHold,
    GlobalAnywhere,
}


pub const BINDING_BUTTONS: [&str; 6] = ["btn_a", "btn_b", "btn_x", "btn_y", "trigger", "grip"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationBinding {
    pub button: String,
    pub animation: String,
    #[serde(default)]
    pub play_mode: PlayMode,
    #[serde(default)]
    pub scope: BindingScope,
}

#[cfg(test)]
mod condition_tests {
    use super::*;

    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn old_equality_leaf_is_byte_compatible() {
        let c: Condition = serde_json::from_str(r#"{"var":"chamber","equals":"loaded"}"#).unwrap();
        assert!(c.holds(&vars(&[("chamber", "loaded")])));
        assert!(!c.holds(&vars(&[("chamber", "empty")])));
        assert!(!c.holds(&vars(&[])));
    }

    #[test]
    fn compound_all_with_numeric_compare() {

        let c: Condition = serde_json::from_str(
            r#"{"all":[{"var":"bolt","equals":"back"},{"var":"mag_rounds","op":"gt","value":0}]}"#,
        )
        .unwrap();
        assert!(c.holds(&vars(&[("bolt", "back"), ("mag_rounds", "30")])));
        assert!(!c.holds(&vars(&[("bolt", "back"), ("mag_rounds", "0")])));
        assert!(!c.holds(&vars(&[("bolt", "forward"), ("mag_rounds", "30")])));
    }

    #[test]
    fn any_and_not_compose() {
        let c: Condition = serde_json::from_str(
            r#"{"any":[{"not":{"var":"safety","equals":"safe"}},{"var":"override","equals":"on"}]}"#,
        )
        .unwrap();
        assert!(c.holds(&vars(&[("safety", "fire")])));
        assert!(!c.holds(&vars(&[("safety", "safe")])));
        assert!(c.holds(&vars(&[("safety", "safe"), ("override", "on")])));
    }
}
