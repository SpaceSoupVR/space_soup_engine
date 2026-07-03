use glam::{Vec3, Quat};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::path::Path;
use anyhow::{Result, Context};

use crate::events::Hand;

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
fn one_vec3_arr() -> [f32; 3] { [1.0, 1.0, 1.0] }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPoseDef {
    #[serde(default)]
    pub hand_offset_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub hand_offset_rot: [f32; 4],
    /// Visual-only preview scale for the hand mesh — never affects gameplay.
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
    /// Hand fully locks to the point's exact position *and* rotation
    /// (a PhysX fixed joint) — use for a primary grip that should feel
    /// exactly like the authored pose, e.g. a rifle's stock.
    Snap,
    /// Hand's position is followed but rotation is left free (a PhysX
    /// spherical joint) — use for a secondary/support grip that shouldn't
    /// fight a `Snap` grip's rotation on the same object, e.g. a barrel.
    Free,
}

impl Default for GripKind {
    fn default() -> Self { Self::Snap }
}

/// A named point on an object that a hand can grab — replaces the old
/// "capture wherever you happened to touch it" grab (`grab_at_joint`) with
/// an authored, repeatable location. Any number of points may exist on one
/// object, and different hands may hold different points simultaneously
/// (e.g. a rifle's `stock` + `barrel`); PhysX resolves the combined
/// constraint on the object's rigid body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPointDef {
    pub name: String,
    #[serde(default)]
    pub kind: GripKind,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],
    /// Visual-only preview scale for the hand mesh — never affects gameplay.
    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BodyMode {
    /// Never moves; PhysX excludes it from broad-phase re-sorting. Use for
    /// floors/walls/level geometry.
    Static,
    /// Position/rotation are still driven by scripts/animation (read from
    /// `cuboid` each frame and pushed into PhysX as a kinematic target), but
    /// it still physically pushes `Dynamic` bodies around and isn't affected
    /// by gravity or collision response itself.
    Kinematic,
    /// Fully simulated: gravity, mass, collision response. Owns
    /// `cuboid.position`/`.rotation` once created — see `GameRuntime::update`
    /// for the exact ordering against script/animation writes.
    Dynamic,
}

fn default_body_mode() -> BodyMode { BodyMode::Dynamic }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    /// Sized from the object's own `cuboid.half_size`.
    Box,
    Sphere { radius: f32 },
    Capsule { radius: f32, half_height: f32 },
}

impl Default for ColliderShape {
    fn default() -> Self { Self::Box }
}

fn default_friction() -> f32 { 0.5 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigidBodyDef {
    #[serde(default = "default_body_mode")]
    pub mode: BodyMode,
    #[serde(default)]
    pub shape: ColliderShape,
    /// `None` means "calculate from volume × density" (see
    /// `rigid_physics::calculated_mass`) — an explicit author override
    /// otherwise. Ignored for `Static`/`Kinematic`.
    #[serde(default)]
    pub mass: Option<f32>,
    #[serde(default = "default_friction")]
    pub friction: f32,
    #[serde(default)]
    pub restitution: f32,
    /// Initial velocity, `Dynamic` only.
    #[serde(default)]
    pub linear_velocity: [f32; 3],
    /// `Dynamic` only: if set, the body teleports back to its spawn
    /// position/rotation (velocity zeroed) every `respawn_interval`
    /// seconds, regardless of where it's come to rest — a simple way to
    /// make a falling object loop instead of settling permanently.
    #[serde(default)]
    pub respawn_interval: Option<f32>,
    /// Overrides `cuboid.half_size` for the physics collider only (`None`
    /// uses `cuboid.half_size`, same as before) — for `ColliderShape::Box`.
    /// Exists because PhysX's box-box collision becomes unreliable for
    /// off-center contacts against an extreme-aspect-ratio box (confirmed:
    /// a large, paper-thin static floor — a common shape for level
    /// geometry with a thin visual mesh — let dynamic objects tunnel
    /// straight through unless positioned exactly above its center). Give
    /// thin mesh-only floors/walls a thicker `collider_half_size` than
    /// their visual bounds; combine with `collider_offset` to keep the
    /// collider's surface aligned with the visible geometry.
    #[serde(default)]
    pub collider_half_size: Option<[f32; 3]>,
    /// Local offset (from `cuboid.position`) applied to the physics shape
    /// only — lets `collider_half_size` grow in one direction (e.g.
    /// downward, for a floor) without shifting the rendered mesh, which is
    /// still anchored at `cuboid.position` directly.
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

    /// Legacy single, hand-agnostic grip pose. Read on load and migrated
    /// into `grip_pose_left`/`grip_pose_right` (see `Scene::load`) — never
    /// written back out once migrated, so this stays `None` in memory after
    /// load except for the brief window before migration runs.
    #[serde(default, rename = "grip_pose", skip_serializing_if = "Option::is_none")]
    pub grip_pose_legacy: Option<GripPoseDef>,

    #[serde(default)]
    pub grip_pose_left: Option<GripPoseDef>,

    #[serde(default)]
    pub grip_pose_right: Option<GripPoseDef>,

    #[serde(default)]
    pub rigid_body: Option<RigidBodyDef>,

    /// Named grab points — see `GripPointDef`. Additive to (and takes
    /// priority over, when present) the legacy `grip_pose_left`/`_right`.
    #[serde(default)]
    pub grip_points: Vec<GripPointDef>,
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
    pub name:    String,
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
        assert!(obj.grip_pose_legacy.is_none(), "legacy field should be cleared after migration");
        let left = obj.grip_pose_left.as_ref().expect("left should be populated");
        let right = obj.grip_pose_right.as_ref().expect("right should be populated");
        assert_eq!(left.hand_offset_pos, [0.1, 0.2, 0.3]);
        assert_eq!(right.hand_offset_pos, [0.1, 0.2, 0.3]);
        assert_eq!(left.finger_curl.get("index1"), Some(&0.5));

        // Round-trip through save: new schema keys should be written, not the legacy one.
        let out_path = std::env::temp_dir().join("grip_pose_migration_test_out.json");
        scene.save(&out_path).unwrap();
        let saved = std::fs::read_to_string(&out_path).unwrap();
        std::fs::remove_file(&out_path).ok();
        assert!(saved.contains("grip_pose_left"));
        assert!(saved.contains("grip_pose_right"));
        assert!(!saved.contains("\"grip_pose\":"));
    }
}
