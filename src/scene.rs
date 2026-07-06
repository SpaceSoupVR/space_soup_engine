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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: f32,
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
    pub color: Option<Color3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPointDef {
    pub name: String,
    #[serde(default)]
    pub kind: GripKind,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
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

    #[test]
    fn pistol_and_slide_are_kinematic_not_physics_driven() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/scenes/lobby.json");
        let scene = Scene::load(&path).expect("lobby.json should parse");

        let slide = scene
            .find_object("glock_slide")
            .expect("glock_slide object should exist");
        assert!(
            slide.slider_joint.is_none(),
            "glock_slide is now a fixed decoration riding along with the pistol's kinematic \
             grab — it should not carry a PhysX rail joint",
        );
        assert!(
            slide.rigid_body.is_none(),
            "glock_slide should have no rigid_body — it's grabbed via the pistol's script, not PhysX",
        );

        let pistol = scene
            .find_object("pistol")
            .expect("pistol object should exist");
        assert!(
            pistol.rigid_body.is_none(),
            "pistol should have no rigid_body — it's picked up via kinematic grab_at_joint",
        );
        let script = pistol.script.as_deref().unwrap_or("");
        assert!(script.contains("grab_at_joint(\"pistol\""));
        assert!(
            script.contains("grab_at_joint(\"glock_slide\""),
            "pistol's on_grab should also kinematically attach glock_slide so the slide follows \
             the frame rigidly",
        );

        assert_eq!(
            pistol.mesh.as_ref().map(|m| m.path.as_str()),
            Some("models/glock_frame.glb"),
            "pistol's mesh should point at the frame-only split, not the original combined model — \
             otherwise the slide's geometry renders twice (once fixed, once as its own moving object)",
        );
    }
}
