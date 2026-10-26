use serde::{Deserialize, Serialize};

use crate::scene_cuboid::Color3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum LightKind {
    #[default]
    Point,
    Spot,

    Directional,
}


fn default_light_color() -> Color3 {
    Color3(255, 255, 255, 255)
}
fn default_light_intensity() -> f32 {
    1.0
}
fn default_light_range() -> f32 {
    5.0
}
pub(crate) fn default_cone_angle() -> f32 {
    45.0
}

pub(crate) fn default_inner_cone_angle() -> f32 {
    0.0
}
fn default_glare_spread() -> f32 {
    0.15
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlareFacesDef {
    #[serde(default = "default_true")]
    pub front: bool,
    #[serde(default = "default_true")]
    pub back: bool,
    #[serde(default = "default_true")]
    pub left: bool,
    #[serde(default = "default_true")]
    pub right: bool,
    #[serde(default = "default_true")]
    pub top: bool,
    #[serde(default = "default_true")]
    pub bottom: bool,
}

impl Default for GlareFacesDef {
    fn default() -> Self {
        Self {
            front: true,
            back: true,
            left: true,
            right: true,
            top: true,
            bottom: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LightMode {

    #[default]
    Realtime,

    Baked,

    Stationary,
}

impl LightMode {

    pub fn is_live(self) -> bool {
        self != LightMode::Baked
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightDef {
    #[serde(default)]
    pub kind: LightKind,

    #[serde(default)]
    pub mode: LightMode,
    #[serde(default = "default_light_color")]
    pub color: Color3,
    #[serde(default = "default_light_intensity")]
    pub intensity: f32,
    #[serde(default = "default_light_range")]
    pub range: f32,
    #[serde(default = "default_cone_angle")]
    pub cone_angle_deg: f32,

    #[serde(default = "default_inner_cone_angle")]
    pub inner_cone_angle_deg: f32,
    #[serde(default)]
    pub glare_faces: GlareFacesDef,
    #[serde(default = "default_glare_spread")]
    pub glare_spread: f32,

    #[serde(default = "default_light_enabled")]
    pub enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<Color3>,
}

fn default_light_enabled() -> bool {
    true
}

impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::default(),
            mode: LightMode::default(),
            color: default_light_color(),
            intensity: default_light_intensity(),
            range: default_light_range(),
            cone_angle_deg: default_cone_angle(),
            inner_cone_angle_deg: default_inner_cone_angle(),
            glare_faces: GlareFacesDef::default(),
            glare_spread: default_glare_spread(),
            enabled: default_light_enabled(),
            socket: None,
            emissive: None,
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn a_light_defaults_to_realtime() {

        assert_eq!(LightDef::default().mode, LightMode::Realtime);
        let from_json: LightDef = serde_json::from_str("{}").unwrap();
        assert_eq!(from_json.mode, LightMode::Realtime);
    }

    #[test]
    fn a_scene_written_before_modes_existed_still_loads() {

        let old: LightDef = serde_json::from_str(
            r#"{"kind":"Point","intensity":5,"range":15}"#,
        ).unwrap();
        assert_eq!(old.mode, LightMode::Realtime);
        assert_eq!(old.intensity, 5.0);
    }

    #[test]
    fn a_scene_that_still_carries_the_removed_glow_fields_loads() {

        let with_glow: LightDef = serde_json::from_str(
            r#"{"kind":"Spot","intensity":5,"range":15,"cone_angle_deg":70,
                "glow_enabled":true,"glow_intensity":20,"glow_range":2.5}"#,
        )
        .expect("a scene carrying the old glow fields must still load");
        assert_eq!(with_glow.intensity, 5.0);
        assert_eq!(with_glow.cone_angle_deg, 70.0);
    }

    #[test]
    fn the_mode_survives_a_round_trip() {
        let mut l = LightDef::default();
        l.mode = LightMode::Baked;
        let back: LightDef = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back.mode, LightMode::Baked);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedLightPose {
    pub position: glam::Vec3,
    pub rotation: glam::Quat,
}

impl ResolvedLightPose {

    pub fn direction(&self) -> glam::Vec3 {
        (self.rotation * glam::Vec3::NEG_Z).normalize_or_zero()
    }
}

pub fn resolve_light_pose(
    light: &LightDef,
    object_pos: glam::Vec3,
    object_rot: glam::Quat,
    socket: Option<&crate::scene_rig::SocketDef>,
) -> ResolvedLightPose {
    let Some(socket) = socket else {
        return ResolvedLightPose { position: object_pos, rotation: object_rot };
    };
    let local_rot = glam::Quat::from_array(socket.local_rot).normalize();
    let rotation = (object_rot * local_rot).normalize();
    let position = if light.kind == LightKind::Directional {
        object_pos
    } else {
        object_pos + object_rot * glam::Vec3::from_array(socket.local_pos)
    };
    ResolvedLightPose { position, rotation }
}

pub const LAMP_RADIUS: f32 = 0.05;

pub fn emissive_drive(light: &LightDef) -> f32 {
    if !light.enabled {
        return 0.0;
    }
    match light.kind {
        LightKind::Point | LightKind::Spot => (light.intensity / (LAMP_RADIUS * LAMP_RADIUS)).max(0.0),
        LightKind::Directional => (light.intensity / default_light_intensity()).max(0.0),
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::scene_rig::SocketDef;
    use glam::{Quat, Vec3};

    fn socket_at(pos: [f32; 3], rot: Quat) -> SocketDef {
        SocketDef {
            name: "bulb".to_string(),
            local_pos: pos,
            local_rot: rot.to_array(),
            part: None,
        }
    }

    #[test]
    fn a_light_with_no_socket_does_not_move() {

        let l = LightDef::default();
        let p = Vec3::new(3.0, 2.0, -1.0);
        let r = Quat::from_rotation_y(0.7);
        let pose = resolve_light_pose(&l, p, r, None);
        assert_eq!(pose.position, p);
        assert_eq!(pose.rotation, r);
    }

    #[test]
    fn a_socket_offset_rides_the_objects_rotation() {

        let l = LightDef::default();
        let s = socket_at([0.0, 1.0, 0.0], Quat::IDENTITY);
        let turned = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let pose = resolve_light_pose(&l, Vec3::ZERO, turned, Some(&s));

        assert!(
            (pose.position - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5,
            "socket offset did not follow the object's rotation: {:?}",
            pose.position,
        );
    }

    #[test]
    fn the_socket_aims_the_beam() {

        let mut l = LightDef { kind: LightKind::Spot, ..Default::default() };
        l.socket = Some("bulb".to_string());

        let down = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let s = socket_at([0.0, 0.0, 0.0], down);
        let pose = resolve_light_pose(&l, Vec3::ZERO, Quat::IDENTITY, Some(&s));
        assert!(
            (pose.direction() - Vec3::NEG_Y).length() < 1e-5,
            "the socket did not aim the beam: {:?}",
            pose.direction(),
        );
    }

    #[test]
    fn a_directional_light_takes_the_rotation_and_ignores_the_offset() {

        let l = LightDef { kind: LightKind::Directional, ..Default::default() };
        let s = socket_at([5.0, 9.0, -2.0], Quat::from_rotation_x(0.5));
        let at = Vec3::new(1.0, 1.0, 1.0);
        let pose = resolve_light_pose(&l, at, Quat::IDENTITY, Some(&s));
        assert_eq!(pose.position, at, "a directional light must not be moved by a socket");
        assert!(
            (pose.rotation.to_array()[0] - Quat::from_rotation_x(0.5).to_array()[0]).abs() < 1e-5,
            "a directional light must still take the socket's rotation",
        );
    }

    #[test]
    fn a_switched_off_lamp_drives_nothing() {
        let mut l = LightDef::default();
        assert!(emissive_drive(&l) > 0.0, "a lamp defaults to on");
        l.enabled = false;
        assert_eq!(emissive_drive(&l), 0.0, "off has to mean off");
    }

    #[test]
    fn the_drive_follows_intensity_so_dimming_dims_the_bulb() {

        let dim = LightDef { intensity: default_light_intensity() * 0.25, ..Default::default() };
        let bright = LightDef { intensity: default_light_intensity() * 2.0, ..Default::default() };
        assert!(emissive_drive(&dim) < emissive_drive(&bright));
        assert!((emissive_drive(&bright) / emissive_drive(&dim) - 8.0).abs() < 1e-4, "in proportion");
    }

    #[test]
    fn a_bulb_is_as_bright_as_the_light_it_gives_off_at_its_own_radius() {
        for kind in [LightKind::Point, LightKind::Spot] {
            for intensity in [0.5f32, 3.0, 12.0] {
                let l = LightDef { kind, intensity, ..Default::default() };
                let bulb = emissive_drive(&l);

                let lit = |albedo: f32, d: f32| albedo * intensity / (d * d).max(LAMP_RADIUS * LAMP_RADIUS);
                assert!((lit(1.0, LAMP_RADIUS) - bulb).abs() < 1e-3 * bulb, "{kind:?} {intensity}: a white wall at the bulb");
                for (albedo, d) in [(1.0, 0.01), (0.8, 0.05), (0.5, 0.1), (0.9, 1.0)] {
                    assert!(lit(albedo, d) <= bulb * (1.0 + 1e-6), "{kind:?} {intensity}: lit at {d} m outshone the bulb");
                }
            }
        }
        let sun = LightDef { kind: LightKind::Directional, intensity: 2.0, ..Default::default() };
        assert_eq!(emissive_drive(&sun), 2.0);
    }

    #[test]
    fn every_new_field_defaults_so_old_scenes_are_unchanged() {
        let old: LightDef = serde_json::from_str(r#"{"kind": "Spot"}"#).expect("minimal light");
        assert!(old.enabled, "a light with no `enabled` must be on");
        assert!(old.socket.is_none(), "no socket means the old placement");
        assert!(old.emissive.is_none());
    }
}
