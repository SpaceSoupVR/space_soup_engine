//! An optic on a held object, frame by frame: where its lenses are, how its
//! magnification control reads the player's hands, and which reticle it draws.
//!
//! Pure like [`crate::optic`]: poses and controls in, numbers out, so the
//! client and the editor share one reading of an [`OpticDef`].

use std::collections::HashMap;

use glam::{Quat, Vec3};

use crate::optic::{OpticController, OpticalPathPose};
use crate::scene::{
    Color3, MagnificationControlDef, MagnificationDef, OpticClass, OpticDef, OpticQualityTier,
    OpticalPathDef, OpticalPathsDef, ReticleDef, ReticleFocalPlane,
};

/// Where a held optic's object and its named model nodes are, in one space.
///
/// Offsets are metres in the object's (or the node's) own frame, scale not
/// applied: the same convention grip points use.
pub struct OpticFrame<'a> {
    pub position: Vec3,
    pub rotation: Quat,
    /// Posed nodes: world position and rotation by name.
    pub nodes: &'a HashMap<String, ([f32; 3], [f32; 4])>,
    /// Each node's rotation in the object's frame at rest, for rings and wheels.
    pub rest_rotations: &'a HashMap<String, Quat>,
}

impl OpticFrame<'_> {
    /// One authored path posed: a named node where it exists, the object's
    /// origin otherwise, each with its offset on top.
    pub fn pose_path(&self, path: &OpticalPathDef) -> OpticalPathPose {
        OpticalPathPose {
            objective: self.point(&path.objective_node, path.objective_offset),
            ocular: self.point(&path.ocular_node, path.ocular_offset),
            ocular_radius_m: path.ocular_radius_m,
        }
    }

    /// Every path of the optic, left before right for binoculars.
    pub fn pose_paths(&self, paths: &OpticalPathsDef) -> Vec<OpticalPathPose> {
        match paths {
            OpticalPathsDef::Monocular { path } => vec![self.pose_path(path)],
            OpticalPathsDef::Binocular { left, right, .. } => {
                vec![self.pose_path(left), self.pose_path(right)]
            }
        }
    }

    /// How far a node has turned about `axis` (in the node's rest frame) since
    /// rest, in degrees. `None` when the model has no such node.
    pub fn node_twist_deg(&self, node: &str, axis: [f32; 3]) -> Option<f32> {
        let (_, now) = self.nodes.get(node)?;
        let rest = self.rest_rotations.get(node).copied().unwrap_or(Quat::IDENTITY);
        let in_object = self.rotation.inverse() * Quat::from_array(*now);
        let turn = rest.inverse() * in_object;
        let axis = Vec3::from(axis).normalize_or_zero();
        if axis == Vec3::ZERO {
            return None;
        }
        let along = Vec3::new(turn.x, turn.y, turn.z).dot(axis);
        Some((2.0 * along.atan2(turn.w)).to_degrees())
    }

    fn point(&self, node: &str, offset: [f32; 3]) -> Vec3 {
        let offset = Vec3::from(offset);
        match self.nodes.get(node).filter(|_| !node.is_empty()) {
            Some((pos, rot)) => Vec3::from(*pos) + Quat::from_array(*rot) * offset,
            None => self.position + self.rotation * offset,
        }
    }
}

/// The player's controls this frame, by the names
/// [`MagnificationControlDef::BUTTONS`] and [`MagnificationControlDef::AXES`] list.
pub struct OpticControls<'a> {
    /// Buttons that went down this frame.
    pub pressed: &'a [&'a str],
    /// Stick axes, each -1..1.
    pub axes: &'a [(&'a str, f32)],
    pub dt_s: f32,
}

impl MagnificationControlDef {
    /// Button names a `ButtonStep` control can take.
    pub const BUTTONS: [&'static str; 10] = [
        "a", "b", "x", "y", "left_stick_click", "right_stick_click",
        "left_stick_up", "left_stick_down", "right_stick_up", "right_stick_down",
    ];
    /// Axis names an `Axis` control can take.
    pub const AXES: [&'static str; 4] = ["left_stick_x", "left_stick_y", "right_stick_x", "right_stick_y"];

    /// The magnification a ring or wheel sets for its node's twist, or `None`
    /// for controls that do not follow a part.
    pub fn power_for_twist(&self, magnification: &MagnificationDef, twist_deg: f32) -> Option<f32> {
        let (range_deg, detents) = match self {
            Self::PhysicalRing { angle_range_deg, detents, .. } => (*angle_range_deg, *detents),
            Self::PhysicalWheel { turns, .. } => (360.0 * turns, false),
            _ => return None,
        };
        let share = (twist_deg / range_deg.abs().max(1e-3)).clamp(0.0, 1.0);
        let (low, high) = (magnification.min(), magnification.max());
        let power = low + (high - low) * share;
        Some(if detents && matches!(magnification, MagnificationDef::Continuous { .. }) { power.round() } else { power })
    }
}

impl OpticController {
    /// Change power from the optic's own control: a ring or wheel follows its
    /// part directly, a button steps with easing, a stick sweeps.
    pub fn apply_controls(&mut self, def: &OpticDef, controls: &OpticControls, frame: &OpticFrame) {
        match &def.magnification_control {
            MagnificationControlDef::ButtonStep { button, wrap } => {
                if controls.pressed.contains(&button.as_str()) {
                    self.step_magnification_smooth(def, 1, *wrap);
                }
            }
            MagnificationControlDef::Axis { source, sensitivity } => {
                let amount = controls.axes.iter().find(|(name, _)| name == source).map_or(0.0, |(_, v)| *v);
                if amount.abs() > 0.15 {
                    let power = self.magnification() + amount * sensitivity * controls.dt_s;
                    self.set_magnification(def, power);
                }
            }
            MagnificationControlDef::PhysicalRing { ring_node: node, rotation_axis: axis, .. }
            | MagnificationControlDef::PhysicalWheel { wheel_node: node, rotation_axis: axis, .. } => {
                let power = frame
                    .node_twist_deg(node, *axis)
                    .and_then(|twist| def.magnification_control.power_for_twist(&def.magnification, twist));
                if let Some(power) = power {
                    self.set_magnification(def, power);
                }
            }
            MagnificationControlDef::None | MagnificationControlDef::ScriptOnly => {}
        }
        self.advance(controls.dt_s);
    }
}

/// The pattern drawn in an optic's focal plane. Stored by [`Self::name`] in
/// [`ReticleDef::style`]; an empty style takes the class's usual one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReticleStyle {
    Clear,
    Dot,
    CircleDot,
    Crosshair,
    Duplex,
    DuplexDot,
    MilDot,
    Chevron,
    Bdc,
}

impl ReticleStyle {
    pub const ALL: [Self; 9] = [
        Self::Clear, Self::Dot, Self::CircleDot, Self::Crosshair, Self::Duplex,
        Self::DuplexDot, Self::MilDot, Self::Chevron, Self::Bdc,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Clear => "none",
            Self::Dot => "dot",
            Self::CircleDot => "circle_dot",
            Self::Crosshair => "crosshair",
            Self::Duplex => "duplex",
            Self::DuplexDot => "duplex_dot",
            Self::MilDot => "mil_dot",
            Self::Chevron => "chevron",
            Self::Bdc => "bdc",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "None",
            Self::Dot => "Red dot (2 MOA)",
            Self::CircleDot => "Circle and dot (65 MOA)",
            Self::Crosshair => "Fine crosshair",
            Self::Duplex => "Duplex",
            Self::DuplexDot => "Duplex, lit centre dot",
            Self::MilDot => "Mil-dot",
            Self::Chevron => "Chevron with drop ticks",
            Self::Bdc => "Crosshair with drop marks",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The reticle an optic of `class` usually carries.
    pub fn for_class(class: OpticClass) -> Self {
        match class {
            OpticClass::ReflexRedDot => Self::Dot,
            OpticClass::Holographic => Self::CircleDot,
            OpticClass::Lpvo => Self::DuplexDot,
            OpticClass::FixedPrism => Self::Chevron,
            OpticClass::PrecisionScope => Self::MilDot,
            OpticClass::Binocular => Self::Clear,
        }
    }

    /// The index the lens shader switches on.
    pub fn shader_index(self) -> u32 {
        Self::ALL.iter().position(|s| *s == self).map_or(0, |i| i as u32)
    }
}

/// What the renderer needs to draw an optic's reticle this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReticleLook {
    pub style: ReticleStyle,
    /// Linear colour times brightness; zero brightness draws an etched black reticle.
    pub emission: [f32; 3],
    pub is_lit: bool,
    /// Multiplies world angle (mrad) into reticle units: 1 in the first focal
    /// plane, magnification over the top power in the second.
    pub scale: f32,
}

impl ReticleLook {
    /// The authored reticle at `magnification`; no reticle authored takes the
    /// class's usual one, lit in its default colour.
    pub fn of(def: &OpticDef, magnification: f32) -> Self {
        let authored = def.reticle.clone().unwrap_or_default();
        let style = ReticleStyle::from_name(&authored.style).unwrap_or_else(|| ReticleStyle::for_class(def.class));
        let top = def.magnification.max().max(1.0);
        let scale = match authored.focal_plane {
            ReticleFocalPlane::First => 1.0,
            ReticleFocalPlane::Second => magnification.max(1e-3) / top,
        };
        let Color3(r, g, b, _) = authored.color;
        let linear = |c: u8| (f32::from(c) / 255.0).powf(2.2) * authored.brightness.max(0.0);
        Self { style, emission: [linear(r), linear(g), linear(b)], is_lit: authored.brightness > 0.0, scale }
    }
}

impl OpticClass {
    /// Reflex and holographic sights have no eye relief to keep: the window
    /// works from any distance behind it.
    pub fn has_unlimited_eye_relief(self) -> bool {
        matches!(self, Self::ReflexRedDot | Self::Holographic)
    }
}

impl OpticDef {
    /// A real optic of `class`, from its spec sheet, to start authoring from.
    pub fn preset(class: OpticClass) -> Self {
        let (magnification, objective_diameter_mm, true_fov_deg_at_1x, eye_relief_mm, radius) = match class {
            OpticClass::ReflexRedDot => (MagnificationDef::Fixed(1.0), 25.0, 30.0, 300.0, 0.012),
            OpticClass::Holographic => (MagnificationDef::Fixed(1.0), 30.0, 30.0, 300.0, 0.015),
            OpticClass::Lpvo => (MagnificationDef::Continuous { min: 1.0, max: 6.0 }, 24.0, 26.0, 95.0, 0.017),
            OpticClass::FixedPrism => (MagnificationDef::Fixed(4.0), 32.0, 28.0, 38.0, 0.015),
            OpticClass::PrecisionScope => (MagnificationDef::Continuous { min: 5.0, max: 25.0 }, 56.0, 23.5, 90.0, 0.02),
            OpticClass::Binocular => (MagnificationDef::Fixed(8.0), 42.0, 60.0, 18.0, 0.012),
        };
        let path = OpticalPathDef { ocular_radius_m: radius, ..OpticalPathDef::default() };
        let paths = match class {
            OpticClass::Binocular => OpticalPathsDef::Binocular { left: path.clone(), right: path, ipd_mm: 64.0 },
            _ => OpticalPathsDef::Monocular { path },
        };
        let focal_plane = if class == OpticClass::PrecisionScope { ReticleFocalPlane::First } else { ReticleFocalPlane::Second };
        let reticle = ReticleDef { style: ReticleStyle::for_class(class).name().to_string(), focal_plane, ..ReticleDef::default() };
        Self {
            class,
            magnification_control: Self::preset_control(&magnification),
            magnification,
            objective_diameter_mm,
            true_fov_deg_at_1x,
            eye_relief_mm,
            paths,
            reticle: Some(reticle),
            zero: None,
            quality: if class.has_unlimited_eye_relief() { OpticQualityTier::Low } else { OpticQualityTier::Balanced },
        }
    }

    fn preset_control(magnification: &MagnificationDef) -> MagnificationControlDef {
        match magnification {
            MagnificationDef::Fixed(_) => MagnificationControlDef::None,
            _ => MagnificationControlDef::Axis { source: "right_stick_y".to_string(), sensitivity: 4.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame<'a>(
        nodes: &'a HashMap<String, ([f32; 3], [f32; 4])>,
        rest: &'a HashMap<String, Quat>,
    ) -> OpticFrame<'a> {
        OpticFrame { position: Vec3::new(1.0, 2.0, 3.0), rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), nodes, rest_rotations: rest }
    }

    #[test]
    fn offsets_turn_with_the_object() {
        let (nodes, rest) = (HashMap::new(), HashMap::new());
        let path = OpticalPathDef { objective_offset: [0.1, 0.0, 0.0], ocular_offset: [-0.1, 0.0, 0.0], ..OpticalPathDef::default() };
        let pose = frame(&nodes, &rest).pose_path(&path);
        // A quarter turn about +Y carries +X onto -Z.
        assert!((pose.objective - Vec3::new(1.0, 2.0, 2.9)).length() < 1e-5, "{:?}", pose.objective);
        assert!((pose.axis() - Vec3::NEG_Z).length() < 1e-5);
    }

    #[test]
    fn a_named_node_wins_over_the_object_origin() {
        let mut nodes = HashMap::new();
        nodes.insert("lens".to_string(), ([5.0, 0.0, 0.0], Quat::IDENTITY.to_array()));
        let rest = HashMap::new();
        let path = OpticalPathDef { objective_node: "lens".into(), objective_offset: [0.0, 0.01, 0.0], ..OpticalPathDef::default() };
        let pose = frame(&nodes, &rest).pose_path(&path);
        assert!((pose.objective - Vec3::new(5.0, 0.01, 0.0)).length() < 1e-6);
        // The missing ocular node falls back to the object.
        assert!((pose.ocular - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-6);
    }

    #[test]
    fn binoculars_pose_two_paths() {
        let (nodes, rest) = (HashMap::new(), HashMap::new());
        let def = OpticDef::preset(OpticClass::Binocular);
        assert_eq!(frame(&nodes, &rest).pose_paths(&def.paths).len(), 2);
    }

    #[test]
    fn a_ring_turned_halfway_sets_the_middle_power() {
        let ring = MagnificationControlDef::PhysicalRing {
            ring_node: "ring".into(),
            rotation_axis: [0.0, 0.0, 1.0],
            angle_range_deg: 120.0,
            detents: false,
        };
        let range = MagnificationDef::Continuous { min: 1.0, max: 6.0 };
        assert_eq!(ring.power_for_twist(&range, 60.0), Some(3.5));
        assert_eq!(ring.power_for_twist(&range, -30.0), Some(1.0));
        assert_eq!(ring.power_for_twist(&range, 500.0), Some(6.0));
    }

    #[test]
    fn a_ring_node_twist_is_read_in_the_object_frame() {
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let twisted = rotation * Quat::from_rotation_z(60f32.to_radians());
        let mut nodes = HashMap::new();
        nodes.insert("ring".to_string(), ([0.0; 3], twisted.to_array()));
        let rest = HashMap::new();
        let twist = frame(&nodes, &rest).node_twist_deg("ring", [0.0, 0.0, 1.0]).unwrap();
        assert!((twist - 60.0).abs() < 1e-3, "{twist}");
    }

    #[test]
    fn a_button_steps_and_a_stick_sweeps() {
        let (nodes, rest) = (HashMap::new(), HashMap::new());
        let mut def = OpticDef::preset(OpticClass::Lpvo);
        def.magnification = MagnificationDef::Stepped { steps: vec![1.0, 3.0, 6.0] };
        def.magnification_control = MagnificationControlDef::ButtonStep { button: "b".into(), wrap: true };
        let mut optic = OpticController::new(&def);
        let pressed = OpticControls { pressed: &["b"], axes: &[], dt_s: 1.0 };
        optic.apply_controls(&def, &pressed, &frame(&nodes, &rest));
        assert!((optic.magnification() - 3.0).abs() < 1e-4);

        let def = OpticDef::preset(OpticClass::Lpvo);
        let mut optic = OpticController::new(&def);
        let stick = OpticControls { pressed: &[], axes: &[("right_stick_y", 1.0)], dt_s: 0.5 };
        optic.apply_controls(&def, &stick, &frame(&nodes, &rest));
        assert!((optic.magnification() - 3.0).abs() < 1e-4, "{}", optic.magnification());
    }

    #[test]
    fn styles_round_trip_by_name_and_empty_takes_the_class() {
        for style in ReticleStyle::ALL {
            assert_eq!(ReticleStyle::from_name(style.name()), Some(style));
        }
        let mut def = OpticDef::preset(OpticClass::FixedPrism);
        def.reticle = None;
        assert_eq!(ReticleLook::of(&def, 4.0).style, ReticleStyle::Chevron);
    }

    #[test]
    fn a_second_plane_reticle_keeps_its_size_and_a_first_plane_one_grows() {
        let mut def = OpticDef::preset(OpticClass::PrecisionScope);
        assert_eq!(ReticleLook::of(&def, 5.0).scale, 1.0);
        if let Some(r) = def.reticle.as_mut() {
            r.focal_plane = ReticleFocalPlane::Second;
        }
        assert!((ReticleLook::of(&def, 5.0).scale - 0.2).abs() < 1e-6);
    }

    #[test]
    fn every_preset_derives_sane_optics() {
        for class in [
            OpticClass::ReflexRedDot, OpticClass::Holographic, OpticClass::Lpvo,
            OpticClass::FixedPrism, OpticClass::PrecisionScope, OpticClass::Binocular,
        ] {
            let def = OpticDef::preset(class);
            let pupil = def.derived_min_exit_pupil_mm();
            assert!((2.0..=30.0).contains(&pupil), "{class:?} exit pupil {pupil}");
            assert!(def.derived_true_fov_deg(def.magnification.max()) < 31.0);
        }
    }
}
