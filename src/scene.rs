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
    EffectDef, EffectKindDef, LaserDef, ParticleEmitterDef, SoundSourceDef, SpawnPointDef,
    TeleportalDef,
};
pub use crate::scene_light::{GlareFacesDef, LightDef, LightKind, LightMode};
pub use crate::scene_physics::{BodyMode, ColliderShape, RigidBodyDef};
pub use crate::scene_rig::{
    GripKind, GripPointDef, GripPoseDef, RigAttachmentDef, SliderJointDef, SocketDef,
    TerrainColliderDef,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameObject {

    pub id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,

    #[serde(default)]
    pub cuboid: CuboidDef,

    #[serde(default)]
    pub mesh: Option<MeshRef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<crate::brush::BrushDef>,

    #[serde(default)]
    pub is_trigger: bool,

    #[serde(default)]
    pub hidden: bool,

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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<crate::door::DoorDef>,

    #[serde(default)]
    pub terrain_collider: Option<TerrainColliderDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walk_area: Option<crate::walk_area::WalkAreaDef>,

    #[serde(default)]
    pub lights: Vec<LightDef>,

    #[serde(default)]
    pub sound: Option<SoundSourceDef>,

    #[serde(default)]
    pub particle_emitter: Option<ParticleEmitterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectDef>,

    #[serde(default)]
    pub laser: Option<LaserDef>,

    #[serde(default)]
    pub spawn_point: Option<SpawnPointDef>,

    #[serde(default)]
    pub teleportal: Option<TeleportalDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prop: Option<PropRef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<SelectorDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_panel: Option<MenuPanelDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_probe: Option<crate::reflection_probe::ReflectionProbeDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakable: Option<BreakableDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_volume: Option<crate::trigger_volume::TriggerVolumeDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectorDef {
    pub set: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuPanelDef {
    pub menu: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PropRef {
    pub path: String,
}

impl GameObject {

    pub fn is_solid_brush(&self) -> bool {
        self.brush.is_some()
            && self
                .solid
                .unwrap_or(self.trigger_volume.is_none() && !self.is_trigger)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostDef {

    #[serde(default = "default_exposure")]
    pub exposure: f32,
    #[serde(default)]
    pub tonemap: ToneMapDef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToneMapDef {

    #[default]
    Aces,

    None,
}

fn default_exposure() -> f32 {
    1.0
}

impl Default for PostDef {
    fn default() -> Self {
        Self { exposure: default_exposure(), tonemap: ToneMapDef::default() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,

    #[serde(default)]
    pub post: PostDef,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<TerrainDef>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub water: Vec<crate::water::WaterDef>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weather: Vec<crate::weather::WeatherDef>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scatter: Vec<ScatterLayer>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sky: Option<SkyDef>,

    #[serde(default)]
    pub objects: Vec<GameObject>,
}

fn default_sky_intensity() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkyDef {

    pub id: String,

    #[serde(default = "default_sky_intensity")]
    pub intensity: f32,

    #[serde(default)]
    pub rotation_deg: f32,

    #[serde(default = "default_true")]
    pub show_background: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_of_day: Option<TimeOfDayDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDayDef {

    #[serde(default = "default_start_hour")]
    pub start_hour: f32,

    #[serde(default)]
    pub day_minutes: f32,
    #[serde(default = "default_latitude")]
    pub latitude_deg: f32,

    #[serde(default = "default_day_of_year")]
    pub day_of_year: f32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub north_deg: Option<f32>,

    #[serde(default = "default_moon_phase")]
    pub moon_phase: f32,

    #[serde(default = "default_one")]
    pub star_density: f32,

    #[serde(default)]
    pub light_pollution: f32,

    #[serde(default = "default_one")]
    pub turbidity: f32,
}

fn default_start_hour() -> f32 {
    15.0
}
fn default_latitude() -> f32 {
    40.0
}
fn default_day_of_year() -> f32 {
    172.0
}
fn default_moon_phase() -> f32 {
    0.45
}
fn default_one() -> f32 {
    1.0
}

impl Default for TimeOfDayDef {
    fn default() -> Self {
        serde_json::from_str("{}").expect("every field has a default")
    }
}

impl TimeOfDayDef {

    pub fn params(&self, photographed_sun: Option<[f32; 3]>) -> space_soup_sky::time_of_day::TimeOfDayParams {
        let p = space_soup_sky::time_of_day::TimeOfDayParams {
            start_hour: self.start_hour,
            day_minutes: self.day_minutes.max(0.0),
            latitude_deg: self.latitude_deg,
            day_of_year: self.day_of_year,
            north_deg: self.north_deg.unwrap_or(180.0),
            moon_phase: self.moon_phase,
            star_density: self.star_density.max(0.0),
            light_pollution: self.light_pollution.max(0.0),
            turbidity: self.turbidity.max(0.05),
        };
        match (self.north_deg, photographed_sun) {
            (None, Some(sun)) => p.with_sun_azimuth_of(sun),
            _ => p,
        }
    }
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

    pub fn resolve_world_transforms(&mut self) {
        let index_of: std::collections::HashMap<&str, usize> = self
            .objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.uuid.as_deref().map(|u| (u, i)))
            .collect();

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

                    parents[i] = None;
                    roots.push(i);
                }
                None => roots.push(i),
            }
        }

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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BreakableDef {

    pub health: f32,

    #[serde(default)]
    pub stages: Vec<DamageStage>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DamageStage {

    pub at: f32,

    #[serde(default)]
    pub hidden_parts: Vec<String>,

    #[serde(default = "default_true")]
    pub solid: bool,

    #[serde(default)]
    pub removed: bool,
}

fn default_true() -> bool {
    true
}

impl BreakableDef {

    pub fn stage_for(&self, damage: f32) -> Option<&DamageStage> {
        if self.health <= 0.0 {

            return self.stages.last();
        }
        let fraction = (damage / self.health).clamp(0.0, 1.0);
        self.stages
            .iter()
            .filter(|s| fraction >= s.at)
            .max_by(|a, b| a.at.total_cmp(&b.at))
    }

    pub fn is_solid_at(&self, damage: f32) -> bool {
        self.stage_for(damage)
            .map_or(true, |s| s.solid && !s.removed)
    }

    pub fn hidden_parts_at<'a>(&'a self, damage: f32, authored: &'a [String]) -> &'a [String] {
        match self.stage_for(damage) {
            Some(stage) => &stage.hidden_parts,
            None => authored,
        }
    }
}

#[cfg(test)]
mod breakable_tests {
    use super::*;

    fn wall() -> BreakableDef {
        BreakableDef {
            health: 100.0,
            stages: vec![
                DamageStage {
                    at: 0.4,
                    hidden_parts: vec!["intact_render".into()],
                    solid: true,
                    ..Default::default()
                },
                DamageStage {
                    at: 0.8,
                    hidden_parts: vec!["intact_render".into(), "cracked_render".into()],
                    solid: false,
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn an_undamaged_object_is_in_no_stage_and_keeps_its_authored_parts() {
        let authored = vec!["scaffold".to_string()];
        let w = wall();
        assert!(w.stage_for(0.0).is_none());
        assert_eq!(w.hidden_parts_at(0.0, &authored), authored.as_slice());
        assert!(w.is_solid_at(0.0));
    }

    #[test]
    fn crossing_a_threshold_takes_that_stage() {
        let w = wall();
        assert!(w.stage_for(39.0).is_none(), "just under the first threshold");
        assert_eq!(w.stage_for(40.0).unwrap().at, 0.4);
        assert_eq!(w.stage_for(79.0).unwrap().at, 0.4);
        assert_eq!(w.stage_for(80.0).unwrap().at, 0.8);
    }

    #[test]
    fn one_large_hit_reaches_the_same_stage_as_many_small_ones() {
        let w = wall();
        let one_shell = w.stage_for(95.0).unwrap();

        let mut damage = 0.0;
        for _ in 0..19 {
            damage += 5.0;
        }
        let many_rounds = w.stage_for(damage).unwrap();

        assert_eq!(one_shell.at, many_rounds.at);
        assert_eq!(one_shell.hidden_parts, many_rounds.hidden_parts);
    }

    #[test]
    fn a_stage_is_a_state_not_a_delta() {
        let w = wall();
        let authored = vec!["scaffold".to_string()];
        let straight_to_breached = w.hidden_parts_at(100.0, &authored).to_vec();

        let _ = w.hidden_parts_at(45.0, &authored);
        let _ = w.hidden_parts_at(85.0, &authored);
        let gradually = w.hidden_parts_at(100.0, &authored).to_vec();

        assert_eq!(straight_to_breached, gradually);
        assert_eq!(gradually, vec!["intact_render", "cracked_render"]);
    }

    #[test]
    fn a_breached_stage_stops_blocking() {
        let w = wall();
        assert!(w.is_solid_at(50.0), "cracked but still a wall");
        assert!(!w.is_solid_at(85.0), "breached");
    }

    #[test]
    fn damage_past_full_health_stays_at_the_last_stage() {
        let w = wall();
        assert_eq!(w.stage_for(1_000_000.0).unwrap().at, 0.8);
    }

    #[test]
    fn zero_health_breaks_immediately_rather_than_never() {
        let w = BreakableDef { health: 0.0, ..wall() };
        assert_eq!(w.stage_for(0.0).unwrap().at, 0.8);
        assert!(!w.is_solid_at(0.0));
    }

    #[test]
    fn a_breakable_with_no_stages_never_changes() {
        let w = BreakableDef { health: 100.0, stages: vec![] };
        assert!(w.stage_for(999.0).is_none());
        assert!(w.is_solid_at(999.0));
    }

    #[test]
    fn stages_resolve_by_threshold_not_by_list_order() {
        let w = BreakableDef {
            health: 100.0,
            stages: vec![
                DamageStage { at: 0.9, hidden_parts: vec!["gone".into()], solid: false, ..Default::default() },
                DamageStage { at: 0.3, hidden_parts: vec!["chipped".into()], solid: true, ..Default::default() },
            ],
        };
        assert_eq!(w.stage_for(95.0).unwrap().hidden_parts, vec!["gone"]);
        assert_eq!(w.stage_for(50.0).unwrap().hidden_parts, vec!["chipped"]);
    }

    #[test]
    fn the_component_is_skipped_when_absent() {
        let obj = GameObject { id: "wall".into(), ..Default::default() };
        let json = serde_json::to_string(&obj).expect("serialize");
        assert!(!json.contains("breakable"), "absent component must not be written: {json}");
    }

    #[test]
    fn it_round_trips_through_json() {
        let obj = GameObject {
            id: "wall".into(),
            breakable: Some(wall()),
            ..Default::default()
        };
        let json = serde_json::to_string(&obj).expect("serialize");
        let back: GameObject = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.breakable, Some(wall()));
    }

    #[test]
    fn solid_defaults_to_true_when_a_stage_omits_it() {
        let stage: DamageStage =
            serde_json::from_str(r#"{"at":0.5,"hidden_parts":[]}"#).expect("parse");
        assert!(stage.solid);
    }
}

#[cfg(test)]
mod breakable_editor_parity {
    use super::*;

    const FROM_EDITOR: &str = r#"{
        "health": 100,
        "stages": [
            { "at": 0.4, "hidden_parts": [], "solid": true },
            { "at": 0.8, "hidden_parts": [], "solid": true }
        ]
    }"#;

    #[test]
    fn the_engine_reads_what_the_editor_writes() {
        let def: BreakableDef = serde_json::from_str(FROM_EDITOR).expect("parse editor output");
        assert_eq!(def.health, 100.0);
        assert_eq!(def.stages.len(), 2);
        assert_eq!(def.stages[0].at, 0.4);
        assert!(def.stages[1].solid);

        assert!(def.stage_for(39.0).is_none());
        assert_eq!(def.stage_for(40.0).unwrap().at, 0.4);
        assert_eq!(def.stage_for(80.0).unwrap().at, 0.8);
    }

    #[test]
    fn an_integer_health_parses_as_a_float() {
        let def: BreakableDef =
            serde_json::from_str(r#"{"health": 250, "stages": []}"#).expect("parse");
        assert_eq!(def.health, 250.0);
    }
}
