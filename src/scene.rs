use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::events::Hand;

pub use crate::scene_animation::{
    Animation, AnimationBinding, BindingScope, Easing, Keyframe, PartAnimationDef, PartDriver,
    PlayMode, BINDING_BUTTONS,
};
pub use crate::scene_cuboid::{
    distance_to_oriented_box, Color3, CuboidDef, CuboidShape, CuboidStyle, MeshRef,
};
pub use crate::scene_env::{
    LaserDef, ParticleEmitterDef, SoundSourceDef, SpawnPointDef, TeleportalDef,
};
pub use crate::scene_light::{GlareFacesDef, LightDef, LightKind};
pub use crate::scene_physics::{BodyMode, ColliderShape, RigidBodyDef};
pub use crate::scene_rig::{
    GripKind, GripPointDef, GripPoseDef, RigAttachmentDef, SliderJointDef, SocketDef,
    TerrainColliderDef,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub part_animations: Vec<PartAnimationDef>,

    /// Parts of this object's model that are not drawn.
    ///
    /// State, not an animation channel. A keyframe interpolates, and "half
    /// visible" is either meaningless or a fade nobody asked for; worse, part
    /// animations keep only each channel's LAST keyframe, so a hide-then-show
    /// timeline cannot even be represented in the file. Visibility is a thing a
    /// part IS, so it lives on the object and is changed by events.
    ///
    /// The case that needs it: a magazine model carrying both a loaded and an
    /// empty magazine as separate parts, where exactly one should be drawn.
    ///
    /// Rendering already knows how to do this -- the same joint-exclusion path
    /// that hides your own avatar's head from your view.
    #[serde(default)]
    pub hidden_parts: Vec<String>,

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
    pub sockets: Vec<SocketDef>,

    #[serde(default)]
    pub slider_joint: Option<SliderJointDef>,

    #[serde(default)]
    pub terrain_collider: Option<TerrainColliderDef>,

    #[serde(default)]
    pub lights: Vec<LightDef>,

    #[serde(default)]
    pub sound: Option<SoundSourceDef>,

    #[serde(default)]
    pub particle_emitter: Option<ParticleEmitterDef>,

    #[serde(default)]
    pub laser: Option<LaserDef>,

    #[serde(default)]
    pub spawn_point: Option<SpawnPointDef>,

    #[serde(default)]
    pub teleportal: Option<TeleportalDef>,
}

impl GameObject {
    pub fn find_animation(&self, name: &str) -> Option<&Animation> {
        self.animations.iter().find(|a| a.name == name)
    }

    pub fn grip_point(&self, name: &str) -> Option<&GripPointDef> {
        self.grip_points.iter().find(|p| p.name == name)
    }

    pub fn socket(&self, name: &str) -> Option<&SocketDef> {
        self.sockets.iter().find(|s| s.name == name)
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
    include!("scene_tests_grip_pose.rs");
}

#[cfg(test)]
mod grip_points_authoring_test {
    use super::*;
    include!("scene_tests_grip_points.rs");
}

#[cfg(test)]
mod cuboid_geometry_test {
    include!("scene_tests_cuboid_geom.rs");
}

#[cfg(test)]
mod slider_joint_authoring_test {
    use super::*;
    include!("scene_tests_slider_joint.rs");
}

#[cfg(test)]
mod dedupe_object_ids_test {
    use super::*;
    include!("scene_tests_dedupe.rs");
}

#[cfg(test)]
mod particle_and_laser_scene_test {
    use super::*;
    include!("scene_tests_particle_laser.rs");
}
