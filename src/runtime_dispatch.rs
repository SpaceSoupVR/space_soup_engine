use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;
use std::collections::HashMap;

use crate::events::{Hand, InputFrame};
use crate::physics::{Aabb, CollisionEvent};
use crate::runtime::GameRuntime;
use crate::scene::{BindingScope, PlayMode};
use crate::script::EngineCommand;

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
        // Note `on_release` above is GRAB release, and has meant that since before
        // buttons had an up edge at all. Button edges therefore get their own
        // clearly distinct names rather than an `on_release` overload that would
        // silently collide with every existing grab script.
        self.script_host.set_input_axes(&input.axes);
        for press in &input.button_presses {
            let Some(id) = &press.object_id else { continue };
            let hand = press.hand.unwrap_or_default();
            let _ = self.script_host.call(id, "on_press", (press.button.clone(),));
            let _ = self.script_host.call(
                id,
                "on_button_down",
                (press.button.clone(), hand.as_str().to_string()),
            );
        }
        for press in &input.button_releases {
            let Some(id) = &press.object_id else { continue };
            let hand = press.hand.unwrap_or_default();
            let _ = self.script_host.call(
                id,
                "on_button_up",
                (press.button.clone(), hand.as_str().to_string()),
            );
        }
        self.dispatch_part_triggers(input);
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

impl GameRuntime {
    /// Fire part-animation triggers whose blend crossed their threshold this frame.
    ///
    /// Crossing, not "is past" -- otherwise an action fires every frame the blend
    /// happens to sit beyond the line. And latched with hysteresis, because a hand
    /// held near a threshold jitters across it many times a second; one deliberate
    /// motion has to produce one event.
    ///
    /// Runs here rather than on the headset because the actions are authoritative:
    /// spawning a magazine and handing it to physics decides world state that every
    /// player must agree on.
    pub(crate) fn dispatch_part_triggers(&mut self, input: &InputFrame) {
        use crate::scene_animation::{PartTriggerAction, TRIGGER_HYSTERESIS};

        let mut fired: Vec<(String, PartTriggerAction)> = Vec::new();
        for (object_id, clips) in &input.part_blends {
            let Some(obj) = self.scene.find_object(object_id) else { continue };
            for pa in &obj.part_animations {
                let Some(&blend) = clips.get(&pa.clip) else { continue };
                let prev = self
                    .prev_part_blends
                    .get(object_id)
                    .and_then(|m| m.get(&pa.clip))
                    .copied()
                    .unwrap_or(0.0);
                for (i, trig) in pa.triggers.iter().enumerate() {
                    let key = (object_id.clone(), pa.clip.clone(), i);
                    let crossed = if trig.rising {
                        prev < trig.at && blend >= trig.at
                    } else {
                        prev > trig.at && blend <= trig.at
                    };
                    // Rearm only once the blend has retreated clear of the band.
                    let rearmed = if trig.rising {
                        blend < trig.at - TRIGGER_HYSTERESIS
                    } else {
                        blend > trig.at + TRIGGER_HYSTERESIS
                    };
                    if rearmed {
                        self.latched_triggers.remove(&key);
                    } else if crossed && !self.latched_triggers.contains(&key) {
                        self.latched_triggers.insert(key);
                        fired.push((object_id.clone(), trig.action.clone()));
                    }
                }
            }
        }

        for (object_id, action) in fired {
            self.run_part_trigger_action(&object_id, action);
        }

        self.prev_part_blends = input.part_blends.clone();
    }

    fn run_part_trigger_action(&mut self, object_id: &str, action: crate::scene_animation::PartTriggerAction) {
        use crate::scene_animation::PartTriggerAction as A;
        match action {
            A::DetachPart { part, template, impulse } => {
                // A part is a joint inside a skinned mesh, not an object, so it
                // cannot itself become a rigid body. The handover is a spawn plus a
                // hide: put a physics copy into the world and stop drawing the joint.
                let Some(obj) = self.scene.find_object(object_id) else { return };
                let at = obj.cuboid.position;
                let new_id = format!("{object_id}#{part}#{}", self.next_detached_id);
                self.next_detached_id += 1;
                self.script_host.push_command(EngineCommand::SpawnObject {
                    template_id: template,
                    new_id: new_id.clone(),
                    x: at.x,
                    y: at.y,
                    z: at.z,
                });
                if impulse != [0.0, 0.0, 0.0] {
                    self.script_host.push_command(EngineCommand::ApplyImpulse {
                        id: new_id,
                        x: impulse[0],
                        y: impulse[1],
                        z: impulse[2],
                    });
                }
                self.script_host.push_command(EngineCommand::SetPartVisible {
                    id: object_id.to_string(),
                    part,
                    visible: false,
                });
            }
            A::SetPartVisible { part, visible } => {
                self.script_host.push_command(EngineCommand::SetPartVisible {
                    id: object_id.to_string(),
                    part,
                    visible,
                });
            }
            A::PlaySound { id } => self.script_host.push_command(EngineCommand::PlaySound { id }),
            A::SpawnParticleBurst { id, count } => {
                // Authored as u32 -- a negative burst is not a thing anyone means --
                // and widened here to the command's script-facing i64.
                self.script_host
                    .push_command(EngineCommand::SpawnParticleBurst { id, count: count as i64 })
            }
        }
    }
}
