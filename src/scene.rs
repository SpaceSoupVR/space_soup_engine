use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::events::Hand;
pub use crate::scatter::ScatterLayer;
pub use crate::terrain::{TerrainDef, TerrainKind};

pub use crate::scene_animation::{
    Animation, AnimationBinding, BindingScope, CompareOp, Condition, Easing, Keyframe,
    PartAnimationDef, PartDriver, PartTrigger, PartTriggerAction, PlayMode, BINDING_BUTTONS,
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
    /// The human name. Displayed in the editor, and the handle every script
    /// uses: `move_object("m4a1_bolt", ..)`, `get_object_x`, `play_animation`,
    /// `raycast_hit_object`. Renameable, and NOT what structure hangs off --
    /// see `uuid`.
    pub id: String,

    /// Stable identity, assigned once and never changed.
    ///
    /// Exists because `parent` has to point at something a rename cannot break.
    /// Structure used to be inferred from the id string -- the editor's
    /// `familyRoot()` split ids on `_` and asked whether a shorter one existed,
    /// so `m4a1_mag` was a child of `m4a1` by spelling alone. Renaming an object
    /// silently reparented it, and two compound props could not share a part
    /// name.
    ///
    /// Deliberately NOT used for script references. The whole scripting surface
    /// is name-based and there are four structural cross-object references in
    /// the entire lobby, so migrating those to uuids would break every
    /// hand-written script to fix four fields. Names stay the scripting
    /// contract; uuids carry structure.
    ///
    /// `None` on a file written before this existed, which loads as a flat
    /// scene exactly as it did before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,

    /// The `uuid` of this object's transform parent, if it has one.
    ///
    /// When set, `cuboid.position` and `cuboid.rotation` are **parent-relative**
    /// as stored. `Scene::load` leaves them that way so a load/save round trip
    /// is lossless; `resolve_world_transforms` composes them into world space,
    /// and the runtime calls it before anything reads a transform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,

    #[serde(default)]
    pub cuboid: CuboidDef,

    #[serde(default)]
    pub mesh: Option<MeshRef>,

    #[serde(default)]
    pub is_trigger: bool,

    #[serde(default)]
    pub hidden: bool,

    /// Free-form labels for organising a scene: "cover", "lighting", "audio",
    /// "blockout", "pass-2".
    ///
    /// Organisation rather than behaviour, which is why it is not an authorable
    /// component: nothing in the runtime reads a tag today. It lives on
    /// GameObject rather than in an editor sidecar because it is the natural
    /// seed for data layers -- streaming a district, enabling a lighting pass,
    /// turning a whole gameplay set on for a mode -- and a sidecar keyed by
    /// object id would break on exactly the rename that `uuid` exists to
    /// survive.
    ///
    /// Skipped when empty so existing scenes gain nothing on their next save.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

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

    /// The scene's ground, if it has authored terrain.
    ///
    /// Optional and skipped when absent, so every existing scene is unchanged.
    /// A scene without terrain is exactly what the lobby is today: geometry
    /// made of objects, with `terrain_collider` marking some of it walkable.
    /// That path is untouched -- this is the ground you sculpt, not the props
    /// standing on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<TerrainDef>,

    /// Procedural placement layers -- trees, grass, rocks.
    ///
    /// Parameters, not output. Five thousand scattered trees are a handful of
    /// strokes plus a seed here, and are resolved to instances at load; storing
    /// them as objects would put them in `objects`, which is the one thing that
    /// would make this format unusable at map scale.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scatter: Vec<ScatterLayer>,

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

    /// Compose parent-relative transforms into world space, in place.
    ///
    /// Stored transforms are parent-relative whenever `parent` is set, because
    /// that is what makes moving a compound prop one edit rather than N. But
    /// eleven modules read `cuboid.position` expecting world space, so rather
    /// than teach all of them about hierarchy, the runtime flattens once at
    /// load and they carry on unchanged.
    ///
    /// The tradeoff that buys: moving a parent AT RUNTIME (a script calling
    /// `move_object` on it) does not drag its children, because after this call
    /// there is no hierarchy left in memory to propagate through. Static level
    /// structure is what this is for. Runtime propagation needs the composition
    /// to live in the render/physics path instead, which is a much larger
    /// change and is not needed until something authored moves as a group.
    ///
    /// Unknown parents and cycles are dropped with a warning rather than
    /// failing the load: a scene that is half-broken should still open in the
    /// editor so it can be fixed there.
    pub fn resolve_world_transforms(&mut self) {
        let index_of: std::collections::HashMap<&str, usize> = self
            .objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.uuid.as_deref().map(|u| (u, i)))
            .collect();

        // Parent index per object, and the roots to walk down from.
        let parents: Vec<Option<usize>> = self
            .objects
            .iter()
            .map(|o| {
                let parent_uuid = o.parent.as_deref()?;
                match index_of.get(parent_uuid) {
                    Some(&i) => Some(i),
                    None => {
                        log::warn!(
                            "scene '{}': object '{}' names a parent {parent_uuid} \
                             that is not in this scene -- treating it as a root",
                            self.name,
                            o.id
                        );
                        None
                    }
                }
            })
            .collect();

        let mut parents = parents;
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); self.objects.len()];
        let mut roots: Vec<usize> = Vec::new();
        for i in 0..self.objects.len() {
            match parents[i] {
                Some(p) if p != i => children[p].push(i),
                Some(_) => {
                    log::warn!(
                        "scene '{}': object '{}' is its own parent -- ignoring",
                        self.name,
                        self.objects[i].id
                    );
                    // Clearing the link, not just treating it as a root: the
                    // composition step below reads `parents[i]`, so leaving it
                    // set made the object compose against itself and double its
                    // own offset.
                    parents[i] = None;
                    roots.push(i);
                }
                None => roots.push(i),
            }
        }

        // Iterative so a deep hierarchy cannot blow the stack, and so anything
        // left unvisited is provably in a cycle.
        let mut visited = vec![false; self.objects.len()];
        let mut stack: Vec<usize> = roots;
        while let Some(i) = stack.pop() {
            if visited[i] {
                continue;
            }
            visited[i] = true;

            if let Some(p) = parents[i] {
                let (parent_pos, parent_rot) = {
                    let parent = &self.objects[p];
                    (parent.cuboid.position, parent.cuboid.rotation)
                };
                let child = &mut self.objects[i];
                child.cuboid.position = parent_pos + parent_rot * child.cuboid.position;
                child.cuboid.rotation = (parent_rot * child.cuboid.rotation).normalize();
            }

            stack.extend(children[i].iter().copied());
        }

        for (i, seen) in visited.iter().enumerate() {
            if !seen {
                log::warn!(
                    "scene '{}': object '{}' is in a parent cycle -- its transform \
                     was left as authored",
                    self.name,
                    self.objects[i].id
                );
            }
        }
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
