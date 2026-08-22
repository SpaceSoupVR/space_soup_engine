use glam::{Quat, Vec3};
use log::warn;
use space_soup_protocol::PlayerId;

use crate::attach::Attachment;
use crate::rig::JointId;
use crate::runtime::{GameRuntime, ParticleBurst};
use crate::scene::Color3;
use crate::script::EngineCommand;

impl GameRuntime {
    pub(crate) fn apply_script_commands(&mut self) {
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
                        self.attachments.attach(&id, PlayerId::local(), att);
                    }
                    None => warn!("attach_to_joint: unknown joint name '{joint}'"),
                },
                EngineCommand::GrabAtJoint {
                    id,
                    joint,
                    point,
                    player,
                } => match JointId::from_name(&joint) {
                    Some(joint_id) => match (
                        self.rigs.get(&player).and_then(|r| r.get(joint_id)),
                        self.scene.find_object(&id),
                    ) {
                        (Some(joint_tf), Some(obj)) => {
                            let matched_point = point.as_deref().and_then(|p| obj.grip_point(p));
                            // A named point that does not exist silently became a
                            // generic centre-of-object grab, which looks like a
                            // bad grip pose rather than a typo.
                            if let (Some(named), None) = (point.as_deref(), matched_point) {
                                warn!(
                                    "grab_at_joint: '{id}' has no grip point named '{named}' --                                      grabbing at the object centre instead"
                                );
                            }
                            if let Some(g) = matched_point {
                                if self
                                    .attachments
                                    .point_held_by_other(&id, &g.name, player, joint_id)
                                {
                                    warn!(
                                        "grab_at_joint: '{id}' point '{}' already held by another hand",
                                        g.name
                                    );
                                    continue;
                                }
                                // A support grip is not a way to pick the object up.
                                // Every authored grip used to be independently
                                // grabbable, so a rifle could be carried by its
                                // handguard with no hand on the pistol grip.
                                if g.support && !self.attachments.held_by_player(&id, player) {
                                    warn!(
                                        "grab_at_joint: '{id}' point '{}' is a support grip and                                          this player is not holding '{id}' yet -- ignored",
                                        g.name
                                    );
                                    continue;
                                }
                            }
                            let (offset_pos, offset_rot) = matched_point
                                .map(|g| {
                                    let local_rot = Quat::from_array(g.local_rot);
                                    let inv_rot = local_rot.inverse();
                                    (inv_rot * -Vec3::from(g.local_pos), inv_rot)
                                })
                                .unwrap_or_else(|| {
                                    let inv_rot = joint_tf.rotation.inverse();
                                    (
                                        inv_rot * (obj.cuboid.position - joint_tf.position),
                                        inv_rot * obj.cuboid.rotation,
                                    )
                                });
                            let attachment = match matched_point {
                                Some(g) => Attachment::with_grip_point(
                                    joint_id,
                                    offset_pos,
                                    offset_rot,
                                    g.name.clone(),
                                ),
                                None => Attachment::with_offset(joint_id, offset_pos, offset_rot),
                            };
                            self.attachments.attach(&id, player, attachment);
                        }
                        // Naming which of the two is missing, because they have
                        // completely different causes and the old combined message
                        // ("'x' or joint 'y' not found") pointed at neither. A
                        // rig missing the joint is the non-obvious one: the name
                        // parsed fine, the player just has no such joint yet.
                        (None, _) => warn!(
                            "grab_at_joint: player {player:?} has no '{joint}' joint in their rig,                              so '{id}' was not picked up"
                        ),
                        (_, None) => warn!("grab_at_joint: no object '{id}' in the scene"),
                    },
                    None => warn!("grab_at_joint: unknown joint name '{joint}'"),
                },
                EngineCommand::Detach { id, hand, player } => match hand {
                    Some(h) => self
                        .attachments
                        .detach_joint(&id, player, JointId::HandGrip(h)),
                    None => self.attachments.detach(&id),
                },
                EngineCommand::GrabAtPoint {
                    id,
                    point,
                    hand,
                    player,
                } => {
                    let Some(obj) = self.scene.find_object(&id) else {
                        warn!("grab_at_point: unknown object '{id}'");
                        continue;
                    };
                    match obj.grip_point(&point).cloned() {
                        Some(point_def) => self.rigid_physics.grab(player, &id, hand, &point_def),
                        None => warn!("grab_at_point: '{id}' has no grip point named '{point}'"),
                    }
                }
                EngineCommand::ReleaseGrip { id, hand, player } => {
                    self.rigid_physics.release(player, &id, hand);
                }
                EngineCommand::PlaySound { id } => {
                    self.sound_play_requests.insert(id);
                }
                EngineCommand::StopSound { id } => {
                    self.sound_stop_requests.insert(id);
                }
                EngineCommand::SetLightIntensity { id, intensity } => {
                    if let Some(o) = self.scene.find_object_mut(&id) {
                        for light in o.lights.iter_mut() {
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
                EngineCommand::SetPartBlend { id, clip, blend } => {
                    self.manual_part_blends
                        .entry(id)
                        .or_default()
                        .insert(clip, blend.clamp(0.0, 1.0));
                }
                EngineCommand::SetPartVisible { id, part, visible } => {
                    let Some(obj) = self.scene.find_object_mut(&id) else {
                        warn!("set_part_visible: unknown object '{id}'");
                        continue;
                    };
                    // Store only the hidden ones, so "visible" is the default for
                    // every part of every model that never mentions this.
                    if visible {
                        obj.hidden_parts.retain(|p| p != &part);
                    } else if !obj.hidden_parts.iter().any(|p| p == &part) {
                        obj.hidden_parts.push(part);
                    }
                }
                EngineCommand::SetObjectVisible { id, visible } => {
                    let Some(obj) = self.scene.find_object_mut(&id) else {
                        warn!("set_visible: unknown object '{id}'");
                        continue;
                    };
                    // Nothing else to do: the render lists are rebuilt from
                    // `hidden` every frame, and those lists are what reaches the
                    // headset -- so this needs no wire message and no client
                    // change, exactly as breaching did not.
                    obj.hidden = !visible;
                }
                EngineCommand::SetObjectSolid { id, solid } => {
                    let Some(obj) = self.scene.find_object_mut(&id) else {
                        warn!("set_solid: unknown object '{id}'");
                        continue;
                    };
                    let Some(rb) = obj.rigid_body.as_mut() else {
                        // Not a no-op worth swallowing: the author asked for a
                        // collider on something that has no definition of one,
                        // and silence would look exactly like it worked.
                        warn!("set_solid: '{id}' has no rigid_body to enable or disable");
                        continue;
                    };
                    if rb.enabled == solid {
                        continue;
                    }
                    rb.enabled = solid;
                    // Cloned before touching physics: `spawn_actor` needs the
                    // object and the def while `self.scene` is no longer
                    // borrowed. The clone is one object, once, on a state change.
                    let obj = obj.clone();
                    let def = obj.rigid_body.clone().expect("checked above");
                    if solid {
                        // At its CURRENT transform, and a dynamic body comes
                        // back at rest -- momentum is not preserved. Right for a
                        // door or a force field; not a way to resume a rolling
                        // barrel mid-roll.
                        self.rigid_physics.spawn_actor(&obj, &def);
                    } else {
                        // BOTH kinds. Each despawn consults only its own map, so
                        // calling one for the wrong kind returns silently -- the
                        // exact failure that left a breached wall solid.
                        self.rigid_physics.despawn_static(&id);
                        self.rigid_physics.despawn_actor(&id);
                    }
                }
                EngineCommand::SpawnObject { template_id, new_id, x, y, z } => {
                    if self.scene.find_object(&new_id).is_some() {
                        warn!("spawn_object: id '{new_id}' already exists");
                        continue;
                    }
                    let Some(template) = self.scene.find_object(&template_id) else {
                        warn!("spawn_object: unknown template '{template_id}'");
                        continue;
                    };
                    let mut new_obj = template.clone();
                    new_obj.id = new_id;
                    new_obj.cuboid.position = Vec3::new(x, y, z);
                    new_obj.hidden = false;
                    if let Some(def) = new_obj.rigid_body.clone() {
                        self.rigid_physics.spawn_actor(&new_obj, &def);
                    }
                    self.scene.objects.push(new_obj);
                }
                EngineCommand::ApplyImpulse { id, x, y, z } => {
                    self.rigid_physics.apply_impulse(&id, Vec3::new(x, y, z));
                }
                EngineCommand::AttachToSocket { child_id, parent_id, socket } => {
                    let has_socket = self
                        .scene
                        .find_object(&parent_id)
                        .is_some_and(|p| p.socket(&socket).is_some());
                    if !has_socket {
                        warn!("attach_to_socket: '{parent_id}' has no socket named '{socket}'");
                        continue;
                    }
                    if self.scene.find_object(&child_id).is_none() {
                        warn!("attach_to_socket: unknown child object '{child_id}'");
                        continue;
                    }
                    self.rigid_physics.despawn_actor(&child_id);
                    self.socket_attachments.insert(child_id, (parent_id, socket));
                }
                EngineCommand::DetachFromSocket { child_id } => {
                    if self.socket_attachments.remove(&child_id).is_none() {
                        continue;
                    }
                    if let Some(obj) = self.scene.find_object(&child_id) {
                        if let Some(def) = obj.rigid_body.clone() {
                            self.rigid_physics.spawn_actor(obj, &def);
                        }
                    }
                }
                EngineCommand::SpawnParticleBurst { id, count } => {
                    let Some(obj) = self.scene.find_object(&id) else {
                        warn!("spawn_particle_burst: unknown object '{id}'");
                        continue;
                    };
                    let template = obj.particle_emitter.clone().unwrap_or_default();
                    let burst_id = format!("{id}#burst{}", self.next_particle_burst_id);
                    self.next_particle_burst_id += 1;
                    self.particle_bursts.push(ParticleBurst {
                        id: burst_id,
                        position: obj.cuboid.position,
                        direction: obj.cuboid.rotation * Vec3::NEG_Z,
                        color: template.color,
                        count: count.max(0) as u32,
                        speed: template.speed,
                        spread_deg: template.spread_deg,
                        particle_size: template.particle_size,
                        lifetime: template.lifetime,
                        elapsed: 0.0,
                    });
                }
            }
        }
    }
}
