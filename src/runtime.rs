use anyhow::Result;
use glam::{Quat, Vec3};
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::animation::{sample, AnimationPlayer};
use crate::attach::{Attachment, AttachmentTable};
use crate::audio::SoundEngine;
use crate::events::{Hand, InputFrame};
use crate::locomotion::{Locomotion, LocomotionInput, LocomotionMode, TeleportTarget};
use crate::manifest::Manifest;
use crate::physics::{Aabb, CollisionEvent, CollisionTracker};
use crate::rig::{JointId, PlayerRig};
use crate::rigid_physics::PhysicsWorld;
use crate::scene::{
    BindingScope, Color3, CuboidStyle, GameObject, GripPointDef, LightKind, MeshRef, PlayMode,
    Scene,
};
use crate::script::{EngineCommand, ScriptHost};

#[derive(Debug, Clone)]
pub struct RenderCuboid {
    pub id: String,
    pub position: Vec3,
    pub half_size: Vec3,
    pub rotation: Quat,
    pub color: Color3,
    pub wire_color: Color3,
    pub style: CuboidStyle,
}

#[derive(Debug, Clone)]
pub struct RenderMesh {
    pub id: String,
    pub path: String,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Debug, Clone)]
pub struct RenderLight {
    pub id: String,
    pub position: Vec3,
    /// Aim direction for `Spot` lights (derived from `cuboid.rotation`); unused for `Point`.
    pub direction: Vec3,
    pub kind: LightKind,
    pub color: Color3,
    pub intensity: f32,
    pub range: f32,
    pub cone_angle_deg: f32,
}

pub struct GameRuntime {
    game_dir: PathBuf,
    manifest: Manifest,
    scene: Scene,

    script_host: ScriptHost,
    players: HashMap<String, AnimationPlayer>,
    /// Sequential-mode animations waiting per object; the front entry starts
    /// when the object's current animation finishes.
    anim_queues: HashMap<String, Vec<String>>,
    collisions: CollisionTracker,
    rigid_physics: PhysicsWorld,
    sound_engine: SoundEngine,

    pub rig: PlayerRig,
    pub attachments: AttachmentTable,
    pub locomotion: Locomotion,

    pending_scene_change: Option<String>,
    sound_play_requests: HashSet<String>,
    sound_stop_requests: HashSet<String>,
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
            rigid_physics: PhysicsWorld::new(),
            sound_engine: SoundEngine::new(),
            rig: PlayerRig::new(),
            attachments: AttachmentTable::new(),
            locomotion: Locomotion::new(LocomotionMode::Smooth),
            pending_scene_change: None,
            sound_play_requests: HashSet::new(),
            sound_stop_requests: HashSet::new(),
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
                    self.attachments.attach(&obj_id, att);
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
        self.attachments = AttachmentTable::new();
        self.script_host = ScriptHost::new();
        self.compile_scripts();
        self.setup_scene_attachments();
        self.rigid_physics.rebuild(&self.scene, &self.game_dir);

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
        input: &InputFrame,
        rig: PlayerRig,
        locomotion_input: &LocomotionInput,
        teleport_target: Option<TeleportTarget>,
    ) -> (Vec<RenderCuboid>, Vec<RenderMesh>, Vec<RenderLight>, Option<String>) {
        self.pending_scene_change = None;
        self.rig = rig;

        let prev_xz = (self.locomotion.player_offset.x, self.locomotion.player_offset.z);
        self.locomotion
            .update(dt, locomotion_input, &self.rig, teleport_target);
        self.apply_ground_follow(prev_xz);

        self.update_animations(dt);
        self.update_object_position_cache();
        self.update_rig_position_cache();
        self.apply_attachments();
        self.dispatch_collisions();
        self.dispatch_input(input);
        self.dispatch_update_hook(dt);
        self.apply_script_commands();

        self.rigid_physics.step(dt, &mut self.scene, &self.rig);

        let (listener_pos, listener_rot) = self.world_head_transform();
        self.sound_engine.update(
            &self.game_dir,
            &self.scene.objects,
            &self.sound_play_requests,
            &self.sound_stop_requests,
            listener_pos,
            listener_rot,
        );
        self.sound_play_requests.clear();
        self.sound_stop_requests.clear();

        let cuboids = self.collect_render_cuboids();
        let meshes = self.collect_render_meshes();
        let lights = self.collect_render_lights();
        (cuboids, meshes, lights, self.pending_scene_change.take())
    }

    /// Snaps the player's height to the ground directly beneath them and blocks horizontal
    /// movement into slopes steeper than `Locomotion::max_climb_angle_deg`.
    fn apply_ground_follow(&mut self, prev_xz: (f32, f32)) {
        if self.locomotion.mode == LocomotionMode::Disabled {
            return;
        }

        let offset = self.locomotion.player_offset;
        let probe_origin = Vec3::new(offset.x, offset.y + 3.0, offset.z);
        let Some((hit_point, normal)) = self.rigid_physics.raycast_down(probe_origin, 50.0) else {
            return;
        };

        let slope_deg = normal.dot(Vec3::Y).clamp(-1.0, 1.0).acos().to_degrees();
        if slope_deg <= self.locomotion.max_climb_angle_deg {
            self.locomotion.player_offset.y = hit_point.y;
        } else {
            self.locomotion.player_offset.x = prev_xz.0;
            self.locomotion.player_offset.z = prev_xz.1;
        }
    }

    pub fn world_head_transform(&self) -> (Vec3, Quat) {
        let head = self.rig.head();
        (head.position, head.rotation)
    }

    fn update_animations(&mut self, dt: f32) {
        let mut finished: Vec<String> = Vec::new();

        let Self { scene, players, .. } = self;
        for (obj_id, player) in players.iter_mut() {
            let Some(obj) = scene.find_object(obj_id) else {
                continue;
            };
            let Some(anim) = obj.find_animation(&player.anim_name) else {
                continue;
            };
            let duration = anim.duration();
            player.tick(dt, duration);
            if player.finished {
                finished.push(obj_id.clone());
            }
        }

        let samples: Vec<(String, crate::animation::Sample)> = self
            .players
            .iter()
            .filter_map(|(obj_id, player)| {
                let obj = self.scene.find_object(obj_id)?;
                let anim = obj.find_animation(&player.anim_name)?;
                Some((obj_id.clone(), sample(anim, player.elapsed)))
            })
            .collect();

        for (obj_id, s) in samples {
            if let Some(obj_mut) = self.scene.find_object_mut(&obj_id) {
                if let Some(p) = s.position {
                    obj_mut.cuboid.position = p;
                }
                if let Some(r) = s.rotation {
                    obj_mut.cuboid.rotation = r;
                }
                if let Some(sc) = s.scale {
                    obj_mut.cuboid.half_size = sc;
                }
                if let Some(c) = s.color {
                    obj_mut.cuboid.color = c;
                }
            }
        }

        for id in finished {
            self.players.remove(&id);
            // Sequential bindings: start the next queued animation, if any.
            let next = self
                .anim_queues
                .get_mut(&id)
                .and_then(|q| (!q.is_empty()).then(|| q.remove(0)));
            if let Some(anim_name) = next {
                self.play_animation(&id, &anim_name);
            }
        }
    }

    fn play_animation(&mut self, obj_id: &str, anim_name: &str) {
        let Some(obj) = self.scene.find_object(obj_id) else {
            warn!("play_animation: unknown object '{obj_id}'");
            return;
        };
        let Some(anim) = obj.find_animation(anim_name) else {
            warn!("play_animation: object '{obj_id}' has no animation '{anim_name}'");
            return;
        };
        self.players
            .insert(obj_id.to_string(), AnimationPlayer::new(anim));
    }

    fn stop_animation(&mut self, obj_id: &str) {
        self.players.remove(obj_id);
        self.anim_queues.remove(obj_id);
    }

    fn update_object_position_cache(&self) {
        for obj in &self.scene.objects {
            let p = obj.cuboid.position;
            self.script_host.set_object_position(&obj.id, p.x, p.y, p.z);
        }
    }

    fn update_rig_position_cache(&self) {
        let head = self.rig.head();
        self.script_host.set_rig_position(
            "head",
            head.position.x,
            head.position.y,
            head.position.z,
        );

        for hand in [Hand::Left, Hand::Right] {
            let grip = self.rig.hand_grip(hand);
            let aim = self.rig.hand_aim(hand);
            let prefix = hand.as_str();
            self.script_host.set_rig_position(
                &format!("{prefix}_grip"),
                grip.position.x,
                grip.position.y,
                grip.position.z,
            );
            self.script_host.set_rig_position(
                &format!("{prefix}_aim"),
                aim.position.x,
                aim.position.y,
                aim.position.z,
            );
        }
    }

    fn apply_attachments(&mut self) {
        let results = self.attachments.resolve_all_with_visibility(&self.rig);
        for (obj_id, maybe_tf) in results {
            if let Some(obj) = self.scene.find_object_mut(&obj_id) {
                match maybe_tf {
                    Some(tf) => {
                        obj.cuboid.position = tf.position;
                        obj.cuboid.rotation = tf.rotation;
                    }

                    None => obj.hidden = true,
                }
            }
        }
    }

    fn dispatch_collisions(&mut self) {
        let bodies: Vec<(String, Aabb)> = self
            .scene
            .objects
            .iter()
            .filter(|o| o.rigid_body.is_none())
            .map(|o| {
                let aabb = Aabb::from_center_half(o.cuboid.position, o.cuboid.half_size);
                (o.id.clone(), aabb)
            })
            .collect();

        let events = self.collisions.update(&bodies);

        for event in events {
            match event {
                CollisionEvent::Enter(a, b) => {
                    let _ = self
                        .script_host
                        .call(&a, "on_collision_enter", (b.clone(),));
                    let _ = self.script_host.call(&b, "on_collision_enter", (a,));
                }
                CollisionEvent::Exit(a, b) => {
                    let _ = self.script_host.call(&a, "on_collision_exit", (b.clone(),));
                    let _ = self.script_host.call(&b, "on_collision_exit", (a,));
                }
            }
        }
    }

    fn dispatch_input(&mut self, input: &InputFrame) {
        for (id, hand) in &input.pointed {
            let _ = self
                .script_host
                .call(id, "on_point", (hand.as_str().to_string(),));
        }
        for (id, hand, point) in &input.grabbed {
            let _ =
                self.script_host
                    .call(id, "on_grab", (hand.as_str().to_string(), point.clone()));
        }
        for (id, hand) in &input.released {
            let _ = self
                .script_host
                .call(id, "on_release", (hand.as_str().to_string(),));
        }
        for press in &input.button_presses {
            if let Some(id) = &press.object_id {
                let _ = self
                    .script_host
                    .call(id, "on_press", (press.button.clone(),));
            }
        }
        self.dispatch_animation_bindings(input);
    }

    /// Fires `animation_bindings` matching this frame's button presses.
    /// Contextual bindings require the press to carry the bound object's id
    /// (i.e. the player is holding it); global bindings fire from anywhere.
    fn dispatch_animation_bindings(&mut self, input: &InputFrame) {
        let mut to_play: Vec<(String, String, PlayMode)> = Vec::new();
        for press in &input.button_presses {
            for obj in &self.scene.objects {
                for binding in &obj.animation_bindings {
                    if binding.button != press.button || binding.animation.is_empty() {
                        continue;
                    }
                    let in_scope = match binding.scope {
                        BindingScope::GlobalAnywhere => true,
                        BindingScope::ContextualHold => {
                            press.object_id.as_deref() == Some(obj.id.as_str())
                        }
                    };
                    if in_scope {
                        to_play.push((obj.id.clone(), binding.animation.clone(), binding.play_mode));
                    }
                }
            }
        }
        for (obj_id, anim, mode) in to_play {
            match mode {
                PlayMode::Simultaneous => self.play_animation(&obj_id, &anim),
                PlayMode::Sequential => {
                    if self.players.contains_key(&obj_id) {
                        self.anim_queues.entry(obj_id).or_default().push(anim);
                    } else {
                        self.play_animation(&obj_id, &anim);
                    }
                }
            }
        }
    }

    fn dispatch_update_hook(&self, dt: f32) {
        for obj in &self.scene.objects {
            if self.script_host.has_script(&obj.id) {
                let _ = self.script_host.call(&obj.id, "on_update", (dt as f64,));
            }
        }
    }

    fn apply_script_commands(&mut self) {
        let commands = self.script_host.drain_commands();

        for cmd in commands {
            match cmd {
                EngineCommand::MoveObject { id, x, y, z } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        o.cuboid.position = Vec3::new(x, y, z);
                    }
                }
                EngineCommand::RotateObject { id, x, y, z, w } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        o.cuboid.rotation = Quat::from_xyzw(x, y, z, w);
                    }
                }
                EngineCommand::ScaleObject { id, x, y, z } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        o.cuboid.half_size = Vec3::new(x, y, z);
                    }
                }
                EngineCommand::SetColor { id, r, g, b, a } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        o.cuboid.color = Color3(r, g, b, a);
                    }
                }
                EngineCommand::PlayAnim { id, anim } => {
                    self.play_animation(&id, &anim);
                }
                EngineCommand::StopAnim { id } => {
                    self.stop_animation(&id);
                }
                EngineCommand::ChangeScene { scene } => {
                    self.pending_scene_change = Some(scene);
                }
                EngineCommand::DestroyObject { id } => {
                    self.scene.objects.retain(|o| o.id != id);
                    self.players.remove(&id);
                    self.anim_queues.remove(&id);
                    self.attachments.detach(&id);
                }
                EngineCommand::AttachToJoint {
                    id,
                    joint,
                    offset_x,
                    offset_y,
                    offset_z,
                } => match JointId::from_name(&joint) {
                    Some(joint_id) => {
                        let att = Attachment::with_offset(
                            joint_id,
                            Vec3::new(offset_x, offset_y, offset_z),
                            Quat::IDENTITY,
                        );
                        self.attachments.attach(&id, att);
                    }
                    None => warn!("attach_to_joint: unknown joint name '{joint}'"),
                },
                EngineCommand::GrabAtJoint { id, joint } => match JointId::from_name(&joint) {
                    Some(joint_id) => match (self.rig.get(joint_id), self.scene.find_object(&id)) {
                        (Some(joint_tf), Some(obj)) => {
                            let inv_rot = joint_tf.rotation.inverse();
                            let offset_pos = inv_rot * (obj.cuboid.position - joint_tf.position);
                            let offset_rot = inv_rot * obj.cuboid.rotation;
                            self.attachments.attach(
                                &id,
                                Attachment::with_offset(joint_id, offset_pos, offset_rot),
                            );
                        }
                        _ => warn!("grab_at_joint: '{id}' or joint '{joint}' not found"),
                    },
                    None => warn!("grab_at_joint: unknown joint name '{joint}'"),
                },
                EngineCommand::Detach { id } => {
                    self.attachments.detach(&id);
                }
                EngineCommand::GrabAtPoint { id, point, hand } => {
                    let Some(obj) = self.scene.find_object(&id) else {
                        warn!("grab_at_point: unknown object '{id}'");
                        continue;
                    };
                    match obj.grip_point(&point).cloned() {
                        Some(point_def) => self.rigid_physics.grab(&id, hand, &point_def),
                        None => warn!("grab_at_point: '{id}' has no grip point named '{point}'"),
                    }
                }
                EngineCommand::ReleaseGrip { id, hand } => {
                    self.rigid_physics.release(&id, hand);
                }
                EngineCommand::PlaySound { id } => {
                    self.sound_play_requests.insert(id);
                }
                EngineCommand::StopSound { id } => {
                    self.sound_stop_requests.insert(id);
                }
                EngineCommand::SetLightIntensity { id, intensity } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        if let Some(light) = o.light.as_mut() {
                            light.intensity = intensity;
                        }
                    }
                }
                EngineCommand::SetSoundPitch { id, pitch } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        if let Some(sound) = o.sound.as_mut() {
                            sound.pitch = pitch;
                        }
                    }
                }
            }
        }
    }

    pub fn held_grip_point(&self, hand: Hand) -> Option<(&GameObject, &GripPointDef)> {
        let (id, point_name) = self.rigid_physics.held_by(hand)?;
        let obj = self.scene.find_object(id)?;
        let point = obj.grip_point(point_name)?;
        Some((obj, point))
    }

    fn collect_render_cuboids(&self) -> Vec<RenderCuboid> {
        self.scene
            .objects
            .iter()
            .filter(|o| !o.hidden && o.mesh.is_none())
            .map(|o| RenderCuboid {
                id: o.id.clone(),
                position: o.cuboid.position,
                half_size: o.cuboid.half_size,
                rotation: o.cuboid.rotation,
                color: o.cuboid.color,
                wire_color: o.cuboid.wire_color,
                style: o.cuboid.style,
            })
            .collect()
    }

    fn collect_render_meshes(&self) -> Vec<RenderMesh> {
        self.scene
            .objects
            .iter()
            .filter(|o| !o.hidden)
            .filter_map(|o| {
                let mesh_ref: &MeshRef = o.mesh.as_ref()?;
                Some(RenderMesh {
                    id: o.id.clone(),
                    path: mesh_ref.path.clone(),
                    position: o.cuboid.position,
                    rotation: o.cuboid.rotation * mesh_ref.rotation_offset,
                    scale: mesh_ref.scale,
                })
            })
            .collect()
    }

    fn collect_render_lights(&self) -> Vec<RenderLight> {
        // Unlike cuboids/meshes, `hidden` only suppresses a visible body —
        // a light marker object still shines even though it draws nothing.
        self.scene
            .objects
            .iter()
            .filter_map(|o| {
                let light = o.light.as_ref()?;
                Some(RenderLight {
                    id: o.id.clone(),
                    position: o.cuboid.position,
                    direction: o.cuboid.rotation * Vec3::NEG_Z,
                    kind: light.kind,
                    color: light.color,
                    intensity: light.intensity,
                    range: light.range,
                    cone_angle_deg: light.cone_angle_deg,
                })
            })
            .collect()
    }

    /// One-off, non-spatial playback for editor authoring — hear a clip at a
    /// given volume/pitch immediately, without needing a listener or play-mode.
    pub fn preview_sound(&mut self, clip: &str, volume: f32, pitch: f32) {
        self.sound_engine.preview(&self.game_dir, clip, volume, pitch);
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }
}

#[cfg(test)]
mod rigid_physics_test {
    use super::*;

    #[test]
    fn falls_lands_and_loops() {
        let dir = std::env::temp_dir().join("space_soup_engine_rigid_physics_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "floor",
                        "cuboid": { "position": [0.0, -0.5, 0.0], "half_size": [5.0, 0.5, 5.0] },
                        "rigid_body": { "mode": "Static", "shape": "Box" }
                    },
                    {
                        "id": "ball",
                        "cuboid": { "position": [0.0, 5.0, 0.0], "half_size": [0.5, 0.5, 0.5] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 }
                    },
                    {
                        "id": "looping_ball",
                        "cuboid": { "position": [2.0, 1.5, 0.0], "half_size": [0.5, 0.5, 0.5] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0, "respawn_interval": 1.5 }
                    },
                    {
                        "id": "handle_box",
                        "cuboid": { "position": [-3.0, 3.0, 0.0], "half_size": [0.2, 0.2, 0.2] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 },
                        "grip_points": [
                            { "name": "handle", "kind": "Snap", "local_pos": [0.0, 0.0, 0.0] }
                        ],
                        "script": "fn on_grab(hand, point) { grab_at_point(\"handle_box\", point, hand); } fn on_release(hand) { release_grip(\"handle_box\", hand); }"
                    }
                ]
            }"#,
        ).unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;

        let start_y = rt.scene().find_object("ball").unwrap().cuboid.position.y;
        assert!(
            (start_y - 5.0).abs() < 0.01,
            "expected ball to start at y=5.0, got {start_y}"
        );

        let before_grab_y = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;
        assert!((before_grab_y - 3.0).abs() < 0.05, "expected handle_box to still be at its spawn height before being grabbed, got {before_grab_y}");

        let mut grab_input = InputFrame::default();
        grab_input
            .grabbed
            .push(("handle_box".to_string(), Hand::Right, "handle".to_string()));
        let mut rig = PlayerRig::new();
        rig.set_hand_grip(Hand::Right, Vec3::new(-3.0, 3.0, 0.0), Quat::IDENTITY);
        rt.update(dt, &grab_input, rig, &LocomotionInput::default(), None);

        for i in 1..=30 {
            let mut rig = PlayerRig::new();
            rig.set_hand_grip(
                Hand::Right,
                Vec3::new(-3.0, 3.0 - i as f32 * 0.02, 0.0),
                Quat::IDENTITY,
            );
            rt.update(
                dt,
                &InputFrame::default(),
                rig,
                &LocomotionInput::default(),
                None,
            );
        }
        let held_y = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;
        assert!(
            (held_y - 2.4).abs() < 0.1,
            "expected handle_box to follow the hand down to y\u{2248}2.4 while snap-grabbed (gravity should be overridden by the joint), got {held_y}"
        );

        let mut release_input = InputFrame::default();
        release_input
            .released
            .push(("handle_box".to_string(), Hand::Right));
        rt.update(
            dt,
            &release_input,
            PlayerRig::new(),
            &LocomotionInput::default(),
            None,
        );
        let y_at_release = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;

        for _ in 0..30 {
            rt.update(
                dt,
                &InputFrame::default(),
                PlayerRig::new(),
                &LocomotionInput::default(),
                None,
            );
        }
        let y_after_release = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;
        assert!(
            y_after_release < y_at_release - 0.05,
            "expected handle_box to fall freely under gravity after release, went from {y_at_release} to {y_after_release}"
        );

        rt.update(
            dt,
            &InputFrame::default(),
            PlayerRig::new(),
            &LocomotionInput::default(),
            None,
        );
        let after_one_step_y = rt.scene().find_object("ball").unwrap().cuboid.position.y;
        assert!(after_one_step_y < start_y, "expected gravity to have pulled the ball down from its start height by now, went from {start_y} to {after_one_step_y}");

        for _ in 0..180 {
            rt.update(
                dt,
                &InputFrame::default(),
                PlayerRig::new(),
                &LocomotionInput::default(),
                None,
            );
        }
        let landed_y = rt.scene().find_object("ball").unwrap().cuboid.position.y;
        assert!(
            (landed_y - 0.5).abs() < 0.15,
            "expected the ball (half_size.y=0.5) to land resting on the floor's top surface (y=0.0) at y\u{2248}0.5, got {landed_y}"
        );

        let mut saw_high = false;
        let mut saw_low = false;
        for _ in 0..180 {
            rt.update(
                dt,
                &InputFrame::default(),
                PlayerRig::new(),
                &LocomotionInput::default(),
                None,
            );
            let y = rt
                .scene()
                .find_object("looping_ball")
                .unwrap()
                .cuboid
                .position
                .y;
            if y > 1.2 {
                saw_high = true;
            }
            if y < 0.7 {
                saw_low = true;
            }
        }
        assert!(
            saw_high,
            "expected looping_ball to revisit its spawn height (respawn_interval loop)"
        );
        assert!(
            saw_low,
            "expected looping_ball to also reach the floor (it should still fall each cycle)"
        );
    }
}
