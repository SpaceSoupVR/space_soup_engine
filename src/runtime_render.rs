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
            //
            // Markers -- spawn points, reflection probes, trigger volumes -- carry
            // a cuboid only as the editor's handle (the baker's `occludes_light`
            // and `reflection_proxy::is_drawn_geometry` skip them too). Drawn in
            // play, the hall's spawn point was a 10 cm green box half sunk in
            // the marble floor.
            .filter(|o| {
                !o.hidden
                    && !self.damage.is_removed(o)
                    && o.mesh.is_none()
                    && o.brush.is_none()
                    && o.spawn_point.is_none()
                    && o.reflection_probe.is_none()
                    && o.trigger_volume.is_none()
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

                    emissive_drive: o
                        .lights
                        .iter()
                        .map(crate::scene_light::emissive_drive)
                        .fold(0.0f32, f32::max),
                    manual_part_blends: self.manual_part_blends.get(&o.id).cloned().unwrap_or_default(),

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

    pub(crate) fn collect_render_lights(&self) -> Vec<RenderLight> {
        self.scene
            .objects
            .iter()
            .flat_map(|o| {
                o.lights
                    .iter()
                    .enumerate()
                    .filter(|(_, light)| light.mode.is_live())
                    .map(move |(i, light)| {

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

        assert_eq!(
            drive_of(r#"[{"intensity": 4.0, "enabled": false}]"#),
            0.0,
            "an off lamp must have a dark bulb",
        );
    }

    #[test]
    fn an_object_with_no_lights_never_glows() {

        assert_eq!(drive_of("[]"), 0.0);
    }

    #[test]
    fn two_bulbs_in_one_housing_take_the_brightest_not_the_sum() {

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

    use crate::scene::{Color3, GameObject, LightDef, LightKind, LightMode};
    use crate::scene_rig::SocketDef;
    use glam::{Quat, Vec3};

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

    #[test]
    fn a_pendant_lamp_aims_at_the_floor() {
        let (_, dir) = rendered(pendant_lamp());
        assert!(
            dir.y < -0.99,
            "the lamp's beam points {dir:?}, not down; the socket's rotation is \
             not reaching the renderer",
        );
    }

    #[test]
    fn a_pendant_lamp_emits_from_its_bulb() {
        let (pos, _) = rendered(pendant_lamp());
        assert!(
            (pos - Vec3::new(0.0, 3.1 - 1.18, -4.5)).length() < 1e-3,
            "the lamp emits from {pos:?} rather than from its socket",
        );
    }

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
