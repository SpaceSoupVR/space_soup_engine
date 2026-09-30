use serde::{Deserialize, Serialize};

use crate::scene_cuboid::Color3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightKind {
    Point,
    Spot,
    /// The sun. Parallel rays from infinitely far away.
    ///
    /// `position` and `range` are ignored -- the beam travels along `direction`
    /// and does not attenuate, which is what makes a sun a sun rather than a
    /// very bright bulb a long way up. The renderer treats the FIRST directional
    /// light in a scene as the shadow caster.
    Directional,
}

impl Default for LightKind {
    fn default() -> Self {
        Self::Point
    }
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
/// No hotspot: the beam fades from the axis all the way to the rim.
///
/// Zero rather than something prettier because it is what every spot light
/// authored before this field existed already did, and a nonzero default would
/// silently re-shape the beam in every scene on disk.
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

/// Where a light is evaluated: once, into a lightmap, or every frame.
///
/// This is the setting that decides whether a level can afford realistic
/// interior lighting on a headset, and it exists because a light must be one or
/// the other. Evaluated in both places it is counted TWICE -- the bake adds its
/// contribution to the texture and the shader adds it again -- so a room with
/// baked lamps would come out at double brightness, which reads as "the bake is
/// too bright" rather than as double counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LightMode {
    /// Uploaded to the GPU and shaded every frame.
    ///
    /// The default, because it is what every light did before baking existed
    /// and changing that silently would relight every scene already authored.
    /// Costs a light slot; casts a real-time shadow only if it is the sun or
    /// the first shadow-casting spot.
    #[default]
    Realtime,
    /// Computed once into the lightmap and never uploaded.
    ///
    /// Free at runtime and correctly occluded -- including by the walls of the
    /// room it is in, which no real-time point light can manage. Cannot move,
    /// and changing it needs a rebake.
    Baked,
    /// Shaded every frame, SHADOWED FROM THE BAKE: the direct light is
    /// evaluated per pixel -- the real cone, falloff and highlight -- while its
    /// visibility comes from a distance-field mask the baker writes into a
    /// channel of the brush atlas, and its bounce is baked like any lamp's.
    ///
    /// What Unreal calls a stationary light. `Baked` put the whole lamp into
    /// the lightmap, and a 34-degree pool a few lightmap texels wide was drawn
    /// by bilinear filtering as a blocky cross -- in the room and in every
    /// reflection of it (headset, 2026-09-27). Cannot move; changing its
    /// shadows needs a rebake, changing its colour or intensity does not.
    Stationary,
}

impl LightMode {
    /// Whether the renderer shades this light's DIRECT term every frame --
    /// true of everything but `Baked`, whose direct light is in the lightmap.
    pub fn is_live(self) -> bool {
        self != LightMode::Baked
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightDef {
    #[serde(default)]
    pub kind: LightKind,
    /// Baked once or shaded every frame. See [`LightMode`].
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
    /// Full angle of the beam's bright core, inside which there is no falloff.
    ///
    /// Between this and `cone_angle_deg` the beam smoothsteps to nothing, so
    /// the two together are what give a spot an EDGE. With the inner angle at
    /// zero the gradient spans the whole cone and a "45 degree spot" is a soft
    /// blob that never resolves into a 45 degree anything -- which is exactly
    /// how this looked before the field existed. Same convention as
    /// `cone_angle_deg`: a full angle, not a half-angle from the axis.
    #[serde(default = "default_inner_cone_angle")]
    pub inner_cone_angle_deg: f32,
    #[serde(default)]
    pub glare_faces: GlareFacesDef,
    #[serde(default = "default_glare_spread")]
    pub glare_spread: f32,

    /// Whether the lamp is on.
    ///
    /// Separate from deleting the light or setting `intensity` to zero, because
    /// those lose the authored value: a switch has to be able to turn a lamp
    /// back on at the brightness the author chose, and a flicker has to be able
    /// to return to it.
    ///
    /// Also what drives a fixture's emissive, so a bulb goes dark with its beam.
    #[serde(default = "default_light_enabled")]
    pub enabled: bool,

    /// Emit from a named socket on this object's mesh, rather than its origin.
    ///
    /// WHY A SOCKET AND NOT AN OFFSET
    ///
    /// A lamp's bulb is somewhere specific inside its housing, and where that
    /// is belongs to the MESH, not to each placement of it. An offset authored
    /// per instance is the same measurement typed out again for every lamp in
    /// the level, and typed wrong for at least one of them. The socket travels
    /// with the asset, so the light lands correctly wherever the fixture goes.
    ///
    /// The socket's ROTATION is what aims a spot: its forward axis is the beam.
    /// `None` keeps the old behaviour exactly -- the light is at the object's
    /// own position and orientation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,

    /// The emissive colour of this fixture's glowing parts, if it has any.
    ///
    /// Usually `None`: a glTF asset that carries an emissive material already
    /// says which parts glow and in what colour, and that is the better place
    /// for it. This is the override for a mesh that does not, and for tinting a
    /// shared fixture asset per instance.
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
        // Every light behaved this way before baking existed. Flipping the
        // default would silently relight every scene already authored, and the
        // change would look like a rendering regression rather than a new
        // feature nobody opted into.
        assert_eq!(LightDef::default().mode, LightMode::Realtime);
        let from_json: LightDef = serde_json::from_str("{}").unwrap();
        assert_eq!(from_json.mode, LightMode::Realtime);
    }

    #[test]
    fn a_scene_written_before_modes_existed_still_loads() {
        // The field is #[serde(default)], so an older scene file has no `mode`
        // at all and must not fail to parse.
        let old: LightDef = serde_json::from_str(
            r#"{"kind":"Point","intensity":5,"range":15}"#,
        ).unwrap();
        assert_eq!(old.mode, LightMode::Realtime);
        assert_eq!(old.intensity, 5.0);
    }

    #[test]
    fn a_scene_that_still_carries_the_removed_glow_fields_loads() {
        // The editor used to add an omnidirectional PointLight at every bulb so
        // a fixture read as an object in the viewport. Nothing here ever read
        // its settings -- glow_intensity and glow_range were parsed and
        // ignored -- so the preview showed light the game would not draw, and a
        // spot's beam sat inside a bubble that no amount of aiming could move.
        //
        // The fields are gone, but scenes on disk still carry them
        // (game/scenes/lobby_backup.json has three). Serde ignores unknown keys
        // unless told otherwise, and this test is what says so out loud: delete
        // it and a stray #[serde(deny_unknown_fields)] would turn every one of
        // those scenes into a load failure with nothing to explain why.
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

/// Where a light actually sits and points, once its socket is resolved.
///
/// Returned in WORLD space, so every consumer -- the renderer, the baker, the
/// editor's preview -- agrees without each re-deriving it. They have disagreed
/// before, and a light that bakes in one place and renders in another is very
/// hard to read as a socket problem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedLightPose {
    pub position: glam::Vec3,
    pub rotation: glam::Quat,
}

impl ResolvedLightPose {
    /// The direction a spot or directional light emits along.
    ///
    /// -Z, the glTF forward convention, which is also what the socket's own
    /// rotation is authored against.
    pub fn direction(&self) -> glam::Vec3 {
        (self.rotation * glam::Vec3::NEG_Z).normalize_or_zero()
    }
}

/// Resolve a light's pose against the object carrying it.
///
/// With no socket the light is simply at the object: that is what every scene
/// authored so far expects, and it must not move.
///
/// With a socket the light rides the socket's local transform, composed with the
/// object's. This is what makes a fixture asset reusable -- the bulb's position
/// inside its housing is a property of the mesh, so placing the lamp anywhere
/// puts the light in the right place with nothing typed per instance.
///
/// **Directional lights take only the ROTATION.** A directional light has no
/// position -- it is a direction over the whole world -- so offsetting one by a
/// socket computes a number with no visual effect whatsoever, and then invites
/// someone to spend an afternoon working out why moving it changes nothing.
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

/// The radius the lighting gives every lamp: the renderer's and the baker's
/// `LAMP_RADIUS`, inside which a lamp's inverse square stops growing -- a ball
/// of this radius, not a point. Pinned to both by the baker's tests.
pub const LAMP_RADIUS: f32 = 0.05;

/// How hard a fixture's emissive material should be driven right now: the
/// RADIANCE of its bulb, in the renderer's units, which the material's
/// `emissiveFactor` colours (`mesh_pipeline`: factor x mask x drive).
///
/// The bridge between a light entity and the mesh around it: zero when the lamp
/// is off, and set by its intensity when on, so a bulb brightens and dims with
/// its own beam without anything having to synchronise the two.
///
/// SET BY THE LIGHT IT GIVES OFF. The lighting treats a lamp as a ball of
/// [`LAMP_RADIUS`]: a surface at distance `d` facing it is lit to
/// `albedo x intensity / max(d^2, LAMP_RADIUS^2)`. A ball that gives off that
/// light has, in the same units, the radiance `intensity / LAMP_RADIUS^2` -- so
/// its bulb, drawn so, is brighter than anything its light lights by at least
/// `1 / albedo`, as a real bulb is. Driven at the intensity alone, 400 times
/// dimmer, the inside of a sconce's shade, lit from five centimetres, outshone
/// its own bulb twenty to eighty times, and every reflection showed the bulb as
/// a grey disc in a white mouth (user, 2026-09-30: "make the bulb brighter than
/// the shade's inside, like a real bulb"). A spot's bulb is the same ball; its
/// cone only aims what it gives off. The sun has no bulb: a directional light
/// drives at its intensity, as before.
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
        // Every scene authored before sockets existed goes through this path.
        let l = LightDef::default();
        let p = Vec3::new(3.0, 2.0, -1.0);
        let r = Quat::from_rotation_y(0.7);
        let pose = resolve_light_pose(&l, p, r, None);
        assert_eq!(pose.position, p);
        assert_eq!(pose.rotation, r);
    }

    #[test]
    fn a_socket_offset_rides_the_objects_rotation() {
        // The bulb sits 1m up inside its housing. Rotate the LAMP and the bulb
        // must swing with it -- adding the socket offset in world space instead
        // would leave every rotated fixture emitting from beside itself.
        let l = LightDef::default();
        let s = socket_at([0.0, 1.0, 0.0], Quat::IDENTITY);
        let turned = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let pose = resolve_light_pose(&l, Vec3::ZERO, turned, Some(&s));
        // +Y rotated 90 degrees about Z becomes -X.
        assert!(
            (pose.position - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5,
            "socket offset did not follow the object's rotation: {:?}",
            pose.position,
        );
    }

    #[test]
    fn the_socket_aims_the_beam() {
        // The whole reason a spot housing is worth authoring: the socket's
        // forward axis IS the beam, so a lamp modelled pointing down emits down
        // with nothing set per instance.
        let mut l = LightDef { kind: LightKind::Spot, ..Default::default() };
        l.socket = Some("bulb".to_string());
        // Rotate -Z to point at -Y: a quarter turn about X.
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
        // A directional light has no position. Offsetting one computes a number
        // with no visual effect at all, and then invites an afternoon spent
        // working out why moving it changes nothing.
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
        // The property that makes flicker and dimming fall out for free rather
        // than needing their own synchronisation step.
        let dim = LightDef { intensity: default_light_intensity() * 0.25, ..Default::default() };
        let bright = LightDef { intensity: default_light_intensity() * 2.0, ..Default::default() };
        assert!(emissive_drive(&dim) < emissive_drive(&bright));
        assert!((emissive_drive(&bright) / emissive_drive(&dim) - 8.0).abs() < 1e-4, "in proportion");
    }

    /// A BULB OUTSHINES WHAT IT LIGHTS: for a point or a spot of any intensity,
    /// the brightest a white surface can be lit -- at or inside the lamp's
    /// radius, where the lighting stops growing -- is exactly the bulb's
    /// radiance, and anything farther or darker is dimmer. The sun keeps its
    /// old drive.
    #[test]
    fn a_bulb_is_as_bright_as_the_light_it_gives_off_at_its_own_radius() {
        for kind in [LightKind::Point, LightKind::Spot] {
            for intensity in [0.5f32, 3.0, 12.0] {
                let l = LightDef { kind, intensity, ..Default::default() };
                let bulb = emissive_drive(&l);
                // The lighting's inverse square, clamped at the lamp's radius
                // (the renderer's `light_contribution_split`), window 1.
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
