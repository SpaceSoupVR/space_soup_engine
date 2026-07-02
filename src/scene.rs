use glam::{Vec3, Quat};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::path::Path;
use anyhow::{Result, Context};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color3(pub u8, pub u8, pub u8, pub u8);

impl Default for Color3 {
    fn default() -> Self { Self(220, 60, 60, 255) }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CuboidStyle {
    Solid,
    Wireframe,
    SolidAndWire,
}

impl Default for CuboidStyle {
    fn default() -> Self { Self::Solid }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuboidDef {
    #[serde(default)]
    pub position:   Vec3,
    #[serde(default = "default_half_size")]
    pub half_size:  Vec3,
    #[serde(default = "default_rotation")]
    pub rotation:   Quat,
    #[serde(default)]
    pub color:      Color3,
    #[serde(default)]
    pub wire_color: Color3,
    #[serde(default)]
    pub style:      CuboidStyle,
}

fn default_half_size() -> Vec3 { Vec3::splat(0.5) }
fn default_rotation() -> Quat { Quat::IDENTITY }

impl Default for CuboidDef {
    fn default() -> Self {
        Self {
            position:   Vec3::ZERO,
            half_size:  default_half_size(),
            rotation:   Quat::IDENTITY,
            color:      Color3::default(),
            wire_color: Color3(200, 200, 255, 255),
            style:      CuboidStyle::default(),
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

fn default_mesh_scale() -> Vec3 { Vec3::ONE }
fn default_mesh_rotation() -> Quat { Quat::IDENTITY }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Default for Easing {
    fn default() -> Self { Self::Linear }
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear    => t,
            Easing::EaseIn    => t * t,
            Easing::EaseOut   => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 { 2.0 * t * t }
                else       { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keyframe {
    pub t:        f32,
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale:    Option<Vec3>,
    pub color:    Option<Color3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Animation {
    pub name:      String,
    pub keyframes: Vec<Keyframe>,
    #[serde(default)]
    pub easing:    Easing,
    #[serde(default)]
    pub looping:   bool,
}

impl Animation {
    pub fn duration(&self) -> f32 {
        self.keyframes
            .iter()
            .map(|k| k.t)
            .fold(0.0_f32, f32::max)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigAttachmentDef {
    pub joint: String,
    #[serde(default)]
    pub offset: [f32; 3],
}

fn identity_quat_arr() -> [f32; 4] { [0.0, 0.0, 0.0, 1.0] }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPoseDef {
    #[serde(default)]
    pub hand_offset_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub hand_offset_rot: [f32; 4],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
}

impl Default for GripPoseDef {
    fn default() -> Self {
        Self {
            hand_offset_pos: [0.0, 0.0, 0.0],
            hand_offset_rot: identity_quat_arr(),
            finger_curl: HashMap::new(),
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

    #[serde(default)]
    pub grip_pose: Option<GripPoseDef>,
}

impl GameObject {
    pub fn find_animation(&self, name: &str) -> Option<&Animation> {
        self.animations.iter().find(|a| a.name == name)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scene {
    pub name:    String,
    #[serde(default)]
    pub objects: Vec<GameObject>,
}

impl Scene {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read scene {}", path.display()))?;
        let scene: Scene = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse scene {}", path.display()))?;
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
