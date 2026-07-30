//! Runtime state for magnifying optics (weapon sights, scopes, binoculars).
//!
//! This module is deliberately pure: it takes poses and an [`OpticDef`] and
//! produces the small amount of state the renderer needs. It never touches
//! camera matrices, wgpu, or scene mutation, so all of it is unit-testable
//! without a headset or a GPU.
//!
//! The behaviour here is driven by real optics rather than tuning curves:
//!
//! * A scope's eye box is its **exit pupil** (`objective / magnification`).
//!   That is why a 1x25 red dot is forgiving (25 mm) and a 25x56 is not
//!   (2.24 mm) -- and it is the mechanism that makes optic choice a genuine
//!   gameplay tradeoff instead of an assist setting.
//! * Misalignment produces **scope shadow** (a growing crescent), never blur
//!   and never a magnification change. Real glass does not refocus or change
//!   power because your head moved.
//! * A parallax-free optic is collimated, so its image is view-independent.
//!   One render therefore serves both eyes; eye position only affects the
//!   vignette computed here.

use glam::Vec3;

use crate::scene::{MagnificationDef, OpticDef, OpticalPathsDef};

/// Typical human pupil diameter in daylight. Used to decide how far the eye can
/// move before the exit pupil starts clipping it (the onset of scope shadow).
/// Real pupils run ~2 mm bright to ~8 mm dark; 4 mm is the common working value.
pub const EYE_PUPIL_DIAMETER_MM: f32 = 4.0;

/// How deep the eye box is, per millimetre of exit pupil. A big exit pupil
/// gives a forgiving fore/aft window, a small one is fussy in every axis --
/// which matches how high-power scopes actually behave.
///
/// APPROXIMATION: the exact depth depends on the optic's internal focal
/// lengths, which we do not model. Validate on-device and tune this one
/// constant rather than adding per-optic depth sliders.
pub const EYE_BOX_DEPTH_PER_MM_PUPIL: f32 = 4.0;

/// Pose of one optical path, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpticalPathPose {
    /// Front lens: where the scope camera is anchored and where light enters.
    pub objective: Vec3,
    /// Rear lens: what the player looks into.
    pub ocular: Vec3,
    /// Radius of the ocular glass, used for on-screen coverage/gating.
    pub ocular_radius_m: f32,
}

impl OpticalPathPose {
    /// Direction the optic looks, from ocular toward objective.
    pub fn axis(&self) -> Vec3 {
        (self.objective - self.ocular).normalize_or_zero()
    }
}

/// How much of the sight picture a given eye can see, and where it sits in the
/// eye box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeBoxSample {
    /// 1.0 = full clear circle, 0.0 = fully blacked out. Values between are
    /// scope shadow: a crescent eating into the circle.
    pub occupancy: f32,
    /// Perpendicular distance from the optical axis, in millimetres.
    pub lateral_mm: f32,
    /// Distance behind the ocular along the axis, in millimetres. Negative when
    /// the eye is on the wrong side of the glass.
    pub axial_mm: f32,
}

impl EyeBoxSample {
    pub const BLACKED_OUT: Self =
        Self { occupancy: 0.0, lateral_mm: f32::INFINITY, axial_mm: 0.0 };

    /// Whether any usable image reaches this eye.
    pub fn sees_image(&self) -> bool {
        self.occupancy > 0.0
    }
}

/// Where an eye sits relative to one optical path, and how much it can see.
///
/// The eye box is modelled as the exit-pupil disc widened by the eye's own
/// pupil: you get an unshaded image while your pupil is fully inside the exit
/// pupil, shadow while it straddles the edge, and nothing once it is outside.
/// That is a physical construction, not a tuned falloff.
pub fn sample_eye_box(
    eye: Vec3,
    path: &OpticalPathPose,
    exit_pupil_mm: f32,
    eye_relief_mm: f32,
) -> EyeBoxSample {
    let axis = path.axis();
    if axis == Vec3::ZERO {
        return EyeBoxSample::BLACKED_OUT;
    }

    // The eye sits behind the ocular, i.e. opposite the viewing direction.
    let to_eye = eye - path.ocular;
    let back = -axis;
    let axial_m = to_eye.dot(back);
    let axial_mm = axial_m * 1000.0;

    // In front of the glass: you are looking at the objective, not through it.
    if axial_mm <= 0.0 {
        return EyeBoxSample { occupancy: 0.0, lateral_mm: f32::INFINITY, axial_mm };
    }

    let lateral_mm = (to_eye - back * axial_m).length() * 1000.0;

    // Fore/aft: the usable pupil shrinks as you leave the design eye relief.
    let depth_half_mm = (exit_pupil_mm * EYE_BOX_DEPTH_PER_MM_PUPIL).max(1.0);
    let axial_error_mm = (axial_mm - eye_relief_mm).abs();
    let depth_factor = (1.0 - axial_error_mm / depth_half_mm).clamp(0.0, 1.0);
    if depth_factor <= 0.0 {
        return EyeBoxSample { occupancy: 0.0, lateral_mm, axial_mm };
    }

    let usable_pupil_mm = exit_pupil_mm * depth_factor;
    // Unshaded while the eye pupil is fully inside the exit pupil...
    let core_r = ((usable_pupil_mm - EYE_PUPIL_DIAMETER_MM) * 0.5).max(0.0);
    // ...fully dark once it is entirely outside.
    let outer_r = (usable_pupil_mm + EYE_PUPIL_DIAMETER_MM) * 0.5;

    let occupancy = if lateral_mm <= core_r {
        1.0
    } else if lateral_mm >= outer_r {
        0.0
    } else {
        ((outer_r - lateral_mm) / (outer_r - core_r)).clamp(0.0, 1.0)
    };

    EyeBoxSample { occupancy, lateral_mm, axial_mm }
}

/// Fraction of the eye's vertical field the ocular glass spans, used to skip
/// scope work when the optic is a speck on screen (holstered, across the room).
pub fn ocular_screen_fraction(
    eye: Vec3,
    path: &OpticalPathPose,
    eye_fov_y_rad: f32,
) -> f32 {
    let dist = (path.ocular - eye).length();
    if dist <= f32::EPSILON || eye_fov_y_rad <= f32::EPSILON {
        return 0.0;
    }
    let angular_radius = (path.ocular_radius_m / dist).atan();
    (2.0 * angular_radius / eye_fov_y_rad).clamp(0.0, 1.0)
}

/// Current magnification, and the rules for changing it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagnificationState {
    current: f32,
}

impl MagnificationState {
    pub fn new(def: &MagnificationDef) -> Self {
        Self { current: def.min().max(1.0) }
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    /// Set an absolute magnification, clamped to what the optic supports. For a
    /// stepped optic this snaps to the nearest detent, because a variable optic
    /// with detents cannot sit between them.
    pub fn set(&mut self, def: &MagnificationDef, value: f32) {
        self.current = match def {
            MagnificationDef::Fixed(m) => *m,
            MagnificationDef::Stepped { steps } => nearest_step(steps, value),
            MagnificationDef::Continuous { min, max } => value.clamp(*min, *max),
        };
    }

    /// Move by whole detents (stepped) or by a magnification delta (continuous).
    /// `wrap` only applies to stepped optics.
    pub fn step(&mut self, def: &MagnificationDef, delta: i32, wrap: bool) {
        match def {
            MagnificationDef::Fixed(m) => self.current = *m,
            MagnificationDef::Stepped { steps } if !steps.is_empty() => {
                let mut sorted = steps.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let idx = sorted
                    .iter()
                    .position(|s| (*s - self.current).abs() < 1e-4)
                    .unwrap_or(0) as i32;
                let len = sorted.len() as i32;
                let next = if wrap {
                    ((idx + delta) % len + len) % len
                } else {
                    (idx + delta).clamp(0, len - 1)
                };
                self.current = sorted[next as usize];
            }
            MagnificationDef::Stepped { .. } => {}
            MagnificationDef::Continuous { min, max } => {
                self.current = (self.current + delta as f32).clamp(*min, *max);
            }
        }
    }
}

fn nearest_step(steps: &[f32], value: f32) -> f32 {
    steps
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - value)
                .abs()
                .partial_cmp(&(b - value).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1.0)
}

/// What one eye should be shown for one optical path this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathEyeView {
    pub eye_box: EyeBoxSample,
    /// Fraction of the eye's vertical FOV the ocular spans.
    pub screen_fraction: f32,
}

/// Everything the renderer needs about an optic this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct OpticViewState {
    pub magnification: f32,
    /// FOV the scope camera renders with (world/true field of view).
    pub scope_fov_deg: f32,
    /// Scope renders required: 1 for a monocular optic (collimated, so one
    /// render serves both eyes), 2 for binoculars.
    pub render_count: usize,
    /// False when no eye can see usable image, or the optic is too small on
    /// screen to be worth rendering.
    pub render_needed: bool,
    /// Per eye, per path. `[eye][path]`.
    pub eyes: [Vec<PathEyeView>; 2],
}

/// Aim basis captured at the instant the trigger breaks.
///
/// Snapshotting this separately from the render state is what lets a shot be
/// validated against exactly what the player was aiming at when they pulled,
/// rather than against a pose sampled a frame later.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FireSolutionState {
    pub origin: Vec3,
    pub direction: Vec3,
    pub magnification: f32,
    /// Best eye-box occupancy across eyes at the moment of firing -- i.e. how
    /// good the sight picture actually was.
    pub sight_quality: f32,
    pub through_optic: bool,
}

/// Below this fraction of the eye's field, the ocular is too small on screen for
/// scope detail to be perceptible, so the pass is skipped entirely.
pub const MIN_SCREEN_FRACTION_FOR_RENDER: f32 = 0.02;

/// Per-player optic runtime state.
#[derive(Debug, Clone)]
pub struct OpticController {
    magnification: MagnificationState,
    last_fire_solution: Option<FireSolutionState>,
}

impl OpticController {
    pub fn new(def: &OpticDef) -> Self {
        Self {
            magnification: MagnificationState::new(&def.magnification),
            last_fire_solution: None,
        }
    }

    pub fn magnification(&self) -> f32 {
        self.magnification.current()
    }

    pub fn set_magnification(&mut self, def: &OpticDef, value: f32) {
        self.magnification.set(&def.magnification, value);
    }

    pub fn step_magnification(&mut self, def: &OpticDef, delta: i32, wrap: bool) {
        self.magnification.step(&def.magnification, delta, wrap);
    }

    pub fn last_fire_solution(&self) -> Option<FireSolutionState> {
        self.last_fire_solution
    }

    /// Evaluate the optic for this frame.
    ///
    /// `eyes` are the two eye positions in world space; `paths` are the posed
    /// optical paths (one entry for a scope, two for binoculars, matching the
    /// def's [`OpticalPathsDef`]).
    pub fn evaluate(
        &self,
        def: &OpticDef,
        eyes: [Vec3; 2],
        paths: &[OpticalPathPose],
        eye_fov_y_rad: f32,
    ) -> OpticViewState {
        let magnification = self.magnification.current();
        let exit_pupil = def.derived_exit_pupil_mm(magnification);
        let eye_relief = def.eye_relief_mm;

        let sample_eye = |eye: Vec3| -> Vec<PathEyeView> {
            paths
                .iter()
                .map(|p| PathEyeView {
                    eye_box: sample_eye_box(eye, p, exit_pupil, eye_relief),
                    screen_fraction: ocular_screen_fraction(eye, p, eye_fov_y_rad),
                })
                .collect()
        };

        let eyes_views = [sample_eye(eyes[0]), sample_eye(eyes[1])];

        // Render only when some eye both sees usable image and the glass is big
        // enough on screen for the detail to matter.
        let render_needed = eyes_views.iter().any(|views| {
            views.iter().any(|v| {
                v.eye_box.sees_image() && v.screen_fraction >= MIN_SCREEN_FRACTION_FOR_RENDER
            })
        });

        OpticViewState {
            magnification,
            scope_fov_deg: def.derived_true_fov_deg(magnification),
            render_count: match def.paths {
                OpticalPathsDef::Monocular { .. } => 1,
                OpticalPathsDef::Binocular { .. } => 2,
            },
            render_needed,
            eyes: eyes_views,
        }
    }

    /// Capture the aim basis at trigger time. `origin`/`direction` come from the
    /// weapon (muzzle and bore axis), not the head, so the recorded shot matches
    /// the gun rather than where the player happened to be looking.
    pub fn capture_fire_solution(
        &mut self,
        origin: Vec3,
        direction: Vec3,
        view: &OpticViewState,
    ) -> FireSolutionState {
        let sight_quality = view
            .eyes
            .iter()
            .flat_map(|views| views.iter())
            .map(|v| v.eye_box.occupancy)
            .fold(0.0_f32, f32::max);

        let solution = FireSolutionState {
            origin,
            direction: direction.normalize_or_zero(),
            magnification: view.magnification,
            sight_quality,
            through_optic: sight_quality > 0.0,
        };
        self.last_fire_solution = Some(solution);
        solution
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{OpticClass, OpticalPathDef};

    fn path_along_z(ocular_z: f32) -> OpticalPathPose {
        // Looks down -Z; the eye sits at +Z behind the ocular.
        OpticalPathPose {
            objective: Vec3::new(0.0, 0.0, ocular_z - 0.3),
            ocular: Vec3::new(0.0, 0.0, ocular_z),
            ocular_radius_m: 0.02,
        }
    }

    fn red_dot() -> OpticDef {
        OpticDef {
            class: OpticClass::ReflexRedDot,
            magnification: MagnificationDef::Fixed(1.0),
            objective_diameter_mm: 25.0,
            eye_relief_mm: 500.0,
            ..OpticDef::default()
        }
    }

    fn sniper() -> OpticDef {
        OpticDef {
            class: OpticClass::PrecisionScope,
            magnification: MagnificationDef::Continuous { min: 5.0, max: 25.0 },
            objective_diameter_mm: 56.0,
            eye_relief_mm: 90.0,
            ..OpticDef::default()
        }
    }

    /// Eye on the axis at the design eye relief: full, unshaded sight picture.
    #[test]
    fn a_centred_eye_at_design_relief_sees_the_full_picture() {
        let def = sniper();
        let path = path_along_z(0.0);
        let eye = Vec3::new(0.0, 0.0, def.eye_relief_mm / 1000.0);
        let s = sample_eye_box(eye, &path, def.derived_exit_pupil_mm(5.0), def.eye_relief_mm);
        assert_eq!(s.occupancy, 1.0, "centred at correct relief should be unshaded");
        assert!(s.lateral_mm < 1e-3);
        assert!((s.axial_mm - def.eye_relief_mm).abs() < 1e-2);
    }

    /// Move far enough off-axis and the image blacks out entirely.
    #[test]
    fn moving_off_axis_produces_shadow_then_blackout() {
        let def = sniper();
        let exit = def.derived_exit_pupil_mm(25.0); // 2.24mm -- tight
        let path = path_along_z(0.0);
        let at = |lat_mm: f32| {
            let eye = Vec3::new(lat_mm / 1000.0, 0.0, def.eye_relief_mm / 1000.0);
            sample_eye_box(eye, &path, exit, def.eye_relief_mm).occupancy
        };
        let centred = at(0.0);
        let edging = at(2.0);
        let gone = at(20.0);
        assert!(centred > edging, "shadow should grow as the eye moves off axis");
        assert!(edging > gone);
        assert_eq!(gone, 0.0, "far off axis should be fully blacked out");
    }

    /// The headline physical tradeoff: a red dot is dramatically more forgiving
    /// than a high-power scope at the same head offset. This must come from the
    /// exit pupil alone, with no assist term anywhere.
    #[test]
    fn a_red_dot_tolerates_head_movement_a_sniper_scope_does_not() {
        let dot = red_dot();
        let snipe = sniper();
        let lateral_mm = 8.0;

        let dot_path = path_along_z(0.0);
        let dot_eye = Vec3::new(lateral_mm / 1000.0, 0.0, dot.eye_relief_mm / 1000.0);
        let dot_occ =
            sample_eye_box(dot_eye, &dot_path, dot.derived_exit_pupil_mm(1.0), dot.eye_relief_mm)
                .occupancy;

        let snipe_path = path_along_z(0.0);
        let snipe_eye = Vec3::new(lateral_mm / 1000.0, 0.0, snipe.eye_relief_mm / 1000.0);
        let snipe_occ = sample_eye_box(
            snipe_eye,
            &snipe_path,
            snipe.derived_exit_pupil_mm(25.0),
            snipe.eye_relief_mm,
        )
        .occupancy;

        assert_eq!(dot_occ, 1.0, "8mm off axis is nothing to a 25mm exit pupil");
        assert_eq!(snipe_occ, 0.0, "8mm off axis blacks out a 2.24mm exit pupil");
    }

    /// Being too close or too far along the axis also costs you the picture --
    /// the eye box is a volume, not a plane.
    #[test]
    fn wrong_eye_relief_costs_the_sight_picture() {
        let def = sniper();
        let exit = def.derived_exit_pupil_mm(10.0);
        let path = path_along_z(0.0);
        let at = |relief_mm: f32| {
            let eye = Vec3::new(0.0, 0.0, relief_mm / 1000.0);
            sample_eye_box(eye, &path, exit, def.eye_relief_mm).occupancy
        };
        assert_eq!(at(def.eye_relief_mm), 1.0);
        assert!(at(def.eye_relief_mm + 60.0) < 1.0, "too far back should shade");
        assert!(at(def.eye_relief_mm - 60.0) < 1.0, "too close should shade");
    }

    /// Looking at the front of the scope is not looking through it.
    #[test]
    fn an_eye_in_front_of_the_glass_sees_nothing() {
        let def = sniper();
        let path = path_along_z(0.0);
        let eye = Vec3::new(0.0, 0.0, -0.5); // beyond the objective
        let s = sample_eye_box(eye, &path, def.derived_exit_pupil_mm(5.0), def.eye_relief_mm);
        assert_eq!(s.occupancy, 0.0);
        assert!(s.axial_mm <= 0.0);
    }

    /// A holstered or distant optic must cost nothing.
    #[test]
    fn a_distant_optic_is_not_rendered() {
        let def = sniper();
        let ctrl = OpticController::new(&def);
        let far = OpticalPathPose {
            objective: Vec3::new(0.0, 0.0, -30.3),
            ocular: Vec3::new(0.0, 0.0, -30.0),
            ocular_radius_m: 0.02,
        };
        let eyes = [Vec3::new(-0.03, 0.0, 0.0), Vec3::new(0.03, 0.0, 0.0)];
        let view = ctrl.evaluate(&def, eyes, &[far], 1.5);
        assert!(!view.render_needed, "an optic 30m away should be skipped");
    }

    #[test]
    fn an_optic_at_the_eye_is_rendered() {
        let def = sniper();
        let ctrl = OpticController::new(&def);
        let path = path_along_z(0.0);
        let relief = def.eye_relief_mm / 1000.0;
        let eyes = [Vec3::new(0.0, 0.0, relief), Vec3::new(0.065, 0.0, relief)];
        let view = ctrl.evaluate(&def, eyes, &[path], 1.5);
        assert!(view.render_needed);
        assert_eq!(view.render_count, 1, "a monocular optic renders once for both eyes");
        assert_eq!(view.eyes[0][0].eye_box.occupancy, 1.0, "aiming eye sees the picture");
        assert_eq!(view.eyes[1][0].eye_box.occupancy, 0.0, "other eye is 65mm off axis");
    }

    /// Binoculars are two paths, so two renders -- that is what gives stereo depth.
    #[test]
    fn binoculars_render_twice() {
        let def = OpticDef {
            class: OpticClass::Binocular,
            paths: OpticalPathsDef::Binocular {
                left: OpticalPathDef::default(),
                right: OpticalPathDef::default(),
                ipd_mm: 64.0,
            },
            ..OpticDef::default()
        };
        let ctrl = OpticController::new(&def);
        let path = path_along_z(0.0);
        let view = ctrl.evaluate(&def, [Vec3::ZERO, Vec3::ZERO], &[path, path], 1.5);
        assert_eq!(view.render_count, 2);
    }

    #[test]
    fn scope_fov_tracks_magnification() {
        let def = OpticDef {
            magnification: MagnificationDef::Continuous { min: 1.0, max: 8.0 },
            true_fov_deg_at_1x: 24.0,
            ..OpticDef::default()
        };
        let mut ctrl = OpticController::new(&def);
        let path = path_along_z(0.0);
        let v1 = ctrl.evaluate(&def, [Vec3::ZERO, Vec3::ZERO], &[path], 1.5);
        assert_eq!(v1.scope_fov_deg, 24.0);

        ctrl.set_magnification(&def, 8.0);
        let v8 = ctrl.evaluate(&def, [Vec3::ZERO, Vec3::ZERO], &[path], 1.5);
        assert_eq!(v8.scope_fov_deg, 3.0, "8x shows an eighth of the world angle");
    }

    #[test]
    fn stepped_magnification_snaps_clamps_and_wraps() {
        let def = OpticDef {
            magnification: MagnificationDef::Stepped { steps: vec![1.0, 4.0, 8.0] },
            ..OpticDef::default()
        };
        let mut ctrl = OpticController::new(&def);
        assert_eq!(ctrl.magnification(), 1.0);

        ctrl.step_magnification(&def, 1, false);
        assert_eq!(ctrl.magnification(), 4.0);

        // Without wrap, stepping past the top holds at the top.
        ctrl.step_magnification(&def, 5, false);
        assert_eq!(ctrl.magnification(), 8.0);

        // With wrap, one more step comes back around.
        ctrl.step_magnification(&def, 1, true);
        assert_eq!(ctrl.magnification(), 1.0);

        // Absolute sets snap to the nearest detent.
        ctrl.set_magnification(&def, 5.0);
        assert_eq!(ctrl.magnification(), 4.0);
    }

    #[test]
    fn continuous_magnification_clamps_to_the_optics_range() {
        let def = OpticDef {
            magnification: MagnificationDef::Continuous { min: 5.0, max: 25.0 },
            ..OpticDef::default()
        };
        let mut ctrl = OpticController::new(&def);
        assert_eq!(ctrl.magnification(), 5.0);
        ctrl.set_magnification(&def, 100.0);
        assert_eq!(ctrl.magnification(), 25.0);
        ctrl.set_magnification(&def, 0.1);
        assert_eq!(ctrl.magnification(), 5.0);
    }

    /// The shot must record the weapon's basis and how good the sight picture
    /// was, so a hit can later be judged against what the player actually had.
    #[test]
    fn firing_captures_the_aim_basis_and_sight_quality() {
        let def = sniper();
        let mut ctrl = OpticController::new(&def);
        let path = path_along_z(0.0);
        let relief = def.eye_relief_mm / 1000.0;
        let eyes = [Vec3::new(0.0, 0.0, relief), Vec3::new(0.065, 0.0, relief)];
        let view = ctrl.evaluate(&def, eyes, &[path], 1.5);

        let muzzle = Vec3::new(0.0, 0.0, -0.6);
        let solution = ctrl.capture_fire_solution(muzzle, Vec3::new(0.0, 0.0, -2.0), &view);

        assert_eq!(solution.origin, muzzle);
        assert!((solution.direction.length() - 1.0).abs() < 1e-5, "direction is normalised");
        assert_eq!(solution.direction.z.signum(), -1.0);
        assert_eq!(solution.sight_quality, 1.0, "aiming eye had a clear picture");
        assert!(solution.through_optic);
        assert_eq!(ctrl.last_fire_solution(), Some(solution));
    }

    /// Firing with the scope nowhere near your eye is still a legal shot, it
    /// just was not an aimed one -- the renderer and any later hit validation
    /// need to be able to tell the difference.
    #[test]
    fn firing_from_the_hip_records_no_sight_picture() {
        let def = sniper();
        let mut ctrl = OpticController::new(&def);
        let path = path_along_z(0.0);
        let eyes = [Vec3::new(0.4, 0.5, 0.3), Vec3::new(0.46, 0.5, 0.3)];
        let view = ctrl.evaluate(&def, eyes, &[path], 1.5);
        let solution =
            ctrl.capture_fire_solution(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), &view);
        assert_eq!(solution.sight_quality, 0.0);
        assert!(!solution.through_optic);
    }
}
