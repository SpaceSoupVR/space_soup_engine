use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene_cuboid::Color3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Default for Easing {
    fn default() -> Self {
        Self::Linear
    }
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

// No Eq: Cyclic carries a rate, and f32 has no total equality. PartialEq is what
// the driver comparisons actually use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PartDriver {
    HoldTrigger,
    HoldGrip,
    HandPull,
    Manual,
    /// Runs off a clock instead of a hand: while the drive input is held, the
    /// blend sweeps 0 -> 1 -> 0 repeatedly at `cycles_per_second`.
    ///
    /// Everything else here maps a continuous input to a blend, which cannot
    /// express "play once and come back" -- so a reciprocating bolt had to be a
    /// hand-written script per weapon, and could not even tell when the trigger
    /// was released. This makes it authored data.
    ///
    /// Held by the trigger, since that is what an automatic weapon cycles on.
    Cyclic { cycles_per_second: f32 },
}

impl Default for PartDriver {
    fn default() -> Self {
        Self::Manual
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartAnimationDef {
    pub clip: String,
    #[serde(default)]
    pub driver: PartDriver,
    #[serde(default)]
    pub easing: Easing,

    /// Things that happen at a point in this clip's blend.
    #[serde(default)]
    pub triggers: Vec<PartTrigger>,

    /// How this clip combines with the others on a shared part. Override (the
    /// default) takes the part outright; Additive layers on top. See
    /// space_soup's ClipBlendMode.
    #[serde(default)]
    pub blend_mode: ClipBlendMode,

    /// Only respond to its driver while this holds.
    ///
    /// A weapon is a state machine -- bolt forward or locked back, chamber
    /// loaded or empty, safety on or off -- and most of its actions are only
    /// legal in some of those states. Without a gate every clip answers its
    /// driver always, so pulling the trigger cycles the bolt whether or not
    /// there is a round in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_when: Option<Condition>,

    /// The grip point the pulling hand is posed from, for a HandPull clip.
    ///
    /// A HandPull already reads the hand's position to drive the blend, but the
    /// hand itself keeps whatever pose it had -- an open palm dragging a charging
    /// handle. Naming a grip point here gives that hand the grip's finger curl and
    /// orientation for as long as the pull lasts.
    ///
    /// Point it at a part-scoped grip (`GripPointDef::part`) on the part being
    /// pulled, so the hand tracks the handle rather than the receiver. This poses
    /// the hand only; it creates no attachment, so the object does not follow it.
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

/// A condition over authored state variables. Beyond the old single-equality gate a weapon needs
/// compound logic ("bolt is back AND chamber is loaded") and counts ("magazine has rounds left"),
/// so this is a small boolean tree: an equality, a numeric comparison, or all/any/not of others.
///
/// Untagged so the leaf `{ "var": "chamber", "equals": "loaded" }` stays byte-compatible with the
/// clips authored before compound conditions existed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Not { not: Box<Condition> },
    /// Numeric: parses the var as a number (unset = 0) and compares to `value`.
    Compare { var: String, op: CompareOp, value: f64 },
    /// String equality, the backward-compatible leaf.
    Equals {
        var: String,
        equals: String,
        #[serde(default)]
        negate: bool,
    },
}

impl Condition {
    /// `get` reads the current string value of a state var (None = unset, treated as "" / 0).
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

/// Mirrors space_soup::renderer::mesh::skin::ClipBlendMode as authorable data.
/// Declared here rather than imported so the scene format does not depend on the
/// renderer crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClipBlendMode {
    #[default]
    Override,
    Additive,
}

/// Something discrete that fires when a clip's blend crosses a threshold.
///
/// A part animation is a pose blend, not a timeline: the runtime lerps the bind
/// pose toward a target by a scalar, and blend is the only axis it has. So a
/// trigger fires on a blend crossing rather than at a time, which also matches
/// the physical statement an animator actually wants to make -- "once the
/// magazine is 85% of the way out, the feed lips have released it".
///
/// Detaching, spawning and hiding are discrete state changes. They cannot be
/// keyframe channels: a keyframe interpolates, and half-detached is not a state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartTrigger {
    /// Blend value to cross, 0..1.
    pub at: f32,

    /// Fire while the blend is increasing (the default) or while it falls back.
    /// Rising is the usual case -- the action happens as the motion completes.
    #[serde(default = "default_true")]
    pub rising: bool,

    /// Only fire when this holds. Lets one blend crossing mean different things in different
    /// states -- racking the bolt ejects a case only when one is chambered, chambers a fresh
    /// round only when the magazine has any. Absent = fires on every crossing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,

    pub action: PartTriggerAction,
}

fn default_true() -> bool {
    true
}

/// Hysteresis band around a trigger's threshold.
///
/// A player holding a control near the threshold jitters across it many times a
/// second. Without a band, an eject trigger would fire repeatedly while a hand
/// shook. Rearming only once the blend has retreated this far past the threshold
/// makes a single deliberate motion produce a single event.
pub const TRIGGER_HYSTERESIS: f32 = 0.05;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PartTriggerAction {
    /// Hand a part over to physics: spawn `template` at the part's current world
    /// transform, optionally with an impulse, and hide the source part so it does
    /// not appear twice.
    ///
    /// The part itself cannot become a physics body -- it is a joint inside a
    /// skinned mesh, not an object -- so the handover is a spawn plus a hide.
    DetachPart {
        part: String,
        template: String,
        #[serde(default)]
        impulse: [f32; 3],
    },
    /// Show or hide a part. See GameObject::hidden_parts.
    SetPartVisible { part: String, visible: bool },
    /// Record a state, which other clips can require via `enabled_when` and
    /// scripts can read with get_var. This is what makes a weapon a state
    /// machine rather than a set of independent motions.
    SetVar { name: String, value: String },
    /// Numeric state change: add `delta` (may be negative) to a var read as a number. This is what
    /// counts rounds -- feeding chambers one and does `mag_rounds -= 1`.
    AddVar { name: String, delta: f64 },
    /// Drive another clip to its target pose -- the mechanical follow-on. The bolt returning to
    /// battery plays `chamber_feed`, sliding the round up into the chamber.
    PlayClip { clip: String },
    PlaySound { id: String },
    SpawnParticleBurst { id: String, count: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayMode {
    Simultaneous,
    Sequential,
}

impl Default for PlayMode {
    fn default() -> Self {
        Self::Simultaneous
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingScope {
    ContextualHold,
    GlobalAnywhere,
}

impl Default for BindingScope {
    fn default() -> Self {
        Self::ContextualHold
    }
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
        assert!(!c.holds(&vars(&[]))); // unset reads as ""
    }

    #[test]
    fn compound_all_with_numeric_compare() {
        // bolt is back AND the magazine has rounds left
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
