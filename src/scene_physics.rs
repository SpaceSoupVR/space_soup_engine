use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BodyMode {
    Static,

    Kinematic,

    Dynamic,
}

fn default_body_mode() -> BodyMode {
    BodyMode::Dynamic
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Box,
    Sphere { radius: f32 },
    Capsule { radius: f32, half_height: f32 },
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Box
    }
}

fn default_friction() -> f32 {
    0.5
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigidBodyDef {
    /// Whether this collider is in the world.
    ///
    /// An authored OFF state, deliberately NOT the absence of the component.
    /// Three reasons, and the first is the one that forces it:
    ///
    /// 1. A trigger that turns collision ON needs a definition to spawn from.
    ///    If "not solid" meant "no rigid_body", there would be nothing left to
    ///    turn on, and a force field that starts open could never close.
    /// 2. Deleting the component to mean "off" throws away the mass, friction,
    ///    shape and collider size someone tuned, and getting them back is
    ///    retyping them.
    /// 3. It gives the authored state and the runtime state ONE field. The
    ///    alternative -- authored by presence, runtime by a separate flag --
    ///    is two sources of truth for one question, which is how a scene ends
    ///    up disagreeing with what is on screen.
    ///
    /// Defaults to true, so every scene written before this existed keeps
    /// exactly the collision it had.
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_body_mode")]
    pub mode: BodyMode,
    #[serde(default)]
    pub shape: ColliderShape,

    #[serde(default)]
    pub mass: Option<f32>,
    #[serde(default = "default_friction")]
    pub friction: f32,
    #[serde(default)]
    pub restitution: f32,

    #[serde(default)]
    pub linear_velocity: [f32; 3],

    #[serde(default)]
    pub respawn_interval: Option<f32>,

    #[serde(default)]
    pub collider_half_size: Option<[f32; 3]>,

    #[serde(default)]
    pub collider_offset: [f32; 3],
}

impl Default for RigidBodyDef {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: default_body_mode(),
            shape: ColliderShape::default(),
            mass: None,
            friction: default_friction(),
            restitution: 0.0,
            respawn_interval: None,
            linear_velocity: [0.0, 0.0, 0.0],
            collider_half_size: None,
            collider_offset: [0.0, 0.0, 0.0],
        }
    }
}
