use glam::Vec3;
use space_soup_protocol::PlayerId;

use crate::events::Hand;
use crate::rig::JointId;
use crate::runtime::{
    GameRuntime, RenderCuboid, RenderLaser, RenderLight, RenderMesh, RenderParticleBurst,
    RenderParticleEmitter, SoundState,
};
use crate::scene::{CuboidShape, GameObject, GripPointDef, MeshRef};

impl GameRuntime {
    pub fn held_grip_point(&self, player: PlayerId, hand: Hand) -> Option<(&GameObject, &GripPointDef)> {
        if let Some((id, point_name)) = self.rigid_physics.held_by(player, hand) {
            if let Some(point) = self
                .scene
                .find_object(id)
                .and_then(|obj| obj.grip_point(point_name).map(|p| (obj, p)))
            {
                return Some(point);
            }
        }

        let (id, point_name) = self
            .attachments
            .grip_point_at_joint(player, JointId::HandGrip(hand))?;
        let obj = self.scene.find_object(id)?;
        let point = obj.grip_point(point_name)?;
        Some((obj, point))
    }

    pub(crate) fn collect_render_cuboids(&self) -> Vec<RenderCuboid> {
        self.scene
            .objects
            .iter()
            // `hidden` is what the level author decided; `is_removed` is what
            // this match has done to it. A chunk shot out of a wall has to stop
            // being drawn, and it has no named parts for `hidden_parts` to
            // reach -- that field is only consulted for meshes below.
            //
            // Brushes are excluded because they are NOT boxes. Sending one here
            // drew a wall as its bounding cuboid -- a fractured wall came out as
            // twelve overlapping crates -- so the client meshes brushes itself
            // from scene data it already has, and this list would only double
            // -draw them. See `hidden_brushes` for how it is told which to skip.
            .filter(|o| {
                !o.hidden && !self.damage.is_removed(o) && o.mesh.is_none() && o.brush.is_none()
            })
            .map(|o| RenderCuboid {
                id: o.id.clone(),
                position: o.cuboid.position,
                half_size: o.cuboid.half_size,
                rotation: o.cuboid.rotation,
                color: o.cuboid.color,
                wire_color: o.cuboid.wire_color,
                style: o.cuboid.style,
                reflectivity: o.cuboid.reflectivity,
                shape: if o.teleportal.is_some() {
                    CuboidShape::Cylinder
                } else {
                    CuboidShape::Box
                },
            })
            .collect()
    }

    /// Brush objects the client must NOT draw this frame.
    ///
    /// Brush geometry is static scene data that ships with the game, so the
    /// client builds it once at scene load rather than receiving triangles over
    /// the wire -- the same trade terrain makes, and for the same reason: at 64
    /// players the snapshot budget is the binding constraint.
    ///
    /// What that cannot know is what has happened SINCE. A chunk shot out of a
    /// wall, or a brush a script hid, is a runtime fact, and it is the only part
    /// of a brush that has to cross per snapshot. Sending ids rather than
    /// geometry keeps it to a few bytes, and to nothing at all in the ordinary
    /// case where a level is intact.
    pub fn hidden_brushes(&self) -> Vec<String> {
        self.scene
            .objects
            .iter()
            .filter(|o| o.brush.is_some())
            .filter(|o| o.hidden || self.damage.is_removed(o))
            .map(|o| o.id.clone())
            .collect()
    }

    pub(crate) fn collect_render_meshes(&self) -> Vec<RenderMesh> {
        self.scene
            .objects
            .iter()
            .filter(|o| !o.hidden && !self.damage.is_removed(o))
            .filter_map(|o| {
                let mesh_ref: &MeshRef = o.mesh.as_ref()?;
                Some(RenderMesh {
                    id: o.id.clone(),
                    path: mesh_ref.path.clone(),
                    position: o.cuboid.position,
                    rotation: o.cuboid.rotation * mesh_ref.rotation_offset,
                    scale: mesh_ref.scale,
                    // A FIXTURE drives its own emissive from the lights it
                    // carries. The brightest one wins rather than the sum: two
                    // bulbs in one housing do not make the glass twice as bright,
                    // and summing would let a chandelier's glass saturate purely
                    // from the count of its lamps.
                    //
                    // Zero for every object with no lights, which is almost all
                    // of them, so nothing else in any scene changes appearance.
                    emissive_drive: o
                        .lights
                        .iter()
                        .map(crate::scene_light::emissive_drive)
                        .fold(0.0f32, f32::max),
                    manual_part_blends: self.manual_part_blends.get(&o.id).cloned().unwrap_or_default(),
                    // The damage ledger decides this, not the authored list.
                    // Damage therefore reaches the headset through a field that
                    // already replicates -- breaching needed no new wire
                    // message, no new client code and no new decoder.
                    hidden_parts: self.damage.hidden_parts_for(o).to_vec(),
                    disabled_clips: o
                        .part_animations
                        .iter()
                        .filter(|pa| {
                            pa.enabled_when
                                .as_ref()
                                .is_some_and(|c| !c.holds(&|k| self.script_host.var_string(k)))
                        })
                        .map(|pa| pa.clip.clone())
                        .collect(),
                })
            })
            .collect()
    }

    /// The lights the GPU shades every frame.
    ///
    /// `Baked` lights are excluded: their contribution is already in the
    /// lightmap, and uploading them as well would add the same light twice --
    /// once occluded by the walls and once through them. The visible result is
    /// a room that is too bright AND still leaks, which looks like two
    /// unrelated bugs.
    ///
    /// Their index is still taken from the object's full light list, so a
    /// light's id does not change when a sibling is switched to baked.
    pub(crate) fn collect_render_lights(&self) -> Vec<RenderLight> {
        self.scene
            .objects
            .iter()
            .flat_map(|o| {
                o.lights
                    .iter()
                    .enumerate()
                    .filter(|(_, light)| light.mode == crate::LightMode::Realtime)
                    .map(move |(i, light)| {
                    // THROUGH THE SOCKET, exactly as the baker does.
                    //
                    // This used to take the object's own position and rotation
                    // and ignore `light.socket` entirely, so a fixture baked its
                    // shadows and bounce from the bulb while RENDERING its beam
                    // from the housing's origin. For the hanging lamp that is
                    // 1.18m too high -- level with the ceiling it hangs from --
                    // and, worse, the socket's own rotation is what aims the
                    // beam DOWN: without it a pendant lamp fires horizontally
                    // along -Z at ceiling height.
                    //
                    // Both rooms in `test_room` were dark for this reason. It
                    // hid behind a coincidence: every lamp that looked correct
                    // was a bare wall spot with no socket, which is exactly the
                    // case this code path got right.
                    let pose = crate::scene_light::resolve_light_pose(
                        light,
                        o.cuboid.position,
                        o.cuboid.rotation,
                        light.socket.as_deref().and_then(|n| o.socket(n)),
                    );
                    RenderLight {
                    id: format!("{}#{i}", o.id),
                    position: pose.position,
                    direction: pose.direction(),
                    kind: light.kind,
                    color: light.color,
                    intensity: light.intensity,
                    range: light.range,
                    cone_angle_deg: light.cone_angle_deg,
                    inner_cone_angle_deg: light.inner_cone_angle_deg,
                }})
            })
            .collect()
    }

    pub(crate) fn collect_render_particle_emitters(&self) -> Vec<RenderParticleEmitter> {
        self.scene
            .objects
            .iter()
            .filter_map(|o| {
                let pe = o.particle_emitter.as_ref()?;
                Some(RenderParticleEmitter {
                    id: o.id.clone(),
                    position: o.cuboid.position,
                    direction: o.cuboid.rotation * Vec3::NEG_Z,
                    particle_size: pe.particle_size,
                    spawn_rate: pe.spawn_rate,
                    color: pe.color,
                    lifetime: pe.lifetime,
                    speed: pe.speed,
                    spread_deg: pe.spread_deg,
                    size_growth: pe.size_growth,
                })
            })
            .collect()
    }

    pub(crate) fn collect_render_particle_bursts(&self) -> Vec<RenderParticleBurst> {
        self.particle_bursts
            .iter()
            .map(|b| RenderParticleBurst {
                id: b.id.clone(),
                position: b.position,
                direction: b.direction,
                color: b.color,
                count: b.count,
                speed: b.speed,
                spread_deg: b.spread_deg,
                particle_size: b.particle_size,
                lifetime: b.lifetime,
                elapsed: b.elapsed,
            })
            .collect()
    }

    pub(crate) fn collect_render_lasers(&self) -> Vec<RenderLaser> {
        self.scene
            .objects
            .iter()
            .filter_map(|o| {
                let laser = o.laser.as_ref()?;
                let origin = o.cuboid.position;
                let direction = o.cuboid.rotation * Vec3::NEG_Z;
                let end = self
                    .rigid_physics
                    .raycast(origin, direction, laser.max_distance)
                    .map(|(hit_point, _normal)| hit_point)
                    .unwrap_or(origin + direction * laser.max_distance);
                Some(RenderLaser {
                    id: o.id.clone(),
                    origin,
                    direction,
                    end,
                    color: laser.color,
                    beam_width: laser.beam_width,
                })
            })
            .collect()
    }

    pub fn preview_sound(&mut self, clip: &str, volume: f32, pitch: f32) {
        self.sound_engine.preview(&self.game_dir, clip, volume, pitch);
    }

    pub fn active_sounds(&self) -> Vec<SoundState> {
        self.sound_engine
            .active_sounds(&self.scene.objects)
            .into_iter()
            .map(|(object_id, clip, position, volume, pitch, looping, min_distance, max_distance)| SoundState {
                object_id,
                clip,
                position,
                volume,
                pitch,
                looping,
                min_distance,
                max_distance,
            })
            .collect()
    }
}

#[cfg(test)]
mod fixture_tests {
    use crate::runtime_test_support::PHYSX_TEST_LOCK;
    use crate::GameRuntime;

    /// A lamp: ONE object carrying both a mesh and a light.
    ///
    /// The same-entity model, and the reason there is no `LightFixtureDef`:
    /// `GameObject` already holds both a `mesh` and a `lights` list, so a
    /// fixture is a scene that uses what is there rather than a new component.
    fn drive_of(lights_json: &str) -> f32 {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "ss_fixture_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let scenes = dir.join("scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"t","version":"0.1.0","entry_scene":"t","scenes":["t"]}"#,
        )
        .unwrap();
        std::fs::write(
            scenes.join("t.json"),
            format!(
                r#"{{"name":"t","objects":[{{
                    "id": "lamp",
                    "cuboid": {{ "position": [0,0,0], "half_size": [0.2,0.2,0.2] }},
                    "mesh": {{ "path": "models/lamp.glb" }},
                    "lights": {lights_json}
                }}]}}"#
            ),
        )
        .unwrap();
        let rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        rt.collect_render_meshes()
            .into_iter()
            .find(|m| m.id == "lamp")
            .expect("the lamp has a mesh and so must be drawn")
            .emissive_drive
    }

    #[test]
    fn a_lit_lamp_drives_its_own_mesh() {
        assert!(
            drive_of(r#"[{"intensity": 4.0}]"#) > 0.0,
            "a lamp that is on must light its own bulb",
        );
    }

    #[test]
    fn switching_a_lamp_off_darkens_its_bulb() {
        // The whole point of the same-entity model: nothing has to keep the
        // beam and the glass in step, so they cannot disagree.
        assert_eq!(
            drive_of(r#"[{"intensity": 4.0, "enabled": false}]"#),
            0.0,
            "an off lamp must have a dark bulb",
        );
    }

    #[test]
    fn an_object_with_no_lights_never_glows() {
        // Almost everything in a level. Adding fixtures must not relight it.
        assert_eq!(drive_of("[]"), 0.0);
    }

    #[test]
    fn two_bulbs_in_one_housing_take_the_brightest_not_the_sum() {
        // Summing would let a chandelier's glass saturate purely from the COUNT
        // of its lamps -- brightness decided by how the fixture was modelled
        // rather than by how bright it actually is.
        let one = drive_of(r#"[{"intensity": 4.0}]"#);
        let three = drive_of(
            r#"[{"intensity": 4.0}, {"intensity": 4.0}, {"intensity": 4.0}]"#,
        );
        assert_eq!(one, three, "three equal bulbs must glow like one, not three times as hard");
    }

    #[test]
    fn a_dimmed_lamp_dims_its_bulb() {
        assert!(
            drive_of(r#"[{"intensity": 1.0}]"#) < drive_of(r#"[{"intensity": 8.0}]"#),
            "the bulb follows the beam, so dimming and flicker come for free",
        );
    }
}

#[cfg(test)]
mod socket_light_tests {
    //! Where a fixture's beam actually comes from.
    //!
    //! The baker resolves a light through its socket and always has; the
    //! renderer did not, so a lamp's shadows and bounce were computed at the
    //! bulb while its beam was emitted from the housing's origin. Nothing
    //! errored, and it looked like a lighting bug rather than two subsystems
    //! disagreeing about a position.

    use crate::scene::{Color3, GameObject, LightDef, LightKind, LightMode};
    use crate::scene_rig::SocketDef;
    use glam::{Quat, Vec3};

    /// The hanging lamp as `test_room` authors it: socket 1.18m below the
    /// housing, rotated a quarter turn so the beam points at the floor.
    fn pendant_lamp() -> GameObject {
        let mut o = GameObject::default();
        o.id = "lamp".to_string();
        o.cuboid.position = Vec3::new(0.0, 3.1, -4.5);
        o.cuboid.rotation = Quat::IDENTITY;
        o.sockets = vec![SocketDef {
            name: "bulb".to_string(),
            local_pos: [0.0, -1.18, 0.0],
            local_rot: [-0.7071068, 0.0, 0.0, 0.7071068],
            part: None,
        }];
        let mut l = LightDef {
            kind: LightKind::Spot,
            mode: LightMode::Realtime,
            color: Color3(255, 244, 214, 255),
            intensity: 9.0,
            range: 14.0,
            cone_angle_deg: 64.0,
            inner_cone_angle_deg: 30.0,
            socket: Some("bulb".to_string()),
            ..Default::default()
        };
        l.socket = Some("bulb".to_string());
        o.lights = vec![l];
        o
    }

    /// Through a real `GameRuntime`, because that is the path the headset
    /// takes -- testing `resolve_light_pose` directly would pass whether or not
    /// `collect_render_lights` ever called it, which is precisely the bug.
    fn rendered(o: GameObject) -> (Vec3, Vec3) {
        let _guard = crate::runtime_test_support::PHYSX_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_socket_light_test");
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();
        let scene = crate::scene::Scene { name: "test".to_string(), objects: vec![o], ..Default::default() };
        std::fs::write(
            dir.join("scenes/test.json"),
            serde_json::to_string(&scene).unwrap(),
        )
        .unwrap();
        let rt = crate::runtime::GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let lights = rt.collect_render_lights();
        assert_eq!(lights.len(), 1, "the lamp did not produce a render light");
        (lights[0].position, lights[0].direction)
    }

    /// THE regression: a pendant lamp points DOWN.
    ///
    /// Without the socket's rotation the beam is `object_rot * -Z`, which for
    /// an unrotated lamp is horizontal -- so both rooms in `test_room` had
    /// their lamps firing sideways along the ceiling and the floors were black.
    #[test]
    fn a_pendant_lamp_aims_at_the_floor() {
        let (_, dir) = rendered(pendant_lamp());
        assert!(
            dir.y < -0.99,
            "the lamp's beam points {dir:?}, not down; the socket's rotation is \
             not reaching the renderer",
        );
    }

    /// And it emits from the BULB, not from the housing's origin 1.18m above.
    #[test]
    fn a_pendant_lamp_emits_from_its_bulb() {
        let (pos, _) = rendered(pendant_lamp());
        assert!(
            (pos - Vec3::new(0.0, 3.1 - 1.18, -4.5)).length() < 1e-3,
            "the lamp emits from {pos:?} rather than from its socket",
        );
    }

    /// The renderer and the baker must agree, because they are the two halves
    /// of one lamp: the beam you see and the shadow it casts.
    #[test]
    fn the_renderer_and_the_baker_place_a_light_identically() {
        let o = pendant_lamp();
        let (pos, dir) = rendered(o.clone());
        let baked = crate::scene_light::resolve_light_pose(
            &o.lights[0],
            o.cuboid.position,
            o.cuboid.rotation,
            o.socket("bulb"),
        );
        assert!(
            (pos - baked.position).length() < 1e-4,
            "renderer puts the light at {pos:?}, baker at {:?}",
            baked.position,
        );
        assert!(
            (dir - baked.direction()).length() < 1e-4,
            "renderer aims it {dir:?}, baker aims it {:?}",
            baked.direction(),
        );
    }

    /// A bare spot with no socket is untouched -- which is every lamp that
    /// looked correct while this was broken, and why it stayed hidden.
    #[test]
    fn a_socketless_spot_is_unchanged() {
        let mut o = pendant_lamp();
        o.sockets.clear();
        o.lights[0].socket = None;
        o.cuboid.rotation = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let (pos, dir) = rendered(o);
        assert!((pos - Vec3::new(0.0, 3.1, -4.5)).length() < 1e-4);
        assert!((dir - Vec3::X).length() < 1e-3, "got {dir:?}");
    }
}
