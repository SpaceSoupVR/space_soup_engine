use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;
use std::collections::HashMap;

use crate::events::{Hand, InputFrame};
use crate::physics::{Aabb, CollisionEvent};
use crate::runtime::GameRuntime;
use crate::scene::{BindingScope, PlayMode};

impl GameRuntime {
    pub(crate) fn age_particle_bursts(&mut self, dt: f32) {
        for burst in &mut self.particle_bursts {
            burst.elapsed += dt;
        }
        self.particle_bursts.retain(|b| b.elapsed < b.lifetime);
    }

    pub(crate) fn update_object_position_cache(&self) {
        for obj in &self.scene.objects {
            let p = obj.cuboid.position;
            self.script_host.set_object_position(&obj.id, p.x, p.y, p.z);
            let r = obj.cuboid.rotation;
            self.script_host.set_object_rotation(&obj.id, r.x, r.y, r.z, r.w);
            let h = obj.cuboid.half_size;
            self.script_host.set_object_half_size(&obj.id, h.x, h.y, h.z);
        }
    }

    pub(crate) fn update_rig_position_cache(&self, player: PlayerId) {
        let Some(rig) = self.rigs.get(&player) else {
            return;
        };
        let head = rig.head();
        self.script_host.set_rig_position(
            "head",
            head.position.x,
            head.position.y,
            head.position.z,
        );

        for hand in [Hand::Left, Hand::Right] {
            let grip = rig.hand_grip(hand);
            let aim = rig.hand_aim(hand);
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

    pub(crate) fn apply_attachments(&mut self) {
        let results = self.attachments.resolve_all_with_visibility(&self.rigs);
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

    pub(crate) fn apply_socket_attachments(&mut self) {
        let pairs: Vec<(String, String, String)> = self
            .socket_attachments
            .iter()
            .map(|(child, (parent, socket))| (child.clone(), parent.clone(), socket.clone()))
            .collect();

        for (child_id, parent_id, socket_name) in pairs {
            let Some(parent) = self.scene.find_object(&parent_id) else { continue };
            let Some(socket) = parent.socket(&socket_name) else { continue };
            let world_pos = parent.cuboid.position + parent.cuboid.rotation * Vec3::from(socket.local_pos);
            let world_rot = parent.cuboid.rotation * Quat::from_array(socket.local_rot);
            if let Some(child) = self.scene.find_object_mut(&child_id) {
                child.cuboid.position = world_pos;
                child.cuboid.rotation = world_rot;
            }
        }
    }

    pub(crate) fn dispatch_collisions(&mut self) {
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

    pub(crate) fn classify_teleport_pair(
        a: &str,
        b: &str,
        player_ids: &HashMap<String, PlayerId>,
    ) -> Option<(PlayerId, String)> {
        match (player_ids.get(a), player_ids.get(b)) {
            (Some(&p), None) => Some((p, b.to_string())),
            (None, Some(&p)) => Some((p, a.to_string())),
            _ => None,
        }
    }

    pub(crate) fn dispatch_teleportals(&mut self) {
        const PLAYER_HALF_XZ: f32 = 0.25;
        const PLAYER_HALF_Y: f32 = 0.15;

        let mut player_ids: HashMap<String, PlayerId> = HashMap::new();
        let mut bodies: Vec<(String, Aabb)> = Vec::new();

        for (&player, locomotion) in &self.locomotions {
            let body_id = Self::player_body_id(player);
            let center = locomotion.player_offset + Vec3::new(0.0, PLAYER_HALF_Y, 0.0);
            let half = Vec3::new(PLAYER_HALF_XZ, PLAYER_HALF_Y, PLAYER_HALF_XZ);
            bodies.push((body_id.clone(), Aabb::from_center_half(center, half)));
            player_ids.insert(body_id, player);
        }

        for o in &self.scene.objects {
            if o.teleportal.is_some() {
                let aabb = Aabb::from_center_half(o.cuboid.position, o.cuboid.half_size);
                bodies.push((o.id.clone(), aabb));
            }
        }

        let events = self.teleport_collisions.update(&bodies);

        for event in events {
            match event {
                CollisionEvent::Enter(a, b) => {
                    let Some((player, pad_id)) = Self::classify_teleport_pair(&a, &b, &player_ids)
                    else {
                        continue;
                    };
                    if self.teleport_disarmed.contains(&(player, pad_id.clone())) {
                        continue;
                    }
                    let Some(pad) = self.scene.find_object(&pad_id) else {
                        continue;
                    };
                    let Some(teleportal) = pad.teleportal.as_ref() else {
                        continue;
                    };

                    if let Some(target_scene) = teleportal.target_scene.clone() {
                        self.pending_scene_change = Some(target_scene);
                        continue;
                    }

                    let Some(target_id) = teleportal.target_id.clone() else {
                        continue;
                    };
                    let Some(target) = self.scene.find_object(&target_id) else {
                        continue;
                    };
                    if target.teleportal.is_none() {
                        continue;
                    }
                    let target_position = target.cuboid.position;
                    let target_yaw = Self::yaw_from_forward(target.cuboid.rotation * Vec3::NEG_Z);
                    let target_id = target.id.clone();

                    if let Some(locomotion) = self.locomotions.get_mut(&player) {
                        locomotion.player_offset = target_position;
                        locomotion.player_yaw = target_yaw;
                    }
                    self.teleport_disarmed.insert((player, target_id));
                }
                CollisionEvent::Exit(a, b) => {
                    if let Some((player, pad_id)) = Self::classify_teleport_pair(&a, &b, &player_ids) {
                        self.teleport_disarmed.remove(&(player, pad_id));
                    }
                }
            }
        }
    }

    pub(crate) fn dispatch_input(&mut self, input: &InputFrame) {
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

    pub(crate) fn dispatch_animation_bindings(&mut self, input: &InputFrame) {
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

    pub(crate) fn dispatch_update_hook(&self, dt: f32) {
        for obj in &self.scene.objects {
            if self.script_host.has_script(&obj.id) {
                let _ = self.script_host.call(&obj.id, "on_update", (dt as f64,));
            }
        }
    }

}
