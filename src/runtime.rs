use glam::{Vec3, Quat};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::Result;
use log::{info, warn};

use crate::manifest::Manifest;
use crate::scene::{Scene, GameObject, CuboidDef, Color3, CuboidStyle, MeshRef};
use crate::animation::{AnimationPlayer, sample};
use crate::physics::{Aabb, CollisionTracker, CollisionEvent};
use crate::events::{InputFrame, Hand};
use crate::script::{ScriptHost, EngineCommand};
use crate::rig::{PlayerRig, JointId};
use crate::attach::{Attachment, AttachmentTable};
use crate::locomotion::{Locomotion, LocomotionMode, LocomotionInput, TeleportTarget};

#[derive(Debug, Clone)]
pub struct RenderCuboid {
    pub id:         String,
    pub position:   Vec3,
    pub half_size:  Vec3,
    pub rotation:   Quat,
    pub color:      Color3,
    pub wire_color: Color3,
    pub style:      CuboidStyle,
}

#[derive(Debug, Clone)]
pub struct RenderMesh {
    pub id:             String,
    pub path:           String,
    pub position:       Vec3,
    pub rotation:       Quat,
    pub scale:          Vec3,
}

pub struct GameRuntime {
    game_dir:    PathBuf,
    manifest:    Manifest,
    scene:       Scene,

    script_host: ScriptHost,
    players:     HashMap<String, AnimationPlayer>,
    collisions:  CollisionTracker,

    pub rig:         PlayerRig,
    pub attachments: AttachmentTable,
    pub locomotion:  Locomotion,

    pending_scene_change: Option<String>,
}

impl GameRuntime {
    pub fn load(game_dir: &Path) -> Result<Self> {
        let manifest = Manifest::load(game_dir)?;
        let scene_path = manifest.entry_scene_path(game_dir);
        let scene = Scene::load(&scene_path)?;

        let mut rt = Self {
            game_dir:    game_dir.to_path_buf(),
            manifest,
            scene,
            script_host: ScriptHost::new(),
            players:     HashMap::new(),
            collisions:  CollisionTracker::new(),
            rig:         PlayerRig::new(),
            attachments: AttachmentTable::new(),
            locomotion:  Locomotion::new(LocomotionMode::Smooth),
            pending_scene_change: None,
        };

        rt.compile_scripts();
        info!("GameRuntime: loaded scene '{}' with {} objects",
            rt.scene.name, rt.scene.objects.len());

        Ok(rt)
    }

    pub fn render_lists(&self) -> (Vec<RenderCuboid>, Vec<RenderMesh>) {
        (self.collect_render_cuboids(), self.collect_render_meshes())
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

        self.scene      = scene;
        self.players    = HashMap::new();
        self.collisions = CollisionTracker::new();
        self.script_host = ScriptHost::new();
        self.compile_scripts();

        info!("GameRuntime: switched to scene '{scene_name}'");
        Ok(())
    }

    pub fn manifest(&self) -> &Manifest { &self.manifest }
    pub fn scene_name(&self) -> &str { &self.scene.name }

    pub fn game_dir(&self) -> &Path { &self.game_dir }

    pub fn update(
        &mut self,
        dt:               f32,
        input:            &InputFrame,
        rig:              PlayerRig,
        locomotion_input: &LocomotionInput,
        teleport_target:  Option<TeleportTarget>,
    ) -> (Vec<RenderCuboid>, Vec<RenderMesh>, Option<String>) {
        self.pending_scene_change = None;
        self.rig = rig;

        self.locomotion.update(dt, locomotion_input, &self.rig, teleport_target);

        self.update_animations(dt);
        self.update_object_position_cache();
        self.update_rig_position_cache();
        self.apply_attachments();
        self.dispatch_collisions();
        self.dispatch_input(input);
        self.dispatch_update_hook(dt);
        self.apply_script_commands();

        let cuboids = self.collect_render_cuboids();
        let meshes  = self.collect_render_meshes();
        (cuboids, meshes, self.pending_scene_change.take())
    }

    pub fn world_head_transform(&self) -> (Vec3, Quat) {
        let head = self.rig.head();
        self.locomotion.apply_to_head(head.position, head.rotation)
    }

    fn update_animations(&mut self, dt: f32) {
        let mut finished: Vec<String> = Vec::new();

        let Self { scene, players, .. } = self;
        for (obj_id, player) in players.iter_mut() {
            let Some(obj) = scene.find_object(obj_id) else { continue };
            let Some(anim) = obj.find_animation(&player.anim_name) else { continue };
            let duration = anim.duration();
            player.tick(dt, duration);
            if player.finished {
                finished.push(obj_id.clone());
            }
        }

        let samples: Vec<(String, crate::animation::Sample)> = self.players.iter()
            .filter_map(|(obj_id, player)| {
                let obj = self.scene.find_object(obj_id)?;
                let anim = obj.find_animation(&player.anim_name)?;
                Some((obj_id.clone(), sample(anim, player.elapsed)))
            })
            .collect();

        for (obj_id, s) in samples {
            if let Some(obj_mut) = self.scene.find_object_mut(&obj_id) {
                if let Some(p) = s.position { obj_mut.cuboid.position = p; }
                if let Some(r) = s.rotation { obj_mut.cuboid.rotation = r; }
                if let Some(sc) = s.scale   { obj_mut.cuboid.half_size = sc; }
                if let Some(c) = s.color    { obj_mut.cuboid.color = c; }
            }
        }

        for id in finished {
            self.players.remove(&id);
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
        self.players.insert(obj_id.to_string(), AnimationPlayer::new(anim));
    }

    fn stop_animation(&mut self, obj_id: &str) {
        self.players.remove(obj_id);
    }

    fn update_object_position_cache(&self) {
        for obj in &self.scene.objects {
            let p = obj.cuboid.position;
            self.script_host.set_object_position(&obj.id, p.x, p.y, p.z);
        }
    }

    fn update_rig_position_cache(&self) {
        let head = self.rig.head();
        self.script_host.set_rig_position("head", head.position.x, head.position.y, head.position.z);

        for hand in [Hand::Left, Hand::Right] {
            let grip = self.rig.hand_grip(hand);
            let aim  = self.rig.hand_aim(hand);
            let prefix = hand.as_str();
            self.script_host.set_rig_position(
                &format!("{prefix}_grip"), grip.position.x, grip.position.y, grip.position.z,
            );
            self.script_host.set_rig_position(
                &format!("{prefix}_aim"), aim.position.x, aim.position.y, aim.position.z,
            );
        }
    }

    fn apply_attachments(&mut self) {
        let resolved = self.attachments.resolve_all(&self.rig);
        for (obj_id, tf) in resolved {
            if let Some(obj) = self.scene.find_object_mut(&obj_id) {
                obj.cuboid.position = tf.position;
                obj.cuboid.rotation = tf.rotation;
            }
        }
    }

    fn dispatch_collisions(&mut self) {
        let bodies: Vec<(String, Aabb)> = self.scene.objects.iter()
            .map(|o| {
                let aabb = Aabb::from_center_half(o.cuboid.position, o.cuboid.half_size);
                (o.id.clone(), aabb)
            })
            .collect();

        let events = self.collisions.update(&bodies);

        for event in events {
            match event {
                CollisionEvent::Enter(a, b) => {
                    let _ = self.script_host.call(&a, "on_collision_enter", (b.clone(),));
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
            let _ = self.script_host.call(id, "on_point", (hand.as_str().to_string(),));
        }
        for (id, hand) in &input.grabbed {
            let _ = self.script_host.call(id, "on_grab", (hand.as_str().to_string(),));
        }
        for (id, hand) in &input.released {
            let _ = self.script_host.call(id, "on_release", (hand.as_str().to_string(),));
        }
        for press in &input.button_presses {
            if let Some(id) = &press.object_id {
                let _ = self.script_host.call(id, "on_press", (press.button.clone(),));
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
                    self.attachments.detach(&id);
                }
                EngineCommand::AttachToJoint { id, joint, offset_x, offset_y, offset_z } => {
                    match JointId::from_name(&joint) {
                        Some(joint_id) => {
                            let att = Attachment::with_offset(
                                joint_id,
                                Vec3::new(offset_x, offset_y, offset_z),
                                Quat::IDENTITY,
                            );
                            self.attachments.attach(&id, att);
                        }
                        None => warn!("attach_to_joint: unknown joint name '{joint}'"),
                    }
                }
                EngineCommand::Detach { id } => {
                    self.attachments.detach(&id);
                }
            }
        }
    }

    fn collect_render_cuboids(&self) -> Vec<RenderCuboid> {
        self.scene.objects.iter()
            .filter(|o| !o.hidden && o.mesh.is_none())
            .map(|o| RenderCuboid {
                id:         o.id.clone(),
                position:   o.cuboid.position,
                half_size:  o.cuboid.half_size,
                rotation:   o.cuboid.rotation,
                color:      o.cuboid.color,
                wire_color: o.cuboid.wire_color,
                style:      o.cuboid.style,
            })
            .collect()
    }

    fn collect_render_meshes(&self) -> Vec<RenderMesh> {
        self.scene.objects.iter()
            .filter(|o| !o.hidden)
            .filter_map(|o| {
                let mesh_ref: &MeshRef = o.mesh.as_ref()?;
                Some(RenderMesh {
                    id:       o.id.clone(),
                    path:     mesh_ref.path.clone(),
                    position: o.cuboid.position,
                    rotation: o.cuboid.rotation * mesh_ref.rotation_offset,
                    scale:    mesh_ref.scale,
                })
            })
            .collect()
    }

    pub fn scene(&self) -> &Scene { &self.scene }
    pub fn scene_mut(&mut self) -> &mut Scene { &mut self.scene }
}
