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

pub const SHUT_DEG: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorGeom {

    pub closed_pos: Vec3,
    pub closed_rot: Quat,
    pub half: Vec3,

    pub hinge_local: Vec3,

    pub lo: f32,
    pub hi: f32,
}

impl DoorGeom {
    pub fn new(closed_pos: Vec3, closed_rot: Quat, half: Vec3, def: &DoorDef) -> Self {
        let x = match def.hinge {
            HingeSide::Left => -half.x,
            HingeSide::Right => half.x,
        };
        let (lo, hi) = def.range();
        Self { closed_pos, closed_rot, half, hinge_local: Vec3::new(x, 0.0, 0.0), lo, hi }
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
        let width = 2.0 * self.half.x;
        let base = (-self.hinge_local).normalize_or_zero();
        let mut a = angle;

        for _ in 0..2 {
            for &(c, s) in spheres {
                let p = self.closed_rot.inverse() * (c - self.hinge());
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

    pub fn step_hinge(&self, mut s: HingeState, load: &HingeLoad, heads: &[Vec3], hands: &[(Vec3, Vec3, f32)], dt: f32) -> HingeState {
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
            let stop = i * s.spin.abs() / h;
            let viscous = (load.friction * s.spin).clamp(-stop, stop);
            torque -= viscous;
            torque -= (i * s.spin / h - viscous).clamp(-HINGE_DRY_FRICTION, HINGE_DRY_FRICTION);
            if load.closer > 0.0 {
                torque -= load.closer * s.angle + CLOSER_PRELOAD * (s.angle / 2f32.to_radians()).clamp(-1.0, 1.0);
            }
            s.spin = (s.spin + torque / i * h).clamp(-HINGE_MAX_SPIN, HINGE_MAX_SPIN);
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
