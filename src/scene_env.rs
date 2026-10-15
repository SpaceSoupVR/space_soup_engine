use serde::{Deserialize, Serialize};

use crate::scene_cuboid::Color3;
use crate::scene_light::default_cone_angle;

fn default_volume() -> f32 {
    1.0
}
fn default_pitch() -> f32 {
    1.0
}
fn default_min_distance() -> f32 {
    1.0
}
fn default_max_distance() -> f32 {
    10.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundSourceDef {
    pub clip: String,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_pitch")]
    pub pitch: f32,
    #[serde(default = "default_min_distance")]
    pub min_distance: f32,
    #[serde(default = "default_max_distance")]
    pub max_distance: f32,
    #[serde(default)]
    pub looping: bool,
    #[serde(default)]
    pub autoplay: bool,
    #[serde(default)]
    pub directional: bool,
    #[serde(default = "default_cone_angle")]
    pub cone_angle_deg: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpawnPointDef {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeleportalDef {
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub target_scene: Option<String>,
}

fn default_particle_size() -> f32 {
    0.03
}
fn default_spawn_rate() -> f32 {
    5.0
}
fn default_particle_color() -> Color3 {
    Color3(255, 255, 255, 200)
}
fn default_particle_lifetime() -> f32 {
    2.0
}
fn default_particle_speed() -> f32 {
    0.3
}
fn default_spread_deg() -> f32 {
    15.0
}
fn default_size_growth() -> f32 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleEmitterDef {
    #[serde(default = "default_particle_size")]
    pub particle_size: f32,
    #[serde(default = "default_spawn_rate")]
    pub spawn_rate: f32,
    #[serde(default = "default_particle_color")]
    pub color: Color3,
    #[serde(default = "default_particle_lifetime")]
    pub lifetime: f32,
    #[serde(default = "default_particle_speed")]
    pub speed: f32,
    #[serde(default = "default_spread_deg")]
    pub spread_deg: f32,
    #[serde(default = "default_size_growth")]
    pub size_growth: f32,
}

impl Default for ParticleEmitterDef {
    fn default() -> Self {
        Self {
            particle_size: default_particle_size(),
            spawn_rate: default_spawn_rate(),
            color: default_particle_color(),
            lifetime: default_particle_lifetime(),
            speed: default_particle_speed(),
            spread_deg: default_spread_deg(),
            size_growth: default_size_growth(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKindDef {

    #[default]
    Fire,

    Smoke,

    Embers,

    Dust,

    Coals,
}

fn default_effect_scale() -> f32 {
    1.0
}
fn default_effect_rate() -> f32 {
    1.0
}
fn default_effect_tint() -> Color3 {
    Color3(255, 255, 255, 255)
}
fn effect_one() -> f32 {
    1.0
}
fn is_effect_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_effect_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_effect_seed_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDef {
    #[serde(default)]
    pub kind: EffectKindDef,

    #[serde(default = "default_effect_scale")]
    pub scale: f32,

    #[serde(default = "default_effect_rate")]
    pub rate: f32,

    #[serde(default = "default_effect_tint")]
    pub tint: Color3,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub intensity: f32,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub size_variation: f32,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub life_variation: f32,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub speed_variation: f32,

    #[serde(default, skip_serializing_if = "is_effect_zero")]
    pub temperature_variation: f32,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub flicker: f32,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub flicker_rate: f32,

    #[serde(default = "effect_one", skip_serializing_if = "is_effect_one")]
    pub turbulence: f32,

    #[serde(default, skip_serializing_if = "is_effect_seed_zero")]
    pub seed: u32,
}

impl Default for EffectDef {
    fn default() -> Self {
        Self {
            kind: EffectKindDef::default(),
            scale: default_effect_scale(),
            rate: default_effect_rate(),
            tint: default_effect_tint(),
            intensity: 1.0,
            size_variation: 1.0,
            life_variation: 1.0,
            speed_variation: 1.0,
            temperature_variation: 0.0,
            flicker: 1.0,
            flicker_rate: 1.0,
            turbulence: 1.0,
            seed: 0,
        }
    }
}

fn default_laser_color() -> Color3 {
    Color3(255, 0, 0, 255)
}
fn default_laser_max_distance() -> f32 {
    20.0
}
fn default_beam_width() -> f32 {
    0.02
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaserDef {
    #[serde(default = "default_laser_color")]
    pub color: Color3,
    #[serde(default = "default_laser_max_distance")]
    pub max_distance: f32,
    #[serde(default = "default_beam_width")]
    pub beam_width: f32,
}

impl Default for LaserDef {
    fn default() -> Self {
        Self {
            color: default_laser_color(),
            max_distance: default_laser_max_distance(),
            beam_width: default_beam_width(),
        }
    }
}

#[cfg(test)]
mod effect_tests {
    use crate::scene::{EffectDef, EffectKindDef, GameObject};

    #[test]
    fn an_effect_needs_only_its_kind() {
        let obj: GameObject = serde_json::from_str(r#"{"id": "hearth", "effect": {"kind": "smoke"}}"#).expect("parse");
        let e = obj.effect.expect("an effect");
        assert_eq!(e.kind, EffectKindDef::Smoke);
        assert_eq!((e.scale, e.rate), (1.0, 1.0));
        assert_eq!((e.tint.0, e.tint.1, e.tint.2), (255, 255, 255));
    }

    #[test]
    fn an_effect_is_written_only_when_there_is_one() {
        let plain = serde_json::to_string(&GameObject { id: "wall".into(), ..Default::default() }).expect("serialize");
        assert!(!plain.contains("effect"), "{plain}");
        let fire = GameObject {
            id: "hearth".into(),
            effect: Some(EffectDef { kind: EffectKindDef::Embers, scale: 1.5, ..Default::default() }),
            ..Default::default()
        };
        let json = serde_json::to_string(&fire).expect("serialize");
        assert!(json.contains(r#""kind":"embers""#), "{json}");
        let back: GameObject = serde_json::from_str(&json).expect("parse");
        assert_eq!(back.effect.map(|e| (e.kind, e.scale)), Some((EffectKindDef::Embers, 1.5)));
    }

    #[test]
    fn an_effects_variation_is_written_only_where_it_differs() {
        let obj: GameObject = serde_json::from_str(r#"{"id": "hearth", "effect": {"kind": "fire", "scale": 1.5, "rate": 1, "tint": [255, 255, 255, 255]}}"#).expect("parse");
        let e = obj.effect.clone().expect("an effect");
        assert_eq!((e.intensity, e.size_variation, e.life_variation, e.speed_variation), (1.0, 1.0, 1.0, 1.0));
        assert_eq!((e.temperature_variation, e.flicker, e.flicker_rate, e.turbulence, e.seed), (0.0, 1.0, 1.0, 1.0, 0));
        let json = serde_json::to_string(&obj.effect).expect("serialize");
        assert_eq!(json, r#"{"kind":"fire","scale":1.5,"rate":1.0,"tint":[255,255,255,255]}"#);
        let named: GameObject = serde_json::from_str(r#"{"id": "h", "effect": {"kind": "fire", "intensity": 1, "seed": 0, "flicker": 1.0}}"#).expect("parse");
        assert!(!serde_json::to_string(&named.effect).unwrap().contains("intensity"));
        let set = EffectDef { intensity: 1.4, seed: 12, temperature_variation: 0.3, ..Default::default() };
        let json = serde_json::to_string(&set).expect("serialize");
        assert!(json.contains(r#""intensity":1.4"#) && json.contains(r#""seed":12"#) && json.contains(r#""temperature_variation":0.3"#), "{json}");
        assert!(!json.contains("flicker") && !json.contains("turbulence"), "{json}");
        let back: EffectDef = serde_json::from_str(&json).expect("parse");
        assert_eq!((back.intensity, back.seed, back.temperature_variation), (1.4, 12, 0.3));
    }
}
