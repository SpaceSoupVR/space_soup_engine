use anyhow::Result;
use glam::{Quat, Vec3};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use space_soup_protocol::PlayerId;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::animation::AnimationPlayer;
use crate::attach::{Attachment, AttachmentTable};
use crate::audio::SoundEngine;
use crate::events::InputFrame;
use crate::locomotion::{Locomotion, LocomotionInput, TeleportTarget};
use crate::manifest::Manifest;
use crate::physics::CollisionTracker;
use crate::rig::{JointId, PlayerRig};
use crate::rigid_physics::PhysicsWorld;
use crate::scene::{Color3, CuboidShape, CuboidStyle, LightKind, Scene};
use crate::script::ScriptHost;

#[derive(Debug, Clone, Default)]
pub struct PlayerFrameInput {
    pub rig: PlayerRig,
    pub input: InputFrame,
    pub locomotion_input: LocomotionInput,
    pub teleport_target: Option<TeleportTarget>,

    pub client_offset: Option<Vec3>,
    pub client_yaw: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderCuboid {
    pub id: String,
    pub position: Vec3,
    pub half_size: Vec3,
    pub rotation: Quat,
    pub color: Color3,
    pub wire_color: Color3,
    pub style: CuboidStyle,
    pub reflectivity: f32,
    pub shape: CuboidShape,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderMesh {
    pub id: String,
    pub path: String,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,

    pub emissive_drive: f32,
    pub manual_part_blends: HashMap<String, f32>,

    pub hidden_parts: Vec<String>,

    pub disabled_clips: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderLight {
    pub id: String,
    pub position: Vec3,
    pub direction: Vec3,
    pub kind: LightKind,
    pub color: Color3,
    pub intensity: f32,
    pub range: f32,
    pub cone_angle_deg: f32,

    pub inner_cone_angle_deg: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderParticleEmitter {
    pub id: String,
    pub position: Vec3,
    pub direction: Vec3,
    pub particle_size: f32,
    pub spawn_rate: f32,
    pub color: Color3,
    pub lifetime: f32,
    pub speed: f32,
    pub spread_deg: f32,
    pub size_growth: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderParticleBurst {
    pub id: String,
    pub position: Vec3,
    pub direction: Vec3,
    pub color: Color3,
    pub count: u32,
    pub speed: f32,
    pub spread_deg: f32,
    pub particle_size: f32,
    pub lifetime: f32,
    pub elapsed: f32,
}

pub(crate) struct ParticleBurst {
    pub(crate) id: String,
    pub(crate) position: Vec3,
    pub(crate) direction: Vec3,
    pub(crate) color: Color3,
    pub(crate) count: u32,
    pub(crate) speed: f32,
    pub(crate) spread_deg: f32,
    pub(crate) particle_size: f32,
    pub(crate) lifetime: f32,
    pub(crate) elapsed: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderLaser {
    pub id: String,
    pub origin: Vec3,
    pub direction: Vec3,
    pub end: Vec3,
    pub color: Color3,
    pub beam_width: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundState {
    pub object_id: String,
    pub clip: String,
    pub position: Vec3,
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    pub min_distance: f32,
    pub max_distance: f32,
}

pub struct GameRuntime {
    pub(crate) game_dir: PathBuf,
    manifest: Manifest,
    pub(crate) scene: Scene,

    pub(crate) script_host: ScriptHost,
    pub(crate) players: HashMap<String, AnimationPlayer>,
    pub(crate) anim_queues: HashMap<String, Vec<String>>,
    pub(crate) collisions: CollisionTracker,
    pub(crate) teleport_collisions: CollisionTracker,
    pub(crate) teleport_disarmed: HashSet<(PlayerId, String)>,

    pub(crate) trigger_volumes: Vec<(String, crate::trigger_volume::VolumeShape)>,
    pub(crate) volume_occupants: HashMap<String, HashSet<PlayerId>>,
    pub(crate) volume_fired: HashSet<String>,
    pub(crate) rigid_physics: PhysicsWorld,
    /// The scene's navmesh, if one was baked for it (`navigation`).
    pub(crate) nav: Option<crate::navigation::SceneNav>,
    pub(crate) sound_engine: SoundEngine,

    pub rigs: HashMap<PlayerId, PlayerRig>,
    pub attachments: AttachmentTable,
    pub locomotions: HashMap<PlayerId, Locomotion>,

    pub(crate) pending_scene_change: Option<String>,
    pub(crate) sound_play_requests: HashSet<String>,
    pub(crate) sound_stop_requests: HashSet<String>,
    pub(crate) manual_part_blends: HashMap<String, HashMap<String, f32>>,

    pub(crate) damage: crate::damage::DamageLedger,

    pub(crate) part_transforms: HashMap<String, HashMap<String, (Vec3, Quat)>>,

    pub(crate) prev_part_blends: HashMap<String, HashMap<String, f32>>,

    pub(crate) latched_triggers: std::collections::HashSet<(String, String, usize)>,
    pub(crate) particle_bursts: Vec<ParticleBurst>,
    pub(crate) next_particle_burst_id: u64,
    pub(crate) next_detached_id: u64,
    pub(crate) socket_attachments: HashMap<String, (String, String)>,
    /// The bodies that went into the water lately, oldest first, and the
    /// number the next one takes. See [`Self::water_entries`].
    pub(crate) water_entries: Vec<crate::water::RecentWaterEntry>,
    pub(crate) next_water_entry: u64,
}

impl GameRuntime {
    pub fn load(game_dir: &Path) -> Result<Self> {
        let manifest = Manifest::load(game_dir)?;
        let scene_path = manifest.entry_scene_path(game_dir);
        let mut scene = Scene::load(&scene_path)?;
        scene.resolve_world_transforms();

        let mut rt = Self {
            game_dir: game_dir.to_path_buf(),
            manifest,
            scene,
            script_host: ScriptHost::new(),
            players: HashMap::new(),
            anim_queues: HashMap::new(),
            collisions: CollisionTracker::new(),
            teleport_collisions: CollisionTracker::new(),
            teleport_disarmed: HashSet::new(),
            trigger_volumes: Vec::new(),
            volume_occupants: HashMap::new(),
            volume_fired: HashSet::new(),
            rigid_physics: PhysicsWorld::new(),
            nav: None,
            sound_engine: SoundEngine::new(),
            rigs: HashMap::new(),
            attachments: AttachmentTable::new(),
            locomotions: HashMap::new(),
            pending_scene_change: None,
            sound_play_requests: HashSet::new(),
            sound_stop_requests: HashSet::new(),
            manual_part_blends: HashMap::new(),
            damage: crate::damage::DamageLedger::new(),
            part_transforms: HashMap::new(),
            prev_part_blends: HashMap::new(),
            latched_triggers: std::collections::HashSet::new(),
            particle_bursts: Vec::new(),
            next_particle_burst_id: 0,
            next_detached_id: 0,
            socket_attachments: HashMap::new(),
            water_entries: Vec::new(),
            next_water_entry: 0,
        };

        rt.compile_scripts();
        rt.setup_scene_attachments();

        rt.trigger_volumes = Self::resolve_trigger_volumes(&rt.scene);
        rt.rigid_physics.rebuild(&rt.scene, &rt.game_dir);
        let entry = rt.manifest.entry_scene.clone();
        rt.nav = rt.load_nav(&entry);
        info!(
            "GameRuntime: loaded scene '{}' with {} objects",
            rt.scene.name,
            rt.scene.objects.len()
        );

        Ok(rt)
    }

    /// Scene `scene`'s navmesh, if one was baked: logged either way, and never baked here (a large
    /// level takes seconds; `examples/bake_navmesh.rs` bakes one).
    fn load_nav(&self, scene: &str) -> Option<crate::navigation::SceneNav> {
        match crate::navigation::SceneNav::load(&self.game_dir, scene, Some(&self.rigid_physics)) {
            Ok(Some(nav)) => {
                if nav.stale {
                    log::warn!("navigation: {scene}'s colliders have changed since its navmesh was baked; bake it again");
                }
                info!("navigation: {scene}: {} polygons at {} m cells", nav.info.polygons, nav.info.cell_size);
                Some(nav)
            }
            Ok(None) => None,
            Err(e) => {
                log::warn!("navigation: {scene}: {e}");
                None
            }
        }
    }

    /// The scene's navmesh, if it has one.
    pub fn nav(&self) -> Option<&crate::navigation::SceneNav> {
        self.nav.as_ref()
    }

    pub fn render_lists(&self) -> (Vec<RenderCuboid>, Vec<RenderMesh>, Vec<RenderLight>) {
        (
            self.collect_render_cuboids(),
            self.collect_render_meshes(),
            self.collect_render_lights(),
        )
    }

    fn setup_scene_attachments(&mut self) {
        let defs: Vec<(String, String, [f32; 3])> = self
            .scene
            .objects
            .iter()
            .filter_map(|o| {
                let att = o.rig_attachment.as_ref()?;
                Some((o.id.clone(), att.joint.clone(), att.offset))
            })
            .collect();

        for (entity_id, joint_name, offset) in defs {
            match JointId::from_name(&joint_name) {
                Some(joint_id) => {
                    let offset_world = Vec3::from(offset);
                    let att = if offset_world == Vec3::ZERO {
                        Attachment::rigid(joint_id)
                    } else {
                        Attachment::with_offset(joint_id, offset_world, Quat::IDENTITY)
                    };
                    self.attachments.attach(&entity_id, PlayerId::local(), att);
                    info!("setup_scene_attachments: '{entity_id}' → '{joint_name}'");
                }
                None => {
                    warn!("setup_scene_attachments: unknown joint '{joint_name}' for '{entity_id}'");
                }
            }
        }
    }

    fn compile_scripts(&mut self) {
        for entity in &self.scene.objects {
            if let Some(src) = &entity.script {
                if let Err(e) = self.script_host.compile(&entity.id, src) {
                    warn!("Failed to compile script for '{}': {e}", entity.id);
                }
            }
        }
    }

    pub fn load_scene(&mut self, scene_name: &str) -> Result<()> {
        let path = Manifest::scene_path(&self.game_dir, scene_name);
        let mut scene = Scene::load(&path)?;
        scene.resolve_world_transforms();

        self.scene = scene;

        self.damage.reset();
        self.players = HashMap::new();
        self.anim_queues = HashMap::new();
        self.collisions = CollisionTracker::new();
        self.teleport_collisions = CollisionTracker::new();
        self.teleport_disarmed = HashSet::new();

        self.trigger_volumes = Self::resolve_trigger_volumes(&self.scene);
        self.volume_occupants = HashMap::new();
        self.volume_fired = HashSet::new();
        self.attachments = AttachmentTable::new();
        self.manual_part_blends = HashMap::new();
        self.particle_bursts = Vec::new();
        self.socket_attachments = HashMap::new();
        self.water_entries.clear();
        self.script_host = ScriptHost::new();
        self.compile_scripts();
        self.setup_scene_attachments();
        self.rigid_physics.rebuild(&self.scene, &self.game_dir);
        self.nav = self.load_nav(scene_name);

        if let Some((position, yaw)) = self.find_spawn_point() {
            for locomotion in self.locomotions.values_mut() {
                locomotion.player_offset = position;
                locomotion.player_yaw = yaw;
            }
        }

        info!("GameRuntime: switched to scene '{scene_name}'");
        Ok(())
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn scene_name(&self) -> &str {
        &self.scene.name
    }

    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    pub fn update(
        &mut self,
        dt: f32,
        inputs: &HashMap<PlayerId, PlayerFrameInput>,
    ) -> (
        Vec<RenderCuboid>,
        Vec<RenderMesh>,
        Vec<RenderLight>,
        Vec<RenderParticleEmitter>,
        Vec<RenderParticleBurst>,
        Vec<RenderLaser>,
        Option<String>,
    ) {
        self.pending_scene_change = None;

        let disconnected: Vec<PlayerId> = self
            .rigs
            .keys()
            .copied()
            .filter(|p| !inputs.contains_key(p))
            .collect();
        for player in disconnected {
            self.rigid_physics.remove_player(player);
            self.attachments.remove_player(player);
            self.rigs.remove(&player);
            self.locomotions.remove(&player);
            self.teleport_disarmed.retain(|(p, _)| *p != player);
        }

        for (&player, frame) in inputs {
            self.rigs.insert(player, frame.rig.clone());
            self.rigid_physics.ensure_player(player);
        }

        let spawn = self.find_spawn_point();

        for (&player, frame) in inputs {
            let rig = self.rigs[&player].clone();
            let locomotion = self
                .locomotions
                .entry(player)
                .or_insert_with(|| Self::new_locomotion_at(spawn));

            if let (Some(off), Some(yaw)) = (frame.client_offset, frame.client_yaw) {
                locomotion.player_offset = off;
                locomotion.player_yaw = yaw;
                continue;
            }

            let collision_start = locomotion.collision_start(None);
            locomotion.update(dt, &frame.locomotion_input, &rig, frame.teleport_target);

            let li = &frame.locomotion_input;
            if li.move_stick.0.abs() > 0.1 || li.move_stick.1.abs() > 0.1 || li.turn_stick_x.abs() > 0.1 {
                log::info!(
                    "SRV LOCO: recv move=({:.2},{:.2}) turn={:.2} dt={:.4} -> raw off=({:.2},{:.2},{:.2}) yaw={:.1}",
                    li.move_stick.0, li.move_stick.1, li.turn_stick_x, dt,
                    locomotion.player_offset.x, locomotion.player_offset.y, locomotion.player_offset.z,
                    locomotion.player_yaw.to_degrees()
                );
            }
            locomotion.apply_collision(&self.rigid_physics, collision_start);
        }

        self.update_animations(dt);
        self.age_particle_bursts(dt);
        self.update_object_position_cache();
        self.apply_attachments();
        self.apply_socket_attachments();
        self.dispatch_collisions();
        self.dispatch_teleportals();
        self.dispatch_trigger_volumes();

        for (&player, frame) in inputs {
            self.update_rig_position_cache(player);
            self.script_host.set_current_player(player);
            self.dispatch_input(&frame.input);
        }

        self.dispatch_update_hook(dt);
        self.apply_script_commands();

        self.rigid_physics.step(dt, &mut self.scene, &self.rigs);
        self.note_water_entries(dt);

        let listener = match inputs.len() {
            1 => inputs.keys().next().map(|&p| self.world_head_transform(p)),
            _ => None,
        };
        self.sound_engine.update(
            &self.game_dir,
            &self.scene.objects,
            &self.sound_play_requests,
            &self.sound_stop_requests,
            listener,
        );
        self.sound_play_requests.clear();
        self.sound_stop_requests.clear();

        let cuboids = self.collect_render_cuboids();
        let meshes = self.collect_render_meshes();
        let lights = self.collect_render_lights();
        let particle_emitters = self.collect_render_particle_emitters();
        let particle_bursts = self.collect_render_particle_bursts();
        let lasers = self.collect_render_lasers();
        (
            cuboids,
            meshes,
            lights,
            particle_emitters,
            particle_bursts,
            lasers,
            self.pending_scene_change.take(),
        )
    }

    /// What physics saw go into the water this step joins what it saw lately;
    /// the rest is a step older, and what is older than is told of is dropped.
    fn note_water_entries(&mut self, dt: f32) {
        for e in &mut self.water_entries {
            e.age += dt;
        }
        self.water_entries.retain(|e| e.age < crate::water::WATER_ENTRY_KEPT);
        for entry in self.rigid_physics.take_water_entries() {
            self.water_entries.push(crate::water::RecentWaterEntry { serial: self.next_water_entry, age: 0.0, entry });
            self.next_water_entry += 1;
        }
        while self.water_entries.len() > crate::water::MAX_ENTRIES_TOLD {
            let least = self
                .water_entries
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.entry.energy().total_cmp(&b.1.entry.energy()))
                .map_or(0, |(i, _)| i);
            self.water_entries.remove(least);
        }
    }

    /// THE BODIES THAT WENT INTO THE WATER in the last
    /// [`crate::water::WATER_ENTRY_KEPT`] seconds, oldest first: each as it
    /// met the surface, with its own number and its age. What a server puts
    /// in every snapshot (`WireWorld::water_entries`) so each client raises
    /// the splash once, from the speed and the mass physics had for it.
    pub fn water_entries(&self) -> &[crate::water::RecentWaterEntry] {
        &self.water_entries
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }
}

impl GameRuntime {

    pub fn apply_damage(
        &mut self,
        object_id: &str,
        amount: f32,
    ) -> Option<crate::damage::StageChange> {
        let entity = self.scene.objects.iter().find(|o| o.id == object_id)?;
        let change = self.damage.apply(entity, amount)?;

        if change.breached() {
            let removed_static = self.rigid_physics.despawn_static(&change.object_id);
            self.rigid_physics.despawn_actor(&change.object_id);
            if !removed_static {
                log::debug!(
                    "breach: '{}' had no static collider to remove",
                    change.object_id
                );
            }
        }

        Some(change)
    }

    pub fn apply_damage_at(
        &mut self,
        point: glam::Vec3,
        radius: f32,
        amount: f32,
    ) -> Vec<crate::damage::StageChange> {

        let targets: Vec<(String, f32)> =
            crate::damage::impact_targets(&self.scene.objects, point, radius)
                .into_iter()
                .map(|t| (self.scene.objects[t.index].id.clone(), t.share))
                .collect();

        targets
            .into_iter()
            .filter_map(|(id, share)| self.apply_damage(&id, amount * share))
            .collect()
    }

    pub fn damage_taken(&self, object_id: &str) -> f32 {
        self.damage.damage_for(object_id)
    }

    pub fn is_solid(&self, object_id: &str) -> bool {
        self.scene
            .objects
            .iter()
            .find(|o| o.id == object_id)
            .is_none_or(|o| self.damage.is_solid(o))
    }

    pub fn reset_damage(&mut self) {
        self.damage.reset();
    }
}

#[cfg(test)]
mod damage_delivery_tests {
    use crate::scene::{BreakableDef, DamageStage, GameObject};

    fn breakable_wall() -> GameObject {
        GameObject {
            id: "wall".into(),
            breakable: Some(BreakableDef {
                health: 100.0,
                stages: vec![
                    DamageStage { at: 0.5, hidden_parts: vec!["intact".into()], solid: true, ..Default::default() },
                    DamageStage {
                        at: 0.9,
                        hidden_parts: vec!["intact".into(), "cracked".into()],
                        solid: false,
                        ..Default::default()
                    },
                ],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn damage_changes_what_the_renderer_is_told_to_hide() {
        let mut ledger = crate::damage::DamageLedger::new();
        let wall = breakable_wall();

        assert!(ledger.hidden_parts_for(&wall).is_empty(), "whole wall draws everything");

        ledger.apply(&wall, 60.0);
        assert_eq!(ledger.hidden_parts_for(&wall), ["intact"]);

        ledger.apply(&wall, 40.0);
        assert_eq!(ledger.hidden_parts_for(&wall), ["intact", "cracked"]);
        assert!(!ledger.is_solid(&wall), "and it stops blocking");
    }
}
