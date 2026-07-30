use anyhow::{Context, Result};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::events::Hand;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color3(pub u8, pub u8, pub u8, pub u8);

impl Default for Color3 {
    fn default() -> Self {
        Self(220, 60, 60, 255)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CuboidStyle {
    Solid,
    Wireframe,
    SolidAndWire,
}

impl Default for CuboidStyle {
    fn default() -> Self {
        Self::Solid
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuboidDef {
    #[serde(default)]
    pub position: Vec3,
    #[serde(default = "default_half_size")]
    pub half_size: Vec3,
    #[serde(default = "default_rotation")]
    pub rotation: Quat,
    #[serde(default)]
    pub color: Color3,
    #[serde(default)]
    pub wire_color: Color3,
    #[serde(default)]
    pub style: CuboidStyle,
    /// 0.0 = not reflective, 1.0 = fully mirror-blended. Approximated via
    /// screen-space reflections (see space_soup/src/renderer/ssr.rs) -- only
    /// cuboid objects support this (mesh/GLB objects already use all 4 of
    /// wgpu's default max bind groups).
    #[serde(default)]
    pub reflectivity: f32,
}

fn default_half_size() -> Vec3 {
    Vec3::splat(0.5)
}
fn default_rotation() -> Quat {
    Quat::IDENTITY
}

impl Default for CuboidDef {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            half_size: default_half_size(),
            rotation: Quat::IDENTITY,
            color: Color3::default(),
            wire_color: Color3(200, 200, 255, 255),
            style: CuboidStyle::default(),
            reflectivity: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshRef {
    pub path: String,
    #[serde(default = "default_mesh_scale")]
    pub scale: Vec3,
    #[serde(default = "default_mesh_rotation")]
    pub rotation_offset: Quat,
}

fn default_mesh_scale() -> Vec3 {
    Vec3::ONE
}
fn default_mesh_rotation() -> Quat {
    Quat::IDENTITY
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Default for Easing {
    fn default() -> Self {
        Self::Linear
    }
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: f32,
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
    pub color: Option<Color3>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub name: String,
    pub keyframes: Vec<Keyframe>,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default)]
    pub looping: bool,
}

impl Animation {
    pub fn duration(&self) -> f32 {
        self.keyframes.iter().map(|k| k.t).fold(0.0_f32, f32::max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayMode {
    Simultaneous,
    Sequential,
}

impl Default for PlayMode {
    fn default() -> Self {
        Self::Simultaneous
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingScope {
    ContextualHold,
    GlobalAnywhere,
}

impl Default for BindingScope {
    fn default() -> Self {
        Self::ContextualHold
    }
}

pub const BINDING_BUTTONS: [&str; 6] = ["btn_a", "btn_b", "btn_x", "btn_y", "trigger", "grip"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationBinding {
    pub button: String,
    pub animation: String,
    #[serde(default)]
    pub play_mode: PlayMode,
    #[serde(default)]
    pub scope: BindingScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigAttachmentDef {
    pub joint: String,
    #[serde(default)]
    pub offset: [f32; 3],
}

fn identity_quat_arr() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn one_vec3_arr() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GripPoseDef {
    #[serde(default)]
    pub hand_offset_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub hand_offset_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
}

impl Default for GripPoseDef {
    fn default() -> Self {
        Self {
            hand_offset_pos: [0.0, 0.0, 0.0],
            hand_offset_rot: identity_quat_arr(),
            hand_offset_scale: one_vec3_arr(),
            finger_curl: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GripKind {
    Snap,

    Free,

    Pinch,
}

impl Default for GripKind {
    fn default() -> Self {
        Self::Snap
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GripPointDef {
    pub name: String,
    #[serde(default)]
    pub kind: GripKind,
    #[serde(default)]
    pub hand: crate::events::Hand,
    #[serde(default)]
    pub local_pos: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub local_rot: [f32; 4],

    #[serde(default = "one_vec3_arr")]
    pub hand_offset_scale: [f32; 3],
    #[serde(default)]
    pub finger_curl: HashMap<String, f32>,
    #[serde(default)]
    pub grab_range: Option<f32>,
}

fn default_slider_axis() -> [f32; 3] {
    [1.0, 0.0, 0.0]
}
fn default_slider_travel() -> f32 {
    0.02
}
fn default_slider_stiffness() -> f32 {
    400.0
}
fn default_slider_damping() -> f32 {
    20.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliderJointDef {
    pub parent: String,

    #[serde(default = "default_slider_axis")]
    pub axis: [f32; 3],

    #[serde(default = "default_slider_travel")]
    pub travel: f32,

    #[serde(default = "default_slider_stiffness")]
    pub spring_stiffness: f32,
    #[serde(default = "default_slider_damping")]
    pub spring_damping: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TerrainColliderDef {
    #[serde(default)]
    pub node_filter: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BodyMode {
    Static,

    Kinematic,

    Dynamic,
}

fn default_body_mode() -> BodyMode {
    BodyMode::Dynamic
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Box,
    Sphere { radius: f32 },
    Capsule { radius: f32, half_height: f32 },
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Box
    }
}

fn default_friction() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigidBodyDef {
    #[serde(default = "default_body_mode")]
    pub mode: BodyMode,
    #[serde(default)]
    pub shape: ColliderShape,

    #[serde(default)]
    pub mass: Option<f32>,
    #[serde(default = "default_friction")]
    pub friction: f32,
    #[serde(default)]
    pub restitution: f32,

    #[serde(default)]
    pub linear_velocity: [f32; 3],

    #[serde(default)]
    pub respawn_interval: Option<f32>,

    #[serde(default)]
    pub collider_half_size: Option<[f32; 3]>,

    #[serde(default)]
    pub collider_offset: [f32; 3],
}

impl Default for RigidBodyDef {
    fn default() -> Self {
        Self {
            mode: default_body_mode(),
            shape: ColliderShape::default(),
            mass: None,
            friction: default_friction(),
            restitution: 0.0,
            respawn_interval: None,
            linear_velocity: [0.0, 0.0, 0.0],
            collider_half_size: None,
            collider_offset: [0.0, 0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightKind {
    Point,
    Spot,
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
fn default_cone_angle() -> f32 {
    45.0
}
fn default_glow_range() -> f32 {
    // A real light bulb's own housing and immediate surroundings should
    // read as lit even outside a spotlight's narrow cone -- 0.6 barely
    // cleared the fixture itself (confirmed by screenshotting the rendered
    // result: nearby geometry stayed dark), so this needs to reach a couple
    // meters out to actually brighten neighboring objects/floor.
    2.5
}
fn default_glow_intensity() -> f32 {
    // 90 gave a strong, clearly visible bleed onto surfaces ~1.5m away, but
    // also blew the fixture's own nearby geometry (e.g. the crossbar between
    // the two work_light_stand lamp heads, mere centimeters from the light)
    // out to solid white -- an omnidirectional point light's falloff scales
    // with 1/distance^2, so any intensity strong enough to read clearly at
    // range will overexpose anything sitting right next to it. Pixel-swept
    // this down until the near-field blowout was gone (confirmed via
    // screenshot: 0 fully-white-saturated pixels on that crossbar, down from
    // ~10% of the sampled region at 90) -- the medium-range bleed is more
    // modest as a result, but not blowing out the fixture itself matters more.
    8.0
}
fn default_glare_spread() -> f32 {
    0.15
}
fn default_true() -> bool {
    true
}

/// Which of the 6 directions (relative to the light's own facing --
/// front/back along its aim, the rest perpendicular to that) show a small
/// bulb-icon marker in the editor/preview viewport. All default to visible
/// (so the fixture reads as an obvious lit bulb from any angle); toggling
/// one off is how you say e.g. "no marker visible from directly behind this
/// fixture". Purely a web-editor visual concern, like glare_spread below --
/// not read by collect_render_lights or by quest_app/space_soup at all.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightDef {
    #[serde(default)]
    pub kind: LightKind,
    #[serde(default = "default_light_color")]
    pub color: Color3,
    #[serde(default = "default_light_intensity")]
    pub intensity: f32,
    #[serde(default = "default_light_range")]
    pub range: f32,
    #[serde(default = "default_cone_angle")]
    pub cone_angle_deg: f32,
    /// A real light bulb bleeds a soft glow onto its own housing and
    /// immediate surroundings in every direction, not just down whatever
    /// narrow beam/cone it's aimed at -- a spotlight alone leaves the
    /// fixture itself and anything near it looking unlit. This is the range
    /// (meters) of a small omnidirectional fill light created alongside the
    /// main one to cover that -- real light hitting nearby geometry, not a
    /// visible glow sprite (see glare_faces/glare_spread below for the
    /// actual bulb-icon markers).
    #[serde(default = "default_glow_range")]
    pub glow_range: f32,
    /// Intensity (same WGSL-pipeline units as `intensity`) of that local
    /// fill light.
    #[serde(default = "default_glow_intensity")]
    pub glow_intensity: f32,
    /// Whether the local fill light (glow_range/glow_intensity) is on at
    /// all -- like glare_faces/glare_spread, this is a web-editor-only
    /// preview concern (not read by collect_render_lights or by
    /// quest_app/space_soup), so a fixture can be previewed with just its
    /// spot/point beam and no near-field bleed.
    #[serde(default = "default_true")]
    pub glow_enabled: bool,
    /// Which of the 6 directions show a bulb-icon marker -- see
    /// GlareFacesDef.
    #[serde(default)]
    pub glare_faces: GlareFacesDef,
    /// How far (meters) each enabled marker sits from the light's own
    /// position, along its own direction -- clears the fixture's own opaque
    /// housing mesh (a marker sitting exactly at the light's position gets
    /// depth-occluded by it, confirmed by screenshotting the rendered
    /// result) and is what actually spreads the 6 markers into visually
    /// separate icons instead of one at the center.
    #[serde(default = "default_glare_spread")]
    pub glare_spread: f32,
}

impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::default(),
            color: default_light_color(),
            intensity: default_light_intensity(),
            range: default_light_range(),
            cone_angle_deg: default_cone_angle(),
            glow_range: default_glow_range(),
            glow_intensity: default_glow_intensity(),
            glow_enabled: default_true(),
            glare_faces: GlareFacesDef::default(),
            glare_spread: default_glare_spread(),
        }
    }
}

fn default_volume() -> f32 {
    1.0
}
fn default_pitch() -> f32 {
    1.0
}
fn default_min_distance() -> f32 {
    1.0
}
fn default_max_distance() -> f32 {
    10.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundSourceDef {
    pub clip: String,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_pitch")]
    pub pitch: f32,
    #[serde(default = "default_min_distance")]
    pub min_distance: f32,
    #[serde(default = "default_max_distance")]
    pub max_distance: f32,
    #[serde(default)]
    pub looping: bool,
    #[serde(default)]
    pub autoplay: bool,
    #[serde(default)]
    pub directional: bool,
    #[serde(default = "default_cone_angle")]
    pub cone_angle_deg: f32,
}

fn default_particle_size() -> f32 {
    0.03
}
fn default_spawn_rate() -> f32 {
    5.0
}
fn default_particle_color() -> Color3 {
    Color3(255, 255, 255, 200)
}
fn default_particle_lifetime() -> f32 {
    2.0
}
fn default_particle_speed() -> f32 {
    0.3
}
fn default_spread_deg() -> f32 {
    15.0
}
fn default_size_growth() -> f32 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleEmitterDef {
    #[serde(default = "default_particle_size")]
    pub particle_size: f32,
    #[serde(default = "default_spawn_rate")]
    pub spawn_rate: f32,
    #[serde(default = "default_particle_color")]
    pub color: Color3,
    #[serde(default = "default_particle_lifetime")]
    pub lifetime: f32,
    #[serde(default = "default_particle_speed")]
    pub speed: f32,
    #[serde(default = "default_spread_deg")]
    pub spread_deg: f32,
    /// How much a particle grows over its life: final_size = particle_size
    /// * (1 + size_growth) at the end of its lifetime (0.0 = constant size,
    /// 1.0 = doubles, 2.0 = triples) -- real smoke/dust expands as it
    /// disperses; a flat particle size reads as rain/embers instead.
    /// Defaults to 0 so every existing emitter's look is unchanged.
    #[serde(default = "default_size_growth")]
    pub size_growth: f32,
}

impl Default for ParticleEmitterDef {
    fn default() -> Self {
        Self {
            particle_size: default_particle_size(),
            spawn_rate: default_spawn_rate(),
            color: default_particle_color(),
            lifetime: default_particle_lifetime(),
            speed: default_particle_speed(),
            spread_deg: default_spread_deg(),
            size_growth: default_size_growth(),
        }
    }
}

fn default_laser_color() -> Color3 {
    Color3(255, 0, 0, 255)
}
fn default_laser_max_distance() -> f32 {
    20.0
}
fn default_beam_width() -> f32 {
    0.02
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaserDef {
    #[serde(default = "default_laser_color")]
    pub color: Color3,
    #[serde(default = "default_laser_max_distance")]
    pub max_distance: f32,
    #[serde(default = "default_beam_width")]
    pub beam_width: f32,
}

impl Default for LaserDef {
    fn default() -> Self {
        Self {
            color: default_laser_color(),
            max_distance: default_laser_max_distance(),
            beam_width: default_beam_width(),
        }
    }
}

/// Optic family. Selecting a family is how an author gets sane physical numbers
/// without knowing optics -- a red dot has an enormous eye box and no
/// magnification, a precision scope has a tiny one. Speed of target acquisition
/// is meant to fall out of these physical differences rather than out of any
/// assist curve, so the family is the main gameplay lever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpticClass {
    ReflexRedDot,
    Holographic,
    Lpvo,
    FixedPrism,
    PrecisionScope,
    Binocular,
}

impl Default for OpticClass {
    fn default() -> Self {
        Self::ReflexRedDot
    }
}

/// How much the optic magnifies. `Stepped` carries the detent values a variable
/// optic clicks through; `Continuous` sweeps freely between the bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MagnificationDef {
    Fixed(f32),
    Stepped { steps: Vec<f32> },
    Continuous { min: f32, max: f32 },
}

impl Default for MagnificationDef {
    fn default() -> Self {
        Self::Fixed(1.0)
    }
}

impl MagnificationDef {
    /// Lowest magnification this optic can be set to.
    pub fn min(&self) -> f32 {
        match self {
            Self::Fixed(m) => *m,
            Self::Stepped { steps } => steps.iter().copied().fold(f32::INFINITY, f32::min),
            Self::Continuous { min, .. } => *min,
        }
    }

    /// Highest magnification this optic can be set to. Drives the worst-case
    /// exit pupil, which is what makes a high-power optic physically demanding.
    pub fn max(&self) -> f32 {
        match self {
            Self::Fixed(m) => *m,
            Self::Stepped { steps } => steps.iter().copied().fold(0.0_f32, f32::max),
            Self::Continuous { max, .. } => *max,
        }
    }

    pub fn is_variable(&self) -> bool {
        !matches!(self, Self::Fixed(_))
    }
}

/// How the player changes magnification. Authored per optic because the right
/// ergonomics differ per game and per device: a hunting scope, an LPVO throw
/// lever and a pair of binoculars all want different controls.
///
/// `PhysicalRing`/`PhysicalWheel` are driven by the existing grab/grip
/// interaction system against a named node on the mesh, so they add no new
/// interaction machinery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MagnificationControlDef {
    /// Fixed optic, or magnification is not player-controllable.
    None,
    /// Cycle through the authored steps with a button.
    ButtonStep { button: String, wrap: bool },
    /// Continuous control from an analog axis.
    Axis { source: String, sensitivity: f32 },
    /// Off-hand grabs the magnification ring on the weapon and rotates it.
    PhysicalRing {
        ring_node: String,
        rotation_axis: [f32; 3],
        angle_range_deg: f32,
        detents: bool,
    },
    /// Centre focus/zoom wheel, the pattern binoculars actually use.
    PhysicalWheel {
        wheel_node: String,
        rotation_axis: [f32; 3],
        turns: f32,
    },
    /// Driven only by game script.
    ScriptOnly,
}

impl Default for MagnificationControlDef {
    fn default() -> Self {
        Self::None
    }
}

/// Shape the magnified image is clipped to inside the ocular.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LensClipShape {
    Circle,
    Ellipse,
    MeshMask,
}

impl Default for LensClipShape {
    fn default() -> Self {
        Self::Circle
    }
}

/// One optical path through the device: an objective that gathers the image and
/// an ocular the player looks into. A rifle scope has exactly one; binoculars
/// have two, separated by the interpupillary distance, which is what produces
/// genuine stereo magnification rather than a flat magnified image shown to
/// both eyes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpticalPathDef {
    /// Mesh node marking the front (objective) lens. The scope camera is
    /// anchored here -- NOT at the eye -- because that is where the image is
    /// actually formed, and it is why the view stays correct as the head moves.
    ///
    /// Optional: many weapon models carry no named lens nodes (ours does not),
    /// in which case `objective_offset` is used instead.
    #[serde(default)]
    pub objective_node: String,
    /// Mesh node marking the rear (ocular) lens the player looks into. The
    /// magnified image is masked to this node's projected area.
    #[serde(default)]
    pub ocular_node: String,
    /// Objective position in object-local metres, used when `objective_node` is
    /// empty. Lets an author place an optic on a model with no lens geometry
    /// rather than blocking on new art.
    #[serde(default)]
    pub objective_offset: [f32; 3],
    /// Ocular position in object-local metres, used when `ocular_node` is empty.
    #[serde(default)]
    pub ocular_offset: [f32; 3],
    /// Radius of the ocular glass in metres. Drives both the on-screen mask and
    /// the coverage gating that skips the scope pass when it is a speck.
    #[serde(default = "default_ocular_radius_m")]
    pub ocular_radius_m: f32,
    #[serde(default)]
    pub clip_shape: LensClipShape,
    #[serde(default = "default_edge_feather_px")]
    pub edge_feather_px: f32,
}

fn default_edge_feather_px() -> f32 {
    2.0
}
fn default_ocular_radius_m() -> f32 {
    0.018
}

impl Default for OpticalPathDef {
    fn default() -> Self {
        Self {
            objective_node: String::new(),
            ocular_node: String::new(),
            objective_offset: [0.0, 0.0, 0.0],
            ocular_offset: [0.0, 0.0, 0.0],
            ocular_radius_m: default_ocular_radius_m(),
            clip_shape: LensClipShape::default(),
            edge_feather_px: default_edge_feather_px(),
        }
    }
}

impl OpticalPathDef {
    /// True when this path is positioned by authored offsets rather than by
    /// named mesh nodes.
    pub fn uses_offsets(&self) -> bool {
        self.objective_node.is_empty() || self.ocular_node.is_empty()
    }

    /// Distance from ocular to objective in metres -- the physical length of the
    /// optic, used to derive its axis when placed by offsets.
    pub fn body_length_m(&self) -> f32 {
        let o = Vec3::from_array(self.objective_offset);
        let e = Vec3::from_array(self.ocular_offset);
        (o - e).length()
    }
}

/// The device's optical paths. Kept as an enum rather than a bare `Vec` so the
/// renderer can tell "one path shared by both eyes" from "one path per eye"
/// without inspecting lengths -- those are different compositing rules, not
/// different counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OpticalPathsDef {
    /// Single path. Because a parallax-free optic is collimated, its image is
    /// view-independent, so ONE render serves both eyes; eye position only
    /// affects the eye-box vignette.
    Monocular { path: OpticalPathDef },
    /// Two paths, one per eye. Requires two renders and yields real magnified
    /// depth.
    Binocular {
        left: OpticalPathDef,
        right: OpticalPathDef,
        ipd_mm: f32,
    },
}

impl Default for OpticalPathsDef {
    fn default() -> Self {
        Self::Monocular { path: OpticalPathDef::default() }
    }
}

impl OpticalPathsDef {
    /// Number of scope renders this device costs per frame.
    pub fn render_count(&self) -> usize {
        match self {
            Self::Monocular { .. } => 1,
            Self::Binocular { .. } => 2,
        }
    }
}

/// Which focal plane the reticle lives in. First focal plane scales with
/// magnification so subtensions stay valid at any power; second focal plane
/// keeps a constant apparent size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReticleFocalPlane {
    First,
    Second,
}

impl Default for ReticleFocalPlane {
    fn default() -> Self {
        Self::Second
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReticleDef {
    /// Named reticle style, resolved to art by the renderer.
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub focal_plane: ReticleFocalPlane,
    #[serde(default = "default_reticle_color")]
    pub color: Color3,
    #[serde(default = "default_reticle_brightness")]
    pub brightness: f32,
}

fn default_reticle_color() -> Color3 {
    Color3(255, 40, 40, 255)
}
fn default_reticle_brightness() -> f32 {
    1.0
}

impl Default for ReticleDef {
    fn default() -> Self {
        Self {
            style: String::new(),
            focal_plane: ReticleFocalPlane::default(),
            color: default_reticle_color(),
            brightness: default_reticle_brightness(),
        }
    }
}

/// Ballistic zero. Height-over-bore matters because the optic sits above the
/// barrel, so the sight line and the projectile path cross at the zero distance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroDef {
    #[serde(default = "default_zero_distance_m")]
    pub distance_m: f32,
    #[serde(default = "default_height_over_bore_mm")]
    pub height_over_bore_mm: f32,
}

fn default_zero_distance_m() -> f32 {
    100.0
}
fn default_height_over_bore_mm() -> f32 {
    38.0
}

impl Default for ZeroDef {
    fn default() -> Self {
        Self {
            distance_m: default_zero_distance_m(),
            height_over_bore_mm: default_height_over_bore_mm(),
        }
    }
}

/// Render quality tier for the scope pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpticQualityTier {
    Low,
    Balanced,
    Ultra,
}

impl Default for OpticQualityTier {
    fn default() -> Self {
        Self::Balanced
    }
}

impl OpticQualityTier {
    /// Preferred square edge length of the scope render target.
    pub fn target_resolution(&self) -> u32 {
        match self {
            Self::Low => 512,
            Self::Balanced => 768,
            Self::Ultra => 1024,
        }
    }
}

/// A magnifying optic: weapon sight, scope, or binoculars.
///
/// Authored fields are the physical numbers a real spec sheet carries. Everything
/// the renderer needs beyond that -- exit pupil, eye-box size, apparent field,
/// scope-camera FOV -- is DERIVED (see the `derived_*` methods), deliberately not
/// authored. Hand-dialled optical constants were how the previous rig
/// calibration ended up over 30 degrees wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpticDef {
    #[serde(default)]
    pub class: OpticClass,
    #[serde(default)]
    pub magnification: MagnificationDef,
    #[serde(default)]
    pub magnification_control: MagnificationControlDef,
    /// Front lens diameter. With magnification this sets the exit pupil, and
    /// therefore how forgiving the optic is to get behind.
    #[serde(default = "default_objective_diameter_mm")]
    pub objective_diameter_mm: f32,
    /// True (world) field of view at 1x. Apparent field = this * magnification.
    #[serde(default = "default_true_fov_deg")]
    pub true_fov_deg_at_1x: f32,
    /// Distance behind the ocular where the full image is visible. Real scopes
    /// need at least ~50mm to clear recoil.
    #[serde(default = "default_eye_relief_mm")]
    pub eye_relief_mm: f32,
    #[serde(default)]
    pub paths: OpticalPathsDef,
    #[serde(default)]
    pub reticle: Option<ReticleDef>,
    #[serde(default)]
    pub zero: Option<ZeroDef>,
    #[serde(default)]
    pub quality: OpticQualityTier,
}

fn default_objective_diameter_mm() -> f32 {
    40.0
}
fn default_true_fov_deg() -> f32 {
    24.0
}
fn default_eye_relief_mm() -> f32 {
    90.0
}

impl Default for OpticDef {
    fn default() -> Self {
        Self {
            class: OpticClass::default(),
            magnification: MagnificationDef::default(),
            magnification_control: MagnificationControlDef::default(),
            objective_diameter_mm: default_objective_diameter_mm(),
            true_fov_deg_at_1x: default_true_fov_deg(),
            eye_relief_mm: default_eye_relief_mm(),
            paths: OpticalPathsDef::default(),
            reticle: None,
            zero: None,
            quality: OpticQualityTier::default(),
        }
    }
}

impl OpticDef {
    /// Exit pupil diameter = objective / magnification. This IS the eye-box
    /// diameter, which is why a 4x40 optic is easy to get behind (10mm) and a
    /// 16x40 is not (2.5mm). Derived, never authored.
    pub fn derived_exit_pupil_mm(&self, magnification: f32) -> f32 {
        if magnification <= f32::EPSILON {
            return self.objective_diameter_mm;
        }
        self.objective_diameter_mm / magnification
    }

    /// Tightest eye box this optic will ever present (at maximum power).
    pub fn derived_min_exit_pupil_mm(&self) -> f32 {
        self.derived_exit_pupil_mm(self.magnification.max())
    }

    /// World field of view at a given magnification -- this is the FOV the scope
    /// camera renders with.
    pub fn derived_true_fov_deg(&self, magnification: f32) -> f32 {
        if magnification <= f32::EPSILON {
            return self.true_fov_deg_at_1x;
        }
        self.true_fov_deg_at_1x / magnification
    }

    /// Field of view the image appears to fill for the player. Stays roughly
    /// constant across magnification, which is why higher power shows less world
    /// through the same apparent circle.
    pub fn derived_apparent_fov_deg(&self) -> f32 {
        self.true_fov_deg_at_1x
    }

    /// Scope renders required per frame for this device.
    pub fn render_count(&self) -> usize {
        self.paths.render_count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameObject {
    pub id: String,

    #[serde(default)]
    pub cuboid: CuboidDef,

    #[serde(default)]
    pub mesh: Option<MeshRef>,

    #[serde(default)]
    pub is_trigger: bool,

    #[serde(default)]
    pub hidden: bool,

    #[serde(default)]
    pub script: Option<String>,

    #[serde(default)]
    pub animations: Vec<Animation>,

    #[serde(default)]
    pub animation_bindings: Vec<AnimationBinding>,

    #[serde(default)]
    pub rig_attachment: Option<RigAttachmentDef>,

    #[serde(default, rename = "grip_pose", skip_serializing_if = "Option::is_none")]
    pub grip_pose_legacy: Option<GripPoseDef>,

    #[serde(default)]
    pub grip_pose_left: Option<GripPoseDef>,

    #[serde(default)]
    pub grip_pose_right: Option<GripPoseDef>,

    #[serde(default)]
    pub rigid_body: Option<RigidBodyDef>,

    #[serde(default)]
    pub grip_points: Vec<GripPointDef>,

    #[serde(default)]
    pub slider_joint: Option<SliderJointDef>,

    #[serde(default)]
    pub terrain_collider: Option<TerrainColliderDef>,

    #[serde(default)]
    pub light: Option<LightDef>,

    #[serde(default)]
    pub sound: Option<SoundSourceDef>,

    #[serde(default)]
    pub particle_emitter: Option<ParticleEmitterDef>,

    #[serde(default)]
    pub laser: Option<LaserDef>,

    #[serde(default)]
    pub optic: Option<OpticDef>,
}

impl GameObject {
    pub fn find_animation(&self, name: &str) -> Option<&Animation> {
        self.animations.iter().find(|a| a.name == name)
    }

    pub fn grip_point(&self, name: &str) -> Option<&GripPointDef> {
        self.grip_points.iter().find(|p| p.name == name)
    }

    pub fn grip_pose(&self, hand: Hand) -> Option<&GripPoseDef> {
        match hand {
            Hand::Left => self.grip_pose_left.as_ref(),
            Hand::Right => self.grip_pose_right.as_ref(),
        }
    }

    pub fn grip_pose_mut(&mut self, hand: Hand) -> &mut Option<GripPoseDef> {
        match hand {
            Hand::Left => &mut self.grip_pose_left,
            Hand::Right => &mut self.grip_pose_right,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    #[serde(default)]
    pub objects: Vec<GameObject>,
}

impl Scene {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read scene {}", path.display()))?;
        let mut scene: Scene = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse scene {}", path.display()))?;
        for obj in &mut scene.objects {
            if let Some(legacy) = obj.grip_pose_legacy.take() {
                if obj.grip_pose_left.is_none() {
                    obj.grip_pose_left = Some(legacy.clone());
                }
                if obj.grip_pose_right.is_none() {
                    obj.grip_pose_right = Some(legacy);
                }
            }
        }
        dedupe_object_ids(&mut scene.objects, &path.display().to_string());
        Ok(scene)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)
            .with_context(|| format!("failed to write scene {}", path.display()))?;
        Ok(())
    }

    pub fn find_object(&self, id: &str) -> Option<&GameObject> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn find_object_mut(&mut self, id: &str) -> Option<&mut GameObject> {
        self.objects.iter_mut().find(|o| o.id == id)
    }
}

fn dedupe_object_ids(objects: &mut [GameObject], source: &str) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for obj in objects.iter_mut() {
        if seen.insert(obj.id.clone()) {
            continue;
        }
        let stem = obj.id.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_');
        let stem = if stem.is_empty() { obj.id.as_str() } else { stem };
        let mut n = 2;
        let new_id = loop {
            let candidate = format!("{stem}_{n}");
            if !seen.contains(&candidate) {
                break candidate;
            }
            n += 1;
        };
        log::warn!(
            "scene {source}: duplicate object id '{}' — renamed to '{new_id}'",
            obj.id
        );
        seen.insert(new_id.clone());
        obj.id = new_id;
    }
}

#[cfg(test)]
mod grip_pose_migration_test {
    use super::*;

    #[test]
    fn legacy_grip_pose_migrates_to_both_hands() {
        let json = r#"{
            "name": "test",
            "objects": [{
                "id": "obj1",
                "grip_pose": {
                    "hand_offset_pos": [0.1, 0.2, 0.3],
                    "hand_offset_rot": [0.0, 0.0, 0.0, 1.0],
                    "finger_curl": {"index1": 0.5}
                }
            }]
        }"#;
        let tmp = std::env::temp_dir().join("grip_pose_migration_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let obj = &scene.objects[0];
        assert!(
            obj.grip_pose_legacy.is_none(),
            "legacy field should be cleared after migration"
        );
        let left = obj
            .grip_pose_left
            .as_ref()
            .expect("left should be populated");
        let right = obj
            .grip_pose_right
            .as_ref()
            .expect("right should be populated");
        assert_eq!(left.hand_offset_pos, [0.1, 0.2, 0.3]);
        assert_eq!(right.hand_offset_pos, [0.1, 0.2, 0.3]);
        assert_eq!(left.finger_curl.get("index1"), Some(&0.5));

        let out_path = std::env::temp_dir().join("grip_pose_migration_test_out.json");
        scene.save(&out_path).unwrap();
        let saved = std::fs::read_to_string(&out_path).unwrap();
        std::fs::remove_file(&out_path).ok();
        assert!(saved.contains("grip_pose_left"));
        assert!(saved.contains("grip_pose_right"));
        assert!(!saved.contains("\"grip_pose\":"));
    }
}

#[cfg(test)]
mod dedupe_object_ids_test {
    use super::*;

    #[test]
    fn duplicate_ids_get_renamed_to_stay_unique() {
        let json = r#"{
            "name": "test",
            "objects": [
                {"id": "cloud", "cuboid": {"position": [0.0, 0.0, 0.0]}},
                {"id": "cloud", "cuboid": {"position": [1.0, 0.0, 0.0]}},
                {"id": "cloud", "cuboid": {"position": [2.0, 0.0, 0.0]}}
            ]
        }"#;
        let tmp = std::env::temp_dir().join("dedupe_object_ids_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["cloud", "cloud_2", "cloud_3"]);

        for id in &ids {
            assert!(scene.find_object(id).is_some());
        }
    }

    #[test]
    fn duplicate_of_an_already_numbered_id_skips_past_existing_ones() {
        let json = r#"{
            "name": "test",
            "objects": [
                {"id": "cloud_2", "cuboid": {"position": [0.0, 0.0, 0.0]}},
                {"id": "cloud_2", "cuboid": {"position": [1.0, 0.0, 0.0]}}
            ]
        }"#;
        let tmp = std::env::temp_dir().join("dedupe_object_ids_numbered_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["cloud_2", "cloud_3"]);
    }
}

#[cfg(test)]
mod particle_and_laser_scene_test {
    use super::*;

    #[test]
    fn particle_emitters_and_lasers_load_from_lobby_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/scenes/lobby.json");
        let scene = Scene::load(&path).expect("lobby.json should parse");

        let smoke = scene
            .find_object("smoke_grenade")
            .expect("smoke_grenade exists");
        assert!(smoke.particle_emitter.is_some());

        let green = scene.find_object("laser_green").expect("laser_green exists");
        assert!(green.laser.is_some());

        let red = scene.find_object("laser_red").expect("laser_red exists");
        assert!(red.laser.is_some());
    }
}

#[cfg(test)]
mod optic_tests {
    use super::*;

    /// The whole point of the optional-component pattern: existing scenes with no
    /// `optic` field must keep loading, and must not gain one on save.
    #[test]
    fn a_scene_without_an_optic_round_trips_unchanged() {
        let json = r#"{"id":"m4a1"}"#;
        let obj: GameObject = serde_json::from_str(json).expect("legacy object should load");
        assert!(obj.optic.is_none(), "absent optic must stay absent");

        let out = serde_json::to_string(&obj).unwrap();
        let back: GameObject = serde_json::from_str(&out).unwrap();
        assert!(back.optic.is_none());
    }

    #[test]
    fn an_authored_optic_round_trips() {
        let optic = OpticDef {
            class: OpticClass::PrecisionScope,
            magnification: MagnificationDef::Continuous { min: 5.0, max: 25.0 },
            objective_diameter_mm: 56.0,
            true_fov_deg_at_1x: 22.0,
            ..OpticDef::default()
        };
        let obj = GameObject {
            optic: Some(optic),
            ..serde_json::from_str::<GameObject>(r#"{"id":"awp"}"#).unwrap()
        };

        let json = serde_json::to_string(&obj).unwrap();
        let back: GameObject = serde_json::from_str(&json).unwrap();
        let back_optic = back.optic.expect("optic should survive the round trip");
        assert_eq!(back_optic.class, OpticClass::PrecisionScope);
        assert_eq!(back_optic.magnification.max(), 25.0);
        assert_eq!(back_optic.objective_diameter_mm, 56.0);
    }

    /// Exit pupil is the eye box. Deriving it is what makes a new optic correct
    /// by construction instead of needing hand-dialled eye-box radii.
    #[test]
    fn exit_pupil_is_objective_over_magnification() {
        let optic = OpticDef { objective_diameter_mm: 40.0, ..OpticDef::default() };
        assert_eq!(optic.derived_exit_pupil_mm(4.0), 10.0);
        assert_eq!(optic.derived_exit_pupil_mm(16.0), 2.5);
    }

    /// A red dot should be far easier to get behind than a high-power scope, and
    /// that difference must come out of the physics rather than a tuning slider --
    /// it is the mechanism that makes optic choice a real gameplay tradeoff.
    #[test]
    fn a_red_dot_has_a_much_larger_eye_box_than_a_precision_scope() {
        let red_dot = OpticDef {
            class: OpticClass::ReflexRedDot,
            magnification: MagnificationDef::Fixed(1.0),
            objective_diameter_mm: 25.0,
            ..OpticDef::default()
        };
        let sniper = OpticDef {
            class: OpticClass::PrecisionScope,
            magnification: MagnificationDef::Continuous { min: 5.0, max: 25.0 },
            objective_diameter_mm: 56.0,
            ..OpticDef::default()
        };
        assert!(
            red_dot.derived_min_exit_pupil_mm() > sniper.derived_min_exit_pupil_mm() * 5.0,
            "red dot eye box {} should dwarf the sniper's {}",
            red_dot.derived_min_exit_pupil_mm(),
            sniper.derived_min_exit_pupil_mm()
        );
    }

    /// Higher magnification shows less of the world -- this is the value the scope
    /// camera renders with, so getting it wrong shows up as the wrong zoom.
    #[test]
    fn true_field_of_view_narrows_with_magnification() {
        let optic = OpticDef { true_fov_deg_at_1x: 24.0, ..OpticDef::default() };
        assert_eq!(optic.derived_true_fov_deg(1.0), 24.0);
        assert_eq!(optic.derived_true_fov_deg(8.0), 3.0);
        assert!(optic.derived_true_fov_deg(8.0) < optic.derived_true_fov_deg(4.0));
    }

    /// Monocular optics cost one render because a collimated image is
    /// view-independent; binoculars genuinely need one per eye for stereo depth.
    #[test]
    fn render_count_follows_the_optical_path_count() {
        let scope = OpticDef::default();
        assert_eq!(scope.render_count(), 1);

        let binos = OpticDef {
            class: OpticClass::Binocular,
            paths: OpticalPathsDef::Binocular {
                left: OpticalPathDef::default(),
                right: OpticalPathDef::default(),
                ipd_mm: 64.0,
            },
            ..OpticDef::default()
        };
        assert_eq!(binos.render_count(), 2);
    }

    /// Schema-parity guard: the web editor writes optic JSON in JavaScript while
    /// Rust reads it here, and two implementations of one format is exactly how
    /// the avatar rig ended up with the editor and the runtime disagreeing.
    /// This loads the REAL checked-in scene and insists Rust understands what
    /// the editor produced.
    #[test]
    fn the_checked_in_lobby_scene_optic_deserializes_in_rust() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../game/scenes/lobby.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("skipping: {} not present", path.display());
            return;
        };
        let scene: serde_json::Value = serde_json::from_str(&text).expect("lobby.json is valid json");
        let objects = scene["objects"].as_array().expect("objects array");
        let m4 = objects
            .iter()
            .find(|o| o["id"] == "m4a1")
            .expect("lobby has an m4a1");

        let obj: GameObject =
            serde_json::from_value(m4.clone()).expect("m4a1 should deserialize as a GameObject");
        let optic = obj.optic.expect("the m4a1 fixture carries an optic");

        assert_eq!(optic.class, OpticClass::Lpvo);
        assert_eq!(optic.magnification.min(), 1.0);
        assert_eq!(optic.magnification.max(), 6.0);
        assert!(optic.magnification.is_variable());
        // A 24mm objective at 6x leaves a 4mm exit pupil -- demanding but usable,
        // which is the point of putting a magnified optic on the test fixture.
        assert_eq!(optic.derived_exit_pupil_mm(6.0), 4.0);
        assert!((optic.derived_true_fov_deg(6.0) - 26.0 / 6.0).abs() < 1e-4);

        // The optic must be placeable even though m4a1.glb has no lens geometry.
        let OpticalPathsDef::Monocular { path } = &optic.paths else {
            panic!("fixture should be monocular");
        };
        assert!(path.uses_offsets(), "m4a1 has no lens nodes, so offsets must position it");
        assert!(path.body_length_m() > 0.05, "optic needs real length to have an axis");
        assert!(path.ocular_radius_m > 0.0);
    }

    #[test]
    fn magnification_bounds_are_correct_for_each_form() {
        assert_eq!(MagnificationDef::Fixed(1.0).max(), 1.0);
        assert!(!MagnificationDef::Fixed(1.0).is_variable());

        let stepped = MagnificationDef::Stepped { steps: vec![1.0, 4.0, 8.0] };
        assert_eq!(stepped.min(), 1.0);
        assert_eq!(stepped.max(), 8.0);
        assert!(stepped.is_variable());
    }
}
