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
    // Client-authoritative player pose (see WireLocomotionInput in space_soup_protocol):
    // when present, update() adopts it verbatim instead of simulating locomotion.
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
    pub manual_part_blends: HashMap<String, f32>,
    /// Parts of this model that must not be drawn -- see GameObject::hidden_parts.
    pub hidden_parts: Vec<String>,
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

// Fire-and-forget particle event (muzzle flash, impact spark) -- unlike RenderParticleEmitter
// this has no authored persistent state; it's spawned by a script call, ages out, and is
// dropped, so the engine has to track `elapsed` itself instead of deriving it from sim_time.
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
    pub(crate) rigid_physics: PhysicsWorld,
    pub(crate) sound_engine: SoundEngine,

    pub rigs: HashMap<PlayerId, PlayerRig>,
    pub attachments: AttachmentTable,
    pub locomotions: HashMap<PlayerId, Locomotion>,

    pub(crate) pending_scene_change: Option<String>,
    pub(crate) sound_play_requests: HashSet<String>,
    pub(crate) sound_stop_requests: HashSet<String>,
    pub(crate) manual_part_blends: HashMap<String, HashMap<String, f32>>,

    /// Last frame's part blends, so a trigger fires on a CROSSING rather than on
    /// every frame the blend happens to sit past its threshold.
    pub(crate) prev_part_blends: HashMap<String, HashMap<String, f32>>,

    /// Triggers currently latched, as (object, clip, trigger index). A latched
    /// trigger will not fire again until the blend retreats past the hysteresis
    /// band -- a hand held near the threshold jitters across it many times a
    /// second, and an eject that fires on every jitter is unusable.
    pub(crate) latched_triggers: std::collections::HashSet<(String, String, usize)>,
    pub(crate) particle_bursts: Vec<ParticleBurst>,
    pub(crate) next_particle_burst_id: u64,
    pub(crate) next_detached_id: u64,
    pub(crate) socket_attachments: HashMap<String, (String, String)>,
}

impl GameRuntime {
    pub fn load(game_dir: &Path) -> Result<Self> {
        let manifest = Manifest::load(game_dir)?;
        let scene_path = manifest.entry_scene_path(game_dir);
        let scene = Scene::load(&scene_path)?;

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
            rigid_physics: PhysicsWorld::new(),
            sound_engine: SoundEngine::new(),
            rigs: HashMap::new(),
            attachments: AttachmentTable::new(),
            locomotions: HashMap::new(),
            pending_scene_change: None,
            sound_play_requests: HashSet::new(),
            sound_stop_requests: HashSet::new(),
            manual_part_blends: HashMap::new(),
            prev_part_blends: HashMap::new(),
            latched_triggers: std::collections::HashSet::new(),
            particle_bursts: Vec::new(),
            next_particle_burst_id: 0,
            next_detached_id: 0,
            socket_attachments: HashMap::new(),
        };

        rt.compile_scripts();
        rt.setup_scene_attachments();
        rt.rigid_physics.rebuild(&rt.scene, &rt.game_dir);
        info!(
            "GameRuntime: loaded scene '{}' with {} objects",
            rt.scene.name,
            rt.scene.objects.len()
        );

        Ok(rt)
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

        for (obj_id, joint_name, offset) in defs {
            match JointId::from_name(&joint_name) {
                Some(joint_id) => {
                    let offset_vec = Vec3::from(offset);
                    let att = if offset_vec == Vec3::ZERO {
                        Attachment::rigid(joint_id)
                    } else {
                        Attachment::with_offset(joint_id, offset_vec, Quat::IDENTITY)
                    };
                    self.attachments.attach(&obj_id, PlayerId::local(), att);
                    info!("setup_scene_attachments: '{obj_id}' → '{joint_name}'");
                }
                None => {
                    warn!("setup_scene_attachments: unknown joint '{joint_name}' for '{obj_id}'")
                }
            }
        }
    }

    fn compile_scripts(&mut self) {
        for obj in &self.scene.objects {
            if let Some(src) = &obj.script {
                if let Err(e) = self.script_host.compile(&obj.id, src) {
                    warn!("Failed to compile script for '{}': {e}", obj.id);
                }
            }
        }
    }

    pub fn load_scene(&mut self, scene_name: &str) -> Result<()> {
        let path = Manifest::scene_path(&self.game_dir, scene_name);
        let scene = Scene::load(&path)?;

        self.scene = scene;
        self.players = HashMap::new();
        self.anim_queues = HashMap::new();
        self.collisions = CollisionTracker::new();
        self.teleport_collisions = CollisionTracker::new();
        self.teleport_disarmed = HashSet::new();
        self.attachments = AttachmentTable::new();
        self.manual_part_blends = HashMap::new();
        self.particle_bursts = Vec::new();
        self.socket_attachments = HashMap::new();
        self.script_host = ScriptHost::new();
        self.compile_scripts();
        self.setup_scene_attachments();
        self.rigid_physics.rebuild(&self.scene, &self.game_dir);

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

            // Client-authoritative pose: the client already simulated its own movement;
            // adopt it verbatim so other players see this player where the player sees
            // themselves. No re-simulation, wall collision, or ground follow -- those
            // are the client's job now (legacy clients that send no pose still get the
            // full server-side simulation below).
            if let (Some(off), Some(yaw)) = (frame.client_offset, frame.client_yaw) {
                locomotion.player_offset = off;
                locomotion.player_yaw = yaw;
                continue;
            }

            let prev_xz = (locomotion.player_offset.x, locomotion.player_offset.z);
            locomotion.update(dt, &frame.locomotion_input, &rig, frame.teleport_target);
            // SRV LOCO DIAGNOSTIC: what the SERVER received from the client + the raw
            // result of locomotion.update (BEFORE wall/ground physics). recv~0 while the
            // client pushes => wire/deserialize loss; recv nonzero but off/yaw unchanged
            // => update logic; off moves here but broadcast is 0 => wall/ground reverts it.
            let li = &frame.locomotion_input;
            if li.move_stick.0.abs() > 0.1 || li.move_stick.1.abs() > 0.1 || li.turn_stick_x.abs() > 0.1 {
                log::info!(
                    "SRV LOCO: recv move=({:.2},{:.2}) turn={:.2} dt={:.4} -> raw off=({:.2},{:.2},{:.2}) yaw={:.1}",
                    li.move_stick.0, li.move_stick.1, li.turn_stick_x, dt,
                    locomotion.player_offset.x, locomotion.player_offset.y, locomotion.player_offset.z,
                    locomotion.player_yaw.to_degrees()
                );
            }
            locomotion.apply_collision(&self.rigid_physics, prev_xz);
        }

        self.update_animations(dt);
        self.age_particle_bursts(dt);
        self.update_object_position_cache();
        self.apply_attachments();
        self.apply_socket_attachments();
        self.dispatch_collisions();
        self.dispatch_teleportals();

        for (&player, frame) in inputs {
            self.update_rig_position_cache(player);
            self.script_host.set_current_player(player);
            self.dispatch_input(&frame.input);
        }

        self.dispatch_update_hook(dt);
        self.apply_script_commands();

        self.rigid_physics.step(dt, &mut self.scene, &self.rigs);

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

    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }
}
