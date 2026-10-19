use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HingeSide {

    #[default]
    Left,

    Right,
}

fn default_open_max() -> f32 {
    100.0
}
fn default_door_mass() -> f32 {
    25.0
}
fn default_door_friction() -> f32 {
    3.0
}
fn default_surface_friction() -> f32 {
    0.5
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoorDef {
    #[serde(default)]
    pub hinge: HingeSide,

    #[serde(default)]
    pub open_min_deg: f32,
    #[serde(default = "default_open_max")]
    pub open_max_deg: f32,

    #[serde(default = "default_door_mass")]
    pub mass: f32,

    #[serde(default = "default_door_friction")]
    pub friction: f32,

    #[serde(default)]
    pub closer: f32,

    #[serde(default = "default_surface_friction")]
    pub surface_friction: f32,

    #[serde(default)]
    pub start_deg: f32,
    /// A LATCH: a bolt that holds the leaf shut until its handle is worked --
    /// a hand on the lever, or holding it. Needs `handle`. See [`DoorLatch`].
    #[serde(default, skip_serializing_if = "is_false")]
    pub latch: bool,
    /// THE HANDLE: where it is on the leaf, in the leaf's own frame, metres
    /// from its centre -- x along the leaf, y up; z is not read, the handle
    /// stands out of both faces. A hand that grips there holds the door and
    /// swings it (`PhysicsWorld::grab_door`). None: a door that is only pushed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<[f32; 3]>,
    /// The id of the object drawn as the handle's LEVER, if it has one: a
    /// model of its own, placed at the handle of the closed leaf. It rides
    /// the leaf, and turns down on its spindle (its own z axis; modelled
    /// pointing along its own -x) while the handle is worked. Drawn by the
    /// headset (`quest_app::client_doors`); it has no part in the physics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lever: Option<String>,
    /// WHERE THE HINGE'S PIN IS, from the middle of the hinged edge, metres:
    /// `pin_inset` into the leaf along it, `pin_offset` through it (+z or -z
    /// in the leaf's frame). Both zero -- the leaf turns about the middle of
    /// its own edge -- needs a gap at the jamb of half the leaf's thickness,
    /// or its corners turn into the frame. A real door has one of two hinges:
    /// - BUTT HINGES, the pin at the face the door opens toward
    ///   (`pin_offset` = half the thickness, signed to that face): the leaf
    ///   swings clear of its jamb with a 3 mm gap, to that side only;
    /// - A CENTRE PIVOT, the pin half a thickness in from the edge
    ///   (`pin_inset`), the edge rounded about it: a double-acting leaf that
    ///   turns in place.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pin_inset: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pin_offset: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for DoorDef {
    fn default() -> Self {
        Self {
            hinge: HingeSide::Left,
            open_min_deg: 0.0,
            open_max_deg: default_open_max(),
            mass: default_door_mass(),
            friction: default_door_friction(),
            closer: 0.0,
            surface_friction: default_surface_friction(),
            start_deg: 0.0,
            latch: false,
            handle: None,
            lever: None,
            pin_inset: 0.0,
            pin_offset: 0.0,
        }
    }
}

impl DoorDef {

    pub fn range(&self) -> (f32, f32) {
        let (a, b) = (self.open_min_deg, self.open_max_deg);
        let (lo, hi) = (a.min(b).to_radians(), a.max(b).to_radians());
        (lo.min(0.0), hi.max(0.0))
    }
}

/// How far a hand may be from a door's handle and still work it, metres:
/// the lever is some 12 cm long and a palm is not a point.
pub const HANDLE_REACH: f32 = 0.14;
/// How far the handle stands out of the leaf's face, metres: where a hand
/// holds it.
pub const HANDLE_STANDOFF: f32 = 0.055;
/// Within this of closed, and turning no faster than [`LATCH_SPIN`], a
/// latching door's bolt goes home.
pub const LATCH_DEG: f32 = 1.5;
pub const LATCH_SPIN: f32 = 4.0;
/// What the bolt holds the leaf within, degrees either side of closed: the
/// play of a real latch in its keep.
pub const LATCH_PLAY_DEG: f32 = 0.15;
/// HOW HARD A HAND PULLS A DOOR BY ITS HANDLE: a spring on how far the handle
/// is from the hand, N/m, damped (N s/m), and never more than
/// [`DOOR_PULL_MAX`] newtons -- an arm's steady pull. The cap is what keeps a
/// hand that goes on past the leaf's stop from fighting it: at the stop the
/// pull is one steady force the hinge's limit simply holds.
pub const DOOR_PULL_STIFFNESS: f32 = 2500.0;
pub const DOOR_PULL_DAMPING: f32 = 140.0;
pub const DOOR_PULL_MAX: f32 = 120.0;

/// A SOFT STOP: the fastest a leaf may turn INTO a stop that is `remaining`
/// radians ahead of it, rad/s -- as if the last degrees were against a
/// rubber buffer, or a closer's latch-speed valve. The speed allowed falls
/// with the angle left ([`STOP_RATE`] a second of it, so half of what is left
/// a tick at sixty a second: it cannot overrun), down to a creep that still
/// brings the leaf home ([`STOP_CREEP`]). A leaf flung at its stop arrives
/// in a few hundredths of a second without a bang, a bounce or a moment
/// past its limit, which a hard limit alone gave it (3 degrees past at
/// 20 rad/s).
pub const STOP_RATE: f32 = 30.0;
pub const STOP_CREEP: f32 = 0.25;

/// `spin` (rad/s, positive toward `hi`) of a leaf at `angle` between its
/// stops, held within the soft stops' envelope. See [`STOP_RATE`].
pub fn soft_stop(angle: f32, spin: f32, lo: f32, hi: f32) -> f32 {
    if spin > 0.0 {
        spin.min((hi - angle).max(0.0) * STOP_RATE + STOP_CREEP)
    } else {
        spin.max(-((angle - lo).max(0.0) * STOP_RATE + STOP_CREEP))
    }
}

/// A DOOR'S HANDLE AND LATCH, as the physics, the server and the headset all
/// work them. The rule is one line, so that a door can never be shut for
/// good: THE BOLT IS DRAWN WHILE A HAND IS AT THE HANDLE -- within
/// [`HANDLE_REACH`] of it on either face, or holding it -- and goes home when
/// the leaf is shut ([`LATCH_DEG`]) with no hand there. No key, no direction
/// to turn it, nothing to hold down: reach for the handle and the door is a
/// door.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorLatch {
    /// The handle's place in the leaf's frame (z = 0: the leaf's middle).
    pub handle: Vec3,
    /// Whether there is a bolt at all (`DoorDef::latch`); a handle without
    /// one is only for holding.
    pub bolts: bool,
}

impl DoorLatch {
    pub fn of(def: &DoorDef) -> Option<Self> {
        let h = def.handle?;
        Some(Self { handle: Vec3::new(h[0], h[1], 0.0), bolts: def.latch })
    }

    /// How far a hand at `hand` (world) is from the handle of a leaf whose
    /// centre and rotation are `pose`, of half thickness `half_z`: zero
    /// anywhere along the handle's spindle through the leaf and out to where
    /// a hand holds it on either face.
    pub fn distance(&self, pose: (Vec3, Quat), half_z: f32, hand: Vec3) -> f32 {
        let local = pose.1.inverse() * (hand - pose.0);
        let beyond = (local.z.abs() - (half_z + HANDLE_STANDOFF)).max(0.0);
        Vec3::new(local.x - self.handle.x, local.y - self.handle.y, beyond).length()
    }

    /// Whether any of `hands` (world) is at the handle. See [`DoorLatch`].
    pub fn worked(&self, pose: (Vec3, Quat), half_z: f32, hands: impl IntoIterator<Item = Vec3>) -> bool {
        hands.into_iter().any(|h| self.distance(pose, half_z, h) <= HANDLE_REACH)
    }

    /// Where a hand holds the handle, in the leaf's frame: on the face the
    /// hand at `hand` (world) is on.
    pub fn hold_point(&self, pose: (Vec3, Quat), half_z: f32, hand: Vec3) -> Vec3 {
        let local = pose.1.inverse() * (hand - pose.0);
        let side = if local.z >= 0.0 { 1.0 } else { -1.0 };
        Vec3::new(self.handle.x, self.handle.y, side * (half_z + HANDLE_STANDOFF))
    }

    /// THE BOLT, a step on: `bolted` as it was; the leaf `angle` from closed
    /// and turning at `spin`; `worked` whether a hand is at the handle.
    pub fn bolt(&self, bolted: bool, angle: f32, spin: f32, worked: bool) -> bool {
        if !self.bolts || worked {
            return false;
        }
        bolted || (angle.abs() <= LATCH_DEG.to_radians() && spin.abs() <= LATCH_SPIN)
    }
}

/// The force a hand holding a door's handle pulls it with, newtons, toward
/// the hand: `offset` from the handle's hold point to the hand, `closing` the
/// handle's own velocity (both world). See [`DOOR_PULL_STIFFNESS`].
pub fn door_pull_force(offset: Vec3, closing: Vec3) -> Vec3 {
    (offset * DOOR_PULL_STIFFNESS - closing * DOOR_PULL_DAMPING).clamp_length_max(DOOR_PULL_MAX)
}

/// Below this many degrees from closed a door seals its doorway: portal
/// culling treats the doorway as a wall. Two degrees on an 80 cm leaf is a
/// 2.8 cm gap at the latch edge -- a sliver no culled thing shows through
/// that the frame's jamb does not already hide.
pub const SHUT_DEG: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorGeom {

    pub closed_pos: Vec3,
    pub closed_rot: Quat,
    pub half: Vec3,
    /// The hinge's pin in the leaf's own frame: the middle of the hinged
    /// edge, moved by the door's `pin_inset` and `pin_offset`.
    pub hinge_local: Vec3,

    pub lo: f32,
    pub hi: f32,
}

impl DoorGeom {
    pub fn new(closed_pos: Vec3, closed_rot: Quat, half: Vec3, def: &DoorDef) -> Self {
        let x = match def.hinge {
            HingeSide::Left => -half.x + def.pin_inset,
            HingeSide::Right => half.x - def.pin_inset,
        };
        let (lo, hi) = def.range();
        Self { closed_pos, closed_rot, half, hinge_local: Vec3::new(x, 0.0, def.pin_offset), lo, hi }
    }

    pub fn of(obj: &crate::scene::GameObject) -> Option<Self> {
        let def = obj.door.as_ref()?;
        Some(Self::new(obj.cuboid.position, obj.cuboid.rotation, obj.cuboid.half_size, def))
    }

    pub fn hinge(&self) -> Vec3 {
        self.closed_pos + self.closed_rot * self.hinge_local
    }

    pub fn axis(&self) -> Vec3 {
        self.closed_rot * Vec3::Y
    }

    pub fn pose_at(&self, angle: f32) -> (Vec3, Quat) {
        let rot = self.closed_rot * Quat::from_rotation_y(angle);
        (self.hinge() - rot * self.hinge_local, rot)
    }

    pub fn angle_of(&self, rot: Quat) -> f32 {
        let q = self.closed_rot.inverse() * rot;
        let q = if q.w < 0.0 { -q } else { q };
        2.0 * q.y.atan2(q.w)
    }

    pub fn corners_at(&self, angle: f32) -> [Vec3; 8] {
        let (c, r) = self.pose_at(angle);
        let h = self.half;
        std::array::from_fn(|i| {
            let s = Vec3::new(
                if i & 1 == 0 { -h.x } else { h.x },
                if i & 2 == 0 { -h.y } else { h.y },
                if i & 4 == 0 { -h.z } else { h.z },
            );
            c + r * s
        })
    }

    /// Whether this leaf is bolted where it hangs: a range of nothing (the
    /// inactive leaf of a pair, held by its flush bolts). It is a wall that
    /// looks like a door, and the physics makes it one (`spawn_door`).
    pub fn fixed(&self) -> bool {
        self.hi - self.lo < 1e-4
    }

    /// Whether a leaf at `angle` seals its doorway (`SHUT_DEG`).
    pub fn shut(angle: f32) -> bool {
        angle.abs() < SHUT_DEG.to_radians()
    }

    pub fn open_fraction(angle: f32) -> f32 {
        (angle.abs() / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0)
    }

    pub fn hinge_inertia(&self, mass: f32) -> f32 {
        let (w, t) = (2.0 * self.half.x, 2.0 * self.half.z);
        mass * (w * w + t * t) / 3.0
    }

    fn along(&self, angle: f32) -> Vec3 {
        Quat::from_rotation_y(angle) * (-self.hinge_local).normalize_or_zero()
    }

    pub fn clear_spheres(&self, angle: f32, spheres: &[(Vec3, f32)]) -> f32 {
        let along = Vec3::new(-self.hinge_local.x, 0.0, 0.0);
        let width = self.half.x + along.x.abs();
        let base = along.normalize_or_zero();
        let mut a = angle;

        for _ in 0..2 {
            for &(c, s) in spheres {
                let foot = Quat::from_rotation_y(a) * Vec3::new(0.0, 0.0, -self.hinge_local.z);
                let p = self.closed_rot.inverse() * (c - self.hinge()) - foot;
                if p.y.abs() > self.half.y + s {
                    continue;
                }
                let flat = Vec3::new(p.x, 0.0, p.z);
                let r = flat.length();
                let reach = s + self.half.z;
                if r <= reach || r > width + s {
                    continue;
                }

                let phi = (base.z * flat.x - base.x * flat.z).atan2(base.dot(flat));
                let delta = (reach / r).min(1.0).asin();
                let gap = wrap(phi - a);
                if gap.abs() >= delta {
                    continue;
                }
                a = if gap >= 0.0 { phi - delta } else { phi + delta };
                a = wrap(a).clamp(self.lo, self.hi);
            }
        }
        a
    }

    pub fn body_contact(&self, angle: f32, body: Vec3, y_range: (f32, f32), radius: f32) -> Option<(Vec3, Vec3, f32)> {
        let (centre, rot) = self.pose_at(angle);
        let (lo, hi) = (centre.y - self.half.y, centre.y + self.half.y);
        if y_range.1 < lo || y_range.0 > hi {
            return None;
        }

        let q = rot.inverse() * Vec3::new(body.x - centre.x, 0.0, body.z - centre.z);
        let (hx, hz) = (self.half.x, self.half.z);
        let nearest = Vec3::new(q.x.clamp(-hx, hx), 0.0, q.z.clamp(-hz, hz));
        let off = Vec3::new(q.x - nearest.x, 0.0, q.z - nearest.z);
        let d = off.length();
        let (normal, depth) = if d > 1e-5 {
            (off / d, radius - d)
        } else {

            let side = if q.z >= 0.0 { 1.0 } else { -1.0 };
            (Vec3::new(0.0, 0.0, side), radius + hz - q.z.abs())
        };
        if depth <= 0.0 {
            return None;
        }
        let y = ((y_range.0 + y_range.1) * 0.5).clamp(lo, hi);
        let point = Vec3::new(centre.x, y, centre.z) + rot * nearest;
        Some((point, rot * normal, depth))
    }
}

fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let mut a = a % t;
    if a > std::f32::consts::PI {
        a -= t;
    } else if a <= -std::f32::consts::PI {
        a += t;
    }
    a
}

pub const BODY_PUSH_STIFFNESS: f32 = 1500.0;

pub const BODY_PUSH_DAMPING: f32 = 120.0;

pub const BODY_PUSH_MAX: f32 = 300.0;

pub const BODY_PUSH_RADIUS: f32 = 0.32;

pub const BODY_PUSH_SPAN: (f32, f32) = (1.3, 0.25);

pub fn body_push_force(depth: f32, closing: f32) -> f32 {
    (BODY_PUSH_STIFFNESS * depth + BODY_PUSH_DAMPING * closing).clamp(0.0, BODY_PUSH_MAX)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HingeState {
    pub angle: f32,
    pub spin: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HingeLoad {
    pub inertia: f32,
    pub closer: f32,
    pub friction: f32,
}

impl HingeLoad {
    pub fn of(geom: &DoorGeom, def: &DoorDef) -> Self {
        Self { inertia: geom.hinge_inertia(def.mass.max(0.5)).max(1e-3), closer: def.closer.max(0.0), friction: def.friction.max(0.0) }
    }
}

const HINGE_STEP: f32 = 1.0 / 120.0;

const HINGE_MAX_SPIN: f32 = 20.0;

impl DoorGeom {
    /// THE LEAF `dt` LATER: pushed by the bodies under `heads` (world; as
    /// `rigid_physics_door::push_doors` pushes it) and by `hands` -- world
    /// spheres, each `(where it was dt ago, where it is now, radius)`, moved
    /// along that path through the step, so the leaf is turned just clear of
    /// each (`clear_spheres`) and takes the hand's own speed -- slowed by the
    /// hinge's friction, drawn home by its closer, and held by its stops.
    pub fn step_hinge(&self, s: HingeState, load: &HingeLoad, heads: &[Vec3], hands: &[(Vec3, Vec3, f32)], dt: f32) -> HingeState {
        self.step_hinge_held(s, load, heads, hands, &[], dt)
    }

    /// [`Self::step_hinge`] with hands HOLDING THE HANDLE: each of `pulls` a
    /// hand's grip `(where it was dt ago, where it is now)` and the point of
    /// the leaf it holds, in the leaf's frame (`DoorLatch::hold_point`) --
    /// drawn to the hand by the spring the server's physics uses
    /// (`door_pull_force`, `PhysicsWorld::grab_door`), so the leaf the
    /// headset draws under a hand swings as the server's will.
    pub fn step_hinge_held(
        &self,
        mut s: HingeState,
        load: &HingeLoad,
        heads: &[Vec3],
        hands: &[(Vec3, Vec3, f32)],
        pulls: &[(Vec3, Vec3, Vec3)],
        dt: f32,
    ) -> HingeState {
        use crate::rigid_physics_door::{CLOSER_PRELOAD, HINGE_DRY_FRICTION};
        if dt <= 0.0 {
            return s;
        }
        let steps = (dt / HINGE_STEP).ceil().clamp(1.0, 16.0);
        let h = dt / steps;
        let (axis, hinge, i) = (self.axis(), self.hinge(), load.inertia);
        let mut at: Vec<(Vec3, f32)> = Vec::with_capacity(hands.len());
        for k in 1..=steps as usize {
            let f = k as f32 / steps;
            at.clear();
            at.extend(hands.iter().map(|(from, to, r)| (from.lerp(*to, f), *r)));
            let mut torque = 0.0f32;
            for head in heads {
                let span = (head.y - BODY_PUSH_SPAN.0, head.y - BODY_PUSH_SPAN.1);
                let Some((point, n, depth)) = self.body_contact(s.angle, *head, span, BODY_PUSH_RADIUS) else { continue };
                let arm = point - hinge;
                let closing = (axis * s.spin).cross(arm).dot(n);
                torque += arm.cross(-n * body_push_force(depth, closing)).dot(axis);
            }
            for (from, to, local) in pulls {
                let (centre, rot) = self.pose_at(s.angle);
                let point = centre + rot * *local;
                let arm = point - hinge;
                let moving = (axis * s.spin).cross(arm) - (*to - *from) / dt;
                torque += arm.cross(door_pull_force(from.lerp(*to, f) - point, moving)).dot(axis);
            }
            let stop = i * s.spin.abs() / h;
            let viscous = (load.friction * s.spin).clamp(-stop, stop);
            torque -= viscous;
            torque -= (i * s.spin / h - viscous).clamp(-HINGE_DRY_FRICTION, HINGE_DRY_FRICTION);
            if load.closer > 0.0 {
                torque -= load.closer * s.angle + CLOSER_PRELOAD * (s.angle / 2f32.to_radians()).clamp(-1.0, 1.0);
            }
            s.spin = (s.spin + torque / i * h).clamp(-HINGE_MAX_SPIN, HINGE_MAX_SPIN);
            s.spin = soft_stop(s.angle, s.spin, self.lo, self.hi);
            s.angle += s.spin * h;
            if s.angle <= self.lo || s.angle >= self.hi {
                s.angle = s.angle.clamp(self.lo, self.hi);
                s.spin = 0.0;
            }

            let cleared = self.clear_spheres(s.angle, &at);
            if cleared != s.angle {
                let pushed = (cleared - s.angle) / h;
                if pushed.abs() > s.spin.abs() || pushed * s.spin < 0.0 {
                    s.spin = pushed.clamp(-HINGE_MAX_SPIN, HINGE_MAX_SPIN);
                }
                s.angle = cleared;
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn leaf() -> DoorGeom {
        DoorGeom::new(
            Vec3::new(0.0, 1.05, 0.0),
            Quat::IDENTITY,
            Vec3::new(0.4, 1.05, 0.02),
            &DoorDef { open_min_deg: -100.0, ..Default::default() },
        )
    }

    #[test]
    fn the_leaf_turns_about_its_hinged_edge() {
        let d = leaf();
        assert!((d.hinge() - Vec3::new(-0.4, 1.05, 0.0)).length() < 1e-6);
        let (c, r) = d.pose_at(FRAC_PI_2);

        assert!((c - Vec3::new(-0.4, 1.05, -0.4)).length() < 1e-5, "{c}");
        assert!((d.angle_of(r) - FRAC_PI_2).abs() < 1e-5);
        assert!((d.angle_of(d.pose_at(-1.2).1) + 1.2).abs() < 1e-5);

        let hinge_after = c + r * d.hinge_local;
        assert!((hinge_after - d.hinge()).length() < 1e-5);
    }

    #[test]
    fn a_turned_closed_rotation_measures_angles_from_itself() {
        let def = DoorDef { hinge: HingeSide::Right, ..Default::default() };
        let rot = Quat::from_rotation_y(0.7);
        let d = DoorGeom::new(Vec3::new(3.0, 1.0, -3.0), rot, Vec3::new(0.4, 1.0, 0.02), &def);
        for a in [-0.5f32, 0.0, 0.3, 1.5] {
            assert!((d.angle_of(d.pose_at(a).1) - a).abs() < 1e-5);
        }
        assert!((d.hinge() - (Vec3::new(3.0, 1.0, -3.0) + rot * Vec3::new(0.4, 0.0, 0.0))).length() < 1e-6);
    }

    #[test]
    fn a_hand_in_the_leaf_turns_it_just_clear_and_never_past_its_stop() {
        let d = leaf();

        let hand = (Vec3::new(0.2, 1.0, 0.03), 0.04);
        let a = d.clear_spheres(0.0, &[hand]);
        assert!(a != 0.0, "the hand is inside the closed leaf and must move it");
        let after = d.clear_spheres(a, &[hand]);
        assert!((after - a).abs() < 1e-5, "cleared once, it stays where it was put");

        let (c, r) = d.pose_at(a);
        let local = r.inverse() * (hand.0 - c);
        assert!(local.z.abs() >= d.half.z + hand.1 - 1e-3, "still inside: {local}");

        assert!(a > 0.0, "{a}");

        let one_way = DoorGeom { lo: 0.0, hi: 0.0, ..d };
        assert_eq!(one_way.clear_spheres(0.0, &[hand]), 0.0);

        assert_eq!(d.clear_spheres(0.0, &[(Vec3::new(0.6, 1.0, 0.0), 0.04)]), 0.0);
        assert_eq!(d.clear_spheres(0.0, &[(Vec3::new(0.2, 2.5, 0.0), 0.04)]), 0.0);
    }

    #[test]
    fn a_body_touches_the_leaf_only_within_its_radius_and_pushes_it_away() {
        let d = leaf();
        let span = (0.3, 1.6);

        let (p, n, depth) = d.body_contact(0.0, Vec3::new(0.0, 0.0, 0.27), span, BODY_PUSH_RADIUS).expect("touching");
        assert!((depth - (0.32 - 0.25)).abs() < 1e-4, "{depth}");
        assert!((n - Vec3::Z).length() < 1e-5, "points from the leaf to the body: {n}");
        assert!((p.z - 0.02).abs() < 1e-5);

        assert!(d.body_contact(0.0, Vec3::new(0.0, 0.0, 1.0), span, BODY_PUSH_RADIUS).is_none());
        assert!(d.body_contact(0.0, Vec3::new(0.0, 0.0, 0.27), (2.3, 3.0), BODY_PUSH_RADIUS).is_none());

        assert_eq!(body_push_force(0.0, -1.0), 0.0);
        assert_eq!(body_push_force(10.0, 0.0), BODY_PUSH_MAX);
    }

    #[test]
    fn a_door_seals_its_doorway_only_near_closed() {
        assert!(DoorGeom::shut(0.0) && DoorGeom::shut(1.9f32.to_radians()));
        assert!(!DoorGeom::shut(-3.0f32.to_radians()));
        assert_eq!(DoorGeom::open_fraction(-FRAC_PI_2 * 2.0), 1.0);
    }

    #[test]
    fn authored_limits_always_include_closed() {
        let d = DoorDef { open_min_deg: 20.0, open_max_deg: 90.0, ..Default::default() };
        assert_eq!(d.range().0, 0.0);
        let d = DoorDef { open_min_deg: 90.0, open_max_deg: -45.0, ..Default::default() };
        let (lo, hi) = d.range();
        assert!((lo + 45f32.to_radians()).abs() < 1e-6 && (hi - 90f32.to_radians()).abs() < 1e-6);
    }

    /// BUTT HINGES: the pin at the face the door opens toward. A leaf 3 mm
    /// from its jamb swings a right angle and more to that side and no part
    /// of it comes nearer the jamb than it hung -- which a pin at the middle
    /// of the edge cannot do: there the corners turn 2 cm into the frame.
    #[test]
    fn a_butt_hinged_leaf_swings_clear_of_its_jamb() {
        let half = Vec3::new(0.38, 1.08, 0.022);
        let jamb = -0.383f32;
        let into_the_jamb = |def: &DoorDef| {
            let g = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, half, def);
            (0..=105)
                .flat_map(|deg| g.corners_at(-(deg as f32).to_radians()))
                .filter(|c| c.z < 0.0219)
                .map(|c| jamb - c.x)
                .fold(f32::MIN, f32::max)
        };
        let butt = DoorDef { open_min_deg: -105.0, open_max_deg: 0.0, pin_offset: 0.022, ..Default::default() };
        let middle = DoorDef { open_min_deg: -105.0, open_max_deg: 0.0, ..Default::default() };
        assert!(into_the_jamb(&butt) <= -0.0029, "a butt-hinged leaf turned {} m into its jamb", into_the_jamb(&butt));
        assert!(into_the_jamb(&middle) > 0.015, "and one pinned at the middle of its edge does: {}", into_the_jamb(&middle));
        let pivot = DoorDef { open_min_deg: -105.0, open_max_deg: 105.0, pin_inset: 0.022, ..Default::default() };
        let g = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, half, &pivot);
        let swept = (-105..=105)
            .flat_map(|deg| g.corners_at((deg as f32).to_radians()).into_iter().step_by(2))
            .map(|c| c.x)
            .fold(f32::MAX, f32::min);
        assert!(swept > -0.38 - 0.0095 && swept < -0.38, "{swept}");
        let g = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, half, &butt);
        let (c, r) = g.pose_at(-1.3);
        assert!((c + r * g.hinge_local - g.hinge()).length() < 1e-5);
        assert!((g.hinge() - Vec3::new(-0.38, 1.1, 0.022)).length() < 1e-6);
    }

    /// A hand clears a butt-hinged leaf as it clears any other: turned just
    /// far enough that the hand is outside its slab.
    #[test]
    fn a_hand_clears_a_leaf_whose_pin_is_off_its_middle() {
        let def = DoorDef { open_min_deg: -100.0, open_max_deg: 100.0, pin_offset: 0.022, pin_inset: 0.01, ..Default::default() };
        let d = DoorGeom::new(Vec3::new(0.0, 1.05, 0.0), Quat::IDENTITY, Vec3::new(0.4, 1.05, 0.022), &def);
        for hand in [(Vec3::new(0.2, 1.0, 0.03), 0.04), (Vec3::new(0.25, 1.0, -0.04), 0.05), (Vec3::new(-0.1, 0.6, 0.0), 0.04)] {
            let a = d.clear_spheres(0.0, &[hand]);
            assert!(a != 0.0, "the hand at {} is in the leaf and must move it", hand.0);
            let (c, r) = d.pose_at(a);
            let local = r.inverse() * (hand.0 - c);
            assert!(local.z.abs() >= d.half.z + hand.1 - 2e-3, "still inside: {local} at {} deg", a.to_degrees());
            assert!(local.z.abs() <= d.half.z + hand.1 + 6e-3, "turned further than clear: {local} at {} deg", a.to_degrees());
        }
    }

    /// THE LATCH'S ONE RULE: the bolt is drawn while a hand is at the handle,
    /// on either face, and goes home when the leaf is shut with no hand there.
    #[test]
    fn the_bolt_is_drawn_by_a_hand_at_the_handle_and_goes_home_when_shut() {
        let def = DoorDef { latch: true, handle: Some([0.31, -0.1, 0.0]), ..Default::default() };
        let latch = DoorLatch::of(&def).unwrap();
        let pose = (Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY);
        let at = Vec3::new(0.31, 1.0, 0.0);
        for z in [-0.2, -0.07, 0.0, 0.07, 0.2] {
            assert!(latch.worked(pose, 0.02, [at + Vec3::Z * z]), "a hand {z} m off the leaf at the handle");
        }
        assert!(!latch.worked(pose, 0.02, [at + Vec3::Z * 0.25]), "a hand a quarter metre off the leaf");
        assert!(!latch.worked(pose, 0.02, [at + Vec3::X * 0.2]), "a hand 20 cm along the leaf from the handle");
        assert!(!latch.worked(pose, 0.02, [Vec3::new(-0.31, 1.0, 0.05)]), "a hand at the hinge");
        assert_eq!(latch.hold_point(pose, 0.02, at - Vec3::Z * 0.3).z, -(0.02 + HANDLE_STANDOFF));
        assert_eq!(latch.hold_point(pose, 0.02, at + Vec3::Z * 0.3).z, 0.02 + HANDLE_STANDOFF);
        assert!(latch.bolt(false, 0.01, 0.0, false));
        assert!(!latch.bolt(false, 0.2, 0.0, false) && !latch.bolt(false, 0.0, 6.0, false) && !latch.bolt(false, 0.0, 0.0, true));
        assert!(latch.bolt(true, 0.004, 0.5, false) && !latch.bolt(true, 0.0, 0.0, true));
        let pull = DoorLatch::of(&DoorDef { handle: Some([0.3, 0.0, 0.0]), ..Default::default() }).unwrap();
        assert!(!pull.bolt(false, 0.0, 0.0, false));
        assert!(DoorLatch::of(&DoorDef { latch: true, ..Default::default() }).is_none());
        let old: DoorDef = serde_json::from_str(r#"{"hinge":"Right","mass":22,"open_min_deg":-90,"open_max_deg":90}"#).unwrap();
        assert!(!old.latch && old.handle.is_none() && old.lever.is_none() && old.pin_inset == 0.0 && old.pin_offset == 0.0);
        let text = serde_json::to_string(&old).unwrap();
        assert!(!text.contains("latch") && !text.contains("handle") && !text.contains("pin_") && !text.contains("lever"), "{text}");
    }

    /// A soft stop lets a leaf move freely away from it and in the middle of
    /// its swing, and takes its speed off over the last degrees.
    #[test]
    fn a_soft_stop_slows_only_what_turns_into_it() {
        let (lo, hi) = (0.0, 100f32.to_radians());
        assert_eq!(soft_stop(0.8, 3.0, lo, hi), 3.0, "mid swing");
        assert_eq!(soft_stop(hi - 0.001, -5.0, lo, hi), -5.0, "leaving the stop");
        assert!(soft_stop(hi - 0.02, 5.0, lo, hi) < 1.0, "two hundredths of a radian from it at 5 rad/s");
        assert!((soft_stop(hi, 5.0, lo, hi) - STOP_CREEP).abs() < 1e-6 && (soft_stop(lo, -5.0, lo, hi) + STOP_CREEP).abs() < 1e-6);
        let mut a = 0.5f32;
        for _ in 0..200 {
            let step = soft_stop(a, 20.0, lo, hi) / 60.0;
            assert!(a + step <= hi + STOP_CREEP / 60.0 + 1e-6, "{}", a + step);
            a = (a + step).min(hi);
        }
        assert!((a - hi).abs() < 1e-6, "it never arrived: {a}");
    }

    /// THE HEADSET'S OWN HINGE: a player walking into a shut double-acting
    /// leaf with a closer -- stopped by the walk 25 cm from it, as
    /// `locomotion` stops them -- swings it open and walks through; let go,
    /// the closer shuts it.
    #[test]
    fn walking_into_a_door_on_the_headsets_own_hinge_opens_it() {
        let def = DoorDef { open_min_deg: -90.0, open_max_deg: 90.0, mass: 22.0, closer: 15.0, friction: 10.0, ..Default::default() };
        let g = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, Vec3::new(0.3825, 1.09, 0.02), &def);
        let load = HingeLoad::of(&g, &def);
        let mut s = HingeState::default();

        let mut head = Vec3::new(g.hinge().x + 0.25, 1.6, 1.5);
        let dt = 1.0 / 72.0;
        let mut through = None;
        for frame in 0..(72 * 6) {
            let mut next = head + Vec3::new(0.0, 0.0, -1.2 * dt);

            if let Some((_, n, depth)) = g.body_contact(s.angle, next, (next.y - 1.3, next.y - 0.25), 0.25) {
                next += n * depth;
            }
            head = next;
            s = g.step_hinge(s, &load, &[head], &[], dt);
            if head.z < -0.6 && through.is_none() {
                through = Some(frame);
            }
        }
        let opened = s.angle.to_degrees();
        assert!(through.is_some(), "the player never got through: head {head}, leaf at {opened:.1} deg");

        assert!(s.angle.abs() < 3f32.to_radians(), "the closer left it at {opened:.1} deg");
    }

    #[test]
    fn a_hand_knocks_a_leaf_on_the_headsets_own_hinge() {
        let def = DoorDef { open_min_deg: -90.0, open_max_deg: 90.0, mass: 22.0, friction: 2.0, ..Default::default() };
        let g = DoorGeom::new(Vec3::new(0.0, 1.1, 0.0), Quat::IDENTITY, Vec3::new(0.3825, 1.09, 0.02), &def);
        let load = HingeLoad::of(&g, &def);
        let mut s = HingeState::default();
        let dt = 1.0 / 72.0;

        for k in 0..8 {
            let hand = Vec3::new(g.hinge().x + 0.6, 1.0, 0.12 - 0.02 * k as f32);
            let was = hand + Vec3::new(0.0, 0.0, 0.02);
            s = g.step_hinge(s, &load, &[], &[(was, hand, 0.05)], dt);
        }
        let knocked = s.angle;
        assert!(knocked.abs() > 5f32.to_radians(), "the knock moved it {:.1} deg", knocked.to_degrees());
        for _ in 0..(72 * 8) {
            s = g.step_hinge(s, &load, &[], &[], dt);
            assert!(s.angle >= g.lo - 1e-5 && s.angle <= g.hi + 1e-5);
        }
        assert!(s.angle.abs() > knocked.abs(), "it swung on: {:.1} deg", s.angle.to_degrees());
        assert_eq!(s.spin, 0.0, "and came to rest");
    }
}
