use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color3(pub u8, pub u8, pub u8, pub u8);

impl Default for Color3 {
    fn default() -> Self {
        Self(220, 60, 60, 255)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum CuboidStyle {
    #[default]
    Solid,
    Wireframe,
    SolidAndWire,
}


#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum CuboidShape {
    #[default]
    Box,
    Cylinder,
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

pub fn distance_to_oriented_box(center: Vec3, rotation: Quat, half_size: Vec3, point: Vec3) -> f32 {
    let local = rotation.inverse() * (point - center);
    local.distance(local.clamp(-half_size, half_size))
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
