use anyhow::{Context, Result};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::events::Hand;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color3(pub u8, pub u8, pub u8, pub u8);

impl Default for Color3 {
    fn default() -> Self {
        Self(220, 60, 60, 255)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CuboidStyle {
    Solid,
    Wireframe,
    SolidAndWire,
}

impl Default for CuboidStyle {
    fn default() -> Self {
        Self::Solid
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuboidDef {
    #[serde(default)]
    pub position: Vec3,
    #[serde(default = "default_half_size")]
    pub half_size: Vec3,
    #[serde(default = "default_rotation")]
    pub rotation: Quat,
    #[serde(default)]
    pub color: Color3,
    #[serde(default)]
    pub wire_color: Color3,
    #[serde(default)]
    pub style: CuboidStyle,
    /// 0.0 = not reflective, 1.0 = fully mirror-blended. Approximated via
    /// screen-space reflections (see space_soup/src/renderer/ssr.rs) -- only
    /// cuboid objects support this (mesh/GLB objects already use all 4 of
    /// wgpu's default max bind groups).
    #[serde(default)]
    pub reflectivity: f32,
}

fn default_half_size() -> Vec3 {
    Vec3::splat(0.5)
}
fn default_rotation() -> Quat {
    Quat::IDENTITY
}

impl Default for CuboidDef {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            half_size: default_half_size(),
            rotation: Quat::IDENTITY,
            color: Color3::default(),
            wire_color: Color3(200, 200, 255, 255),
            style: CuboidStyle::default(),
            reflectivity: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshRef {
    pub path: String,
    #[serde(default = "default_mesh_scale")]
    pub scale: Vec3,
    #[serde(default = "default_mesh_rotation")]
    pub rotation_offset: Quat,
}

fn default_mesh_scale() -> Vec3 {
    Vec3::ONE
}
fn default_mesh_rotation() -> Quat {
    Quat::IDENTITY
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigAttachmentDef {
    pub joint: String,
    #[serde(default)]
    pub offset: [f32; 3],
}

fn identity_quat_arr() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn one_vec3_arr() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPoseDef {
    #[serde(default)]
    pub hand_offset_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub hand_offset_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
}

impl Default for GripPoseDef {
    fn default() -> Self {
        Self {
            hand_offset_pos: [0.0, 0.0, 0.0],
            hand_offset_rot: identity_quat_arr(),
            hand_offset_scale: one_vec3_arr(),
            finger_curl: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GripKind {
    Snap,

    Free,

    Pinch,
}

impl Default for GripKind {
    fn default() -> Self {
        Self::Snap
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GripPointDef {
    pub name: String,
    #[serde(default)]
    pub kind: GripKind,
    #[serde(default)]
    pub hand: crate::events::Hand,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
    #[serde(default)]
    pub grab_range: Option<f32>,
}

fn default_slider_axis() -> [f32; 3] {
    [1.0, 0.0, 0.0]
}
fn default_slider_travel() -> f32 {
    0.02
}
fn default_slider_stiffness() -> f32 {
    400.0
}
fn default_slider_damping() -> f32 {
    20.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliderJointDef {
    pub parent: String,

    #[serde(default = "default_slider_axis")]
    pub axis: [f32; 3],

    #[serde(default = "default_slider_travel")]
    pub travel: f32,

    #[serde(default = "default_slider_stiffness")]
    pub spring_stiffness: f32,
    #[serde(default = "default_slider_damping")]
    pub spring_damping: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TerrainColliderDef {
    #[serde(default)]
    pub node_filter: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigidBodyDef {
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightKind {
    Point,
    Spot,
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
fn default_cone_angle() -> f32 {
    45.0
}
fn default_glow_range() -> f32 {
    // A real light bulb's own housing and immediate surroundings should
    // read as lit even outside a spotlight's narrow cone -- 0.6 barely
    // cleared the fixture itself (confirmed by screenshotting the rendered
    // result: nearby geometry stayed dark), so this needs to reach a couple
    // meters out to actually brighten neighboring objects/floor.
    2.5
}
fn default_glow_intensity() -> f32 {
    // 90 gave a strong, clearly visible bleed onto surfaces ~1.5m away, but
    // also blew the fixture's own nearby geometry (e.g. the crossbar between
    // the two work_light_stand lamp heads, mere centimeters from the light)
    // out to solid white -- an omnidirectional point light's falloff scales
    // with 1/distance^2, so any intensity strong enough to read clearly at
    // range will overexpose anything sitting right next to it. Pixel-swept
    // this down until the near-field blowout was gone (confirmed via
    // screenshot: 0 fully-white-saturated pixels on that crossbar, down from
    // ~10% of the sampled region at 90) -- the medium-range bleed is more
    // modest as a result, but not blowing out the fixture itself matters more.
    8.0
}
fn default_glare_spread() -> f32 {
    0.15
}
fn default_true() -> bool {
    true
}

/// Which of the 6 directions (relative to the light's own facing --
/// front/back along its aim, the rest perpendicular to that) show a small
/// bulb-icon marker in the editor/preview viewport. All default to visible
/// (so the fixture reads as an obvious lit bulb from any angle); toggling
/// one off is how you say e.g. "no marker visible from directly behind this
/// fixture". Purely a web-editor visual concern, like glare_spread below --
/// not read by collect_render_lights or by quest_app/space_soup at all.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightDef {
    #[serde(default)]
    pub kind: LightKind,
    #[serde(default = "default_light_color")]
    pub color: Color3,
    #[serde(default = "default_light_intensity")]
    pub intensity: f32,
    #[serde(default = "default_light_range")]
    pub range: f32,
    #[serde(default = "default_cone_angle")]
    pub cone_angle_deg: f32,
    /// A real light bulb bleeds a soft glow onto its own housing and
    /// immediate surroundings in every direction, not just down whatever
    /// narrow beam/cone it's aimed at -- a spotlight alone leaves the
    /// fixture itself and anything near it looking unlit. This is the range
    /// (meters) of a small omnidirectional fill light created alongside the
    /// main one to cover that -- real light hitting nearby geometry, not a
    /// visible glow sprite (see glare_faces/glare_spread below for the
    /// actual bulb-icon markers).
    #[serde(default = "default_glow_range")]
    pub glow_range: f32,
    /// Intensity (same WGSL-pipeline units as `intensity`) of that local
    /// fill light.
    #[serde(default = "default_glow_intensity")]
    pub glow_intensity: f32,
    /// Whether the local fill light (glow_range/glow_intensity) is on at
    /// all -- like glare_faces/glare_spread, this is a web-editor-only
    /// preview concern (not read by collect_render_lights or by
    /// quest_app/space_soup), so a fixture can be previewed with just its
    /// spot/point beam and no near-field bleed.
    #[serde(default = "default_true")]
    pub glow_enabled: bool,
    /// Which of the 6 directions show a bulb-icon marker -- see
    /// GlareFacesDef.
    #[serde(default)]
    pub glare_faces: GlareFacesDef,
    /// How far (meters) each enabled marker sits from the light's own
    /// position, along its own direction -- clears the fixture's own opaque
    /// housing mesh (a marker sitting exactly at the light's position gets
    /// depth-occluded by it, confirmed by screenshotting the rendered
    /// result) and is what actually spreads the 6 markers into visually
    /// separate icons instead of one at the center.
    #[serde(default = "default_glare_spread")]
    pub glare_spread: f32,
}

impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::default(),
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
    /// How much a particle grows over its life: final_size = particle_size
    /// * (1 + size_growth) at the end of its lifetime (0.0 = constant size,
    /// 1.0 = doubles, 2.0 = triples) -- real smoke/dust expands as it
    /// disperses; a flat particle size reads as rain/embers instead.
    /// Defaults to 0 so every existing emitter's look is unchanged.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameObject {
    pub id: String,

    #[serde(default)]
    pub cuboid: CuboidDef,

    #[serde(default)]
    pub mesh: Option<MeshRef>,

    #[serde(default)]
    pub is_trigger: bool,

    #[serde(default)]
    pub hidden: bool,

    #[serde(default)]
    pub script: Option<String>,

    #[serde(default)]
    pub animations: Vec<Animation>,

    #[serde(default)]
    pub animation_bindings: Vec<AnimationBinding>,

    #[serde(default)]
    pub rig_attachment: Option<RigAttachmentDef>,

    #[serde(default, rename = "grip_pose", skip_serializing_if = "Option::is_none")]
    pub grip_pose_legacy: Option<GripPoseDef>,

    #[serde(default)]
    pub grip_pose_left: Option<GripPoseDef>,

    #[serde(default)]
    pub grip_pose_right: Option<GripPoseDef>,

    #[serde(default)]
    pub rigid_body: Option<RigidBodyDef>,

    #[serde(default)]
    pub grip_points: Vec<GripPointDef>,

    #[serde(default)]
    pub slider_joint: Option<SliderJointDef>,

    #[serde(default)]
    pub terrain_collider: Option<TerrainColliderDef>,

    #[serde(default)]
    pub light: Option<LightDef>,

    #[serde(default)]
    pub sound: Option<SoundSourceDef>,

    #[serde(default)]
    pub particle_emitter: Option<ParticleEmitterDef>,

    #[serde(default)]
    pub laser: Option<LaserDef>,
}

impl GameObject {
    pub fn find_animation(&self, name: &str) -> Option<&Animation> {
        self.animations.iter().find(|a| a.name == name)
    }

    pub fn grip_point(&self, name: &str) -> Option<&GripPointDef> {
        self.grip_points.iter().find(|p| p.name == name)
    }

    pub fn grip_pose(&self, hand: Hand) -> Option<&GripPoseDef> {
        match hand {
            Hand::Left => self.grip_pose_left.as_ref(),
            Hand::Right => self.grip_pose_right.as_ref(),
        }
    }

    pub fn grip_pose_mut(&mut self, hand: Hand) -> &mut Option<GripPoseDef> {
        match hand {
            Hand::Left => &mut self.grip_pose_left,
            Hand::Right => &mut self.grip_pose_right,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    #[serde(default)]
    pub objects: Vec<GameObject>,
}

impl Scene {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read scene {}", path.display()))?;
        let mut scene: Scene = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse scene {}", path.display()))?;
        for obj in &mut scene.objects {
            if let Some(legacy) = obj.grip_pose_legacy.take() {
                if obj.grip_pose_left.is_none() {
                    obj.grip_pose_left = Some(legacy.clone());
                }
                if obj.grip_pose_right.is_none() {
                    obj.grip_pose_right = Some(legacy);
                }
            }
        }
        dedupe_object_ids(&mut scene.objects, &path.display().to_string());
        Ok(scene)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)
            .with_context(|| format!("failed to write scene {}", path.display()))?;
        Ok(())
    }

    pub fn find_object(&self, id: &str) -> Option<&GameObject> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn find_object_mut(&mut self, id: &str) -> Option<&mut GameObject> {
        self.objects.iter_mut().find(|o| o.id == id)
    }
}

fn dedupe_object_ids(objects: &mut [GameObject], source: &str) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for obj in objects.iter_mut() {
        if seen.insert(obj.id.clone()) {
            continue;
        }
        let stem = obj.id.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_');
        let stem = if stem.is_empty() { obj.id.as_str() } else { stem };
        let mut n = 2;
        let new_id = loop {
            let candidate = format!("{stem}_{n}");
            if !seen.contains(&candidate) {
                break candidate;
            }
            n += 1;
        };
        log::warn!(
            "scene {source}: duplicate object id '{}' — renamed to '{new_id}'",
            obj.id
        );
        seen.insert(new_id.clone());
        obj.id = new_id;
    }
}

#[cfg(test)]
mod grip_pose_migration_test {
    use super::*;

    #[test]
    fn legacy_grip_pose_migrates_to_both_hands() {
        let json = r#"{
            "name": "test",
            "objects": [{
                "id": "obj1",
                "grip_pose": {
                    "hand_offset_pos": [0.1, 0.2, 0.3],
                    "hand_offset_rot": [0.0, 0.0, 0.0, 1.0],
                    "finger_curl": {"index1": 0.5}
                }
            }]
        }"#;
        let tmp = std::env::temp_dir().join("grip_pose_migration_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let obj = &scene.objects[0];
        assert!(
            obj.grip_pose_legacy.is_none(),
            "legacy field should be cleared after migration"
        );
        let left = obj
            .grip_pose_left
            .as_ref()
            .expect("left should be populated");
        let right = obj
            .grip_pose_right
            .as_ref()
            .expect("right should be populated");
        assert_eq!(left.hand_offset_pos, [0.1, 0.2, 0.3]);
        assert_eq!(right.hand_offset_pos, [0.1, 0.2, 0.3]);
        assert_eq!(left.finger_curl.get("index1"), Some(&0.5));

        let out_path = std::env::temp_dir().join("grip_pose_migration_test_out.json");
        scene.save(&out_path).unwrap();
        let saved = std::fs::read_to_string(&out_path).unwrap();
        std::fs::remove_file(&out_path).ok();
        assert!(saved.contains("grip_pose_left"));
        assert!(saved.contains("grip_pose_right"));
        assert!(!saved.contains("\"grip_pose\":"));
    }
}

#[cfg(test)]
mod dedupe_object_ids_test {
    use super::*;

    #[test]
    fn duplicate_ids_get_renamed_to_stay_unique() {
        let json = r#"{
            "name": "test",
            "objects": [
                {"id": "cloud", "cuboid": {"position": [0.0, 0.0, 0.0]}},
                {"id": "cloud", "cuboid": {"position": [1.0, 0.0, 0.0]}},
                {"id": "cloud", "cuboid": {"position": [2.0, 0.0, 0.0]}}
            ]
        }"#;
        let tmp = std::env::temp_dir().join("dedupe_object_ids_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["cloud", "cloud_2", "cloud_3"]);

        for id in &ids {
            assert!(scene.find_object(id).is_some());
        }
    }

    #[test]
    fn duplicate_of_an_already_numbered_id_skips_past_existing_ones() {
        let json = r#"{
            "name": "test",
            "objects": [
                {"id": "cloud_2", "cuboid": {"position": [0.0, 0.0, 0.0]}},
                {"id": "cloud_2", "cuboid": {"position": [1.0, 0.0, 0.0]}}
            ]
        }"#;
        let tmp = std::env::temp_dir().join("dedupe_object_ids_numbered_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["cloud_2", "cloud_3"]);
    }
}

#[cfg(test)]
mod particle_and_laser_scene_test {
    use super::*;

    #[test]
    fn particle_emitters_and_lasers_load_from_lobby_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/scenes/lobby.json");
        let scene = Scene::load(&path).expect("lobby.json should parse");

        let smoke = scene
            .find_object("smoke_grenade")
            .expect("smoke_grenade exists");
        assert!(smoke.particle_emitter.is_some());

        let green = scene.find_object("laser_green").expect("laser_green exists");
        assert!(green.laser.is_some());

        let red = scene.find_object("laser_red").expect("laser_red exists");
        assert!(red.laser.is_some());
    }
}
