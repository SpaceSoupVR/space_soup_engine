use serde::{Deserialize, Serialize};

use crate::scene_cuboid::Color3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightKind {
    Point,
    Spot,
    /// The sun. Parallel rays from infinitely far away.
    ///
    /// `position` and `range` are ignored -- the beam travels along `direction`
    /// and does not attenuate, which is what makes a sun a sun rather than a
    /// very bright bulb a long way up. The renderer treats the FIRST directional
    /// light in a scene as the shadow caster.
    Directional,
}

impl Default for LightKind {
    fn default() -> Self {
        Self::Point
    }
}

fn default_light_color() -> Color3 {
    Color3(255, 255, 255, 255)
}
fn default_light_intensity() -> f32 {
    1.0
}
fn default_light_range() -> f32 {
    5.0
}
pub(crate) fn default_cone_angle() -> f32 {
    45.0
}
fn default_glow_range() -> f32 {
    2.5
}
fn default_glow_intensity() -> f32 {
    8.0
}
fn default_glare_spread() -> f32 {
    0.15
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlareFacesDef {
    #[serde(default = "default_true")]
    pub front: bool,
    #[serde(default = "default_true")]
    pub back: bool,
    #[serde(default = "default_true")]
    pub left: bool,
    #[serde(default = "default_true")]
    pub right: bool,
    #[serde(default = "default_true")]
    pub top: bool,
    #[serde(default = "default_true")]
    pub bottom: bool,
}

impl Default for GlareFacesDef {
    fn default() -> Self {
        Self {
            front: true,
            back: true,
            left: true,
            right: true,
            top: true,
            bottom: true,
        }
    }
}

/// Where a light is evaluated: once, into a lightmap, or every frame.
///
/// This is the setting that decides whether a level can afford realistic
/// interior lighting on a headset, and it exists because a light must be one or
/// the other. Evaluated in both places it is counted TWICE -- the bake adds its
/// contribution to the texture and the shader adds it again -- so a room with
/// baked lamps would come out at double brightness, which reads as "the bake is
/// too bright" rather than as double counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LightMode {
    /// Uploaded to the GPU and shaded every frame.
    ///
    /// The default, because it is what every light did before baking existed
    /// and changing that silently would relight every scene already authored.
    /// Costs a light slot; casts a real-time shadow only if it is the sun or
    /// the first shadow-casting spot.
    #[default]
    Realtime,
    /// Computed once into the lightmap and never uploaded.
    ///
    /// Free at runtime and correctly occluded -- including by the walls of the
    /// room it is in, which no real-time point light can manage. Cannot move,
    /// and changing it needs a rebake.
    Baked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightDef {
    #[serde(default)]
    pub kind: LightKind,
    /// Baked once or shaded every frame. See [`LightMode`].
    #[serde(default)]
    pub mode: LightMode,
    #[serde(default = "default_light_color")]
    pub color: Color3,
    #[serde(default = "default_light_intensity")]
    pub intensity: f32,
    #[serde(default = "default_light_range")]
    pub range: f32,
    #[serde(default = "default_cone_angle")]
    pub cone_angle_deg: f32,
    #[serde(default = "default_glow_range")]
    pub glow_range: f32,
    #[serde(default = "default_glow_intensity")]
    pub glow_intensity: f32,
    #[serde(default = "default_true")]
    pub glow_enabled: bool,
    #[serde(default)]
    pub glare_faces: GlareFacesDef,
    #[serde(default = "default_glare_spread")]
    pub glare_spread: f32,
}

impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::default(),
            mode: LightMode::default(),
            color: default_light_color(),
            intensity: default_light_intensity(),
            range: default_light_range(),
            cone_angle_deg: default_cone_angle(),
            glow_range: default_glow_range(),
            glow_intensity: default_glow_intensity(),
            glow_enabled: default_true(),
            glare_faces: GlareFacesDef::default(),
            glare_spread: default_glare_spread(),
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn a_light_defaults_to_realtime() {
        // Every light behaved this way before baking existed. Flipping the
        // default would silently relight every scene already authored, and the
        // change would look like a rendering regression rather than a new
        // feature nobody opted into.
        assert_eq!(LightDef::default().mode, LightMode::Realtime);
        let from_json: LightDef = serde_json::from_str("{}").unwrap();
        assert_eq!(from_json.mode, LightMode::Realtime);
    }

    #[test]
    fn a_scene_written_before_modes_existed_still_loads() {
        // The field is #[serde(default)], so an older scene file has no `mode`
        // at all and must not fail to parse.
        let old: LightDef = serde_json::from_str(
            r#"{"kind":"Point","intensity":5,"range":15}"#,
        ).unwrap();
        assert_eq!(old.mode, LightMode::Realtime);
        assert_eq!(old.intensity, 5.0);
    }

    #[test]
    fn the_mode_survives_a_round_trip() {
        let mut l = LightDef::default();
        l.mode = LightMode::Baked;
        let back: LightDef = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back.mode, LightMode::Baked);
    }
}
