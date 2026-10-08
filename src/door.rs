//! DOORS: a leaf hung on a vertical hinge, pushed open by hands and bodies.
//!
//! The object's `cuboid` is the leaf as it hangs CLOSED -- its centre, its
//! rotation and its half size -- and the leaf turns about one of its two
//! vertical edges (`DoorDef::hinge`). The physics is a PhysX joint with only
//! the twist about that edge free, limited to `open_min_deg..open_max_deg`, with hinge
//! friction and an optional door closer as the twist drive
//! (`PhysicsWorld::spawn_door`). Hands push it by contact (they are kinematic
//! spheres); a body pushes it by a soft contact (`body_contact`), because the
//! player is not a physics body -- they stop at walls by ray -- and a soft push
//! cannot fight the joint's limit into a jitter the way a hard kinematic
//! capsule does when a door is pressed against its stop.
//!
//! The geometry here is pure maths shared by the server (authoritative), the
//! client (drawing it, colliding with it, and drawing it ahead of the server
//! where a local hand pushes it: `clear_spheres`) and the renderer's caller
//! (the leaf's corners for its shadow tiles, `shut` for portal culling).
//!
//! LIGHT: a door is never baked. Lightmaps, lamp masks and probe photographs
//! hold the level with its doorways OPEN, and the door adds what it changes
//! live: its own shadows from the lamps, the torch and the sun, which cover the
//! light a closed door stops. See docs/doors-2026-10-07.md.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Which vertical edge of the leaf the hinge is on, in the object's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HingeSide {
    /// The leaf's -x edge.
    #[default]
    Left,
    /// The leaf's +x edge.
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

/// A door hung from this object. See the module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoorDef {
    #[serde(default)]
    pub hinge: HingeSide,
    /// How far the leaf turns from closed, degrees: positive is a turn about
    /// +y (anticlockwise seen from above). 0 to 100 opens one way to a stop
    /// past square; -100 to 100 swings both ways.
    #[serde(default)]
    pub open_min_deg: f32,
    #[serde(default = "default_open_max")]
    pub open_max_deg: f32,
    /// The leaf's mass, kg. A solid interior door is 20-35.
    #[serde(default = "default_door_mass")]
    pub mass: f32,
    /// Hinge friction, N m s/rad: how quickly a pushed door slows.
    #[serde(default = "default_door_friction")]
    pub friction: f32,
    /// A door closer's spring, N m/rad, pulling the leaf back to closed; 0 for
    /// none. A real closer gives about 10-25 N m at 90 degrees.
    #[serde(default)]
    pub closer: f32,
    /// The leaf's surface friction against hands.
    #[serde(default = "default_surface_friction")]
    pub surface_friction: f32,
    /// Where the leaf stands when the level loads, degrees from closed.
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
    /// The limits in radians, `lo <= 0 <= hi` whatever was authored.
    pub fn range(&self) -> (f32, f32) {
        let (a, b) = (self.open_min_deg, self.open_max_deg);
        let (lo, hi) = (a.min(b).to_radians(), a.max(b).to_radians());
        (lo.min(0.0), hi.max(0.0))
    }
}

/// Below this many degrees from closed a door seals its doorway: portal
/// culling treats the doorway as a wall. Two degrees on an 80 cm leaf is a
/// 2.8 cm gap at the latch edge -- a sliver no culled thing shows through
/// that the frame's jamb does not already hide.
pub const SHUT_DEG: f32 = 2.0;

/// A door's leaf where it hangs: everything the physics, the client and the
/// renderer need to place it at any angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorGeom {
    /// The leaf's centre and rotation when closed (the object's cuboid).
    pub closed_pos: Vec3,
    pub closed_rot: Quat,
    pub half: Vec3,
    /// The hinge's point in the leaf's own frame: the middle of the hinged edge.
    pub hinge_local: Vec3,
    /// `DoorDef::range`, radians.
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

    /// The door an object hangs, as authored (closed).
    pub fn of(obj: &crate::scene::GameObject) -> Option<Self> {
        let def = obj.door.as_ref()?;
        Some(Self::new(obj.cuboid.position, obj.cuboid.rotation, obj.cuboid.half_size, def))
    }

    /// The hinge's middle, world.
    pub fn hinge(&self) -> Vec3 {
        self.closed_pos + self.closed_rot * self.hinge_local
    }

    /// The hinge's axis, world: the leaf's own +y.
    pub fn axis(&self) -> Vec3 {
        self.closed_rot * Vec3::Y
    }

    /// The leaf's centre and rotation turned `angle` radians about the hinge.
    pub fn pose_at(&self, angle: f32) -> (Vec3, Quat) {
        let rot = self.closed_rot * Quat::from_rotation_y(angle);
        (self.hinge() - rot * self.hinge_local, rot)
    }

    /// How far a leaf at rotation `rot` is turned from closed, radians in
    /// (-pi, pi]: its twist about the hinge axis.
    pub fn angle_of(&self, rot: Quat) -> f32 {
        let q = self.closed_rot.inverse() * rot;
        let q = if q.w < 0.0 { -q } else { q };
        2.0 * q.y.atan2(q.w)
    }

    /// The leaf's eight corners at `angle`, world.
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

    /// Whether a leaf at `angle` seals its doorway (`SHUT_DEG`).
    pub fn shut(angle: f32) -> bool {
        angle.abs() < SHUT_DEG.to_radians()
    }

    /// How open the doorway is, 0 shut to 1 at a right angle or more.
    pub fn open_fraction(angle: f32) -> f32 {
        (angle.abs() / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0)
    }

    /// The leaf's moment of inertia about its hinge, kg m^2, at `mass`: a
    /// slab turning about one edge.
    pub fn hinge_inertia(&self, mass: f32) -> f32 {
        let (w, t) = (2.0 * self.half.x, 2.0 * self.half.z);
        mass * (w * w + t * t) / 3.0
    }

    /// The leaf's direction from the hinge at `angle`, in the hinge's frame
    /// (the closed leaf's axes): along the leaf, away from the hinge.
    fn along(&self, angle: f32) -> Vec3 {
        Quat::from_rotation_y(angle) * (-self.hinge_local).normalize_or_zero()
    }

    /// THE ANGLE THAT CLEARS `spheres` (world centre, radius) -- a hand pushing
    /// the leaf -- starting from `angle`: the leaf turned just far enough
    /// that no sphere sits inside it, within the limits. The client draws a
    /// door it is pushing at this angle where the server's is still behind, so
    /// the leaf moves with the hand with no network lag and a hand never
    /// passes through the drawn door; the server's answer, a few frames later,
    /// is the same push.
    pub fn clear_spheres(&self, angle: f32, spheres: &[(Vec3, f32)]) -> f32 {
        let width = 2.0 * self.half.x;
        let base = (-self.hinge_local).normalize_or_zero();
        let mut a = angle;
        // Twice: clearing one sphere can turn the leaf into another.
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
                // The sphere's bearing about +y from the closed leaf's
                // direction, as `angle` is measured.
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

    /// A BODY AGAINST THE LEAF: the vertical capsule of `radius` round
    /// `body` (world; its x and z are the body's, `y_range` its height)
    /// against the leaf at `angle`. `None` when they do not touch; else the
    /// leaf's point nearest the body (world), the unit direction from that
    /// point toward the body (horizontal), and how deep the body reaches in.
    pub fn body_contact(&self, angle: f32, body: Vec3, y_range: (f32, f32), radius: f32) -> Option<(Vec3, Vec3, f32)> {
        let (centre, rot) = self.pose_at(angle);
        let (lo, hi) = (centre.y - self.half.y, centre.y + self.half.y);
        if y_range.1 < lo || y_range.0 > hi {
            return None;
        }
        // In the leaf's frame, flattened: a rectangle against a point.
        let q = rot.inverse() * Vec3::new(body.x - centre.x, 0.0, body.z - centre.z);
        let (hx, hz) = (self.half.x, self.half.z);
        let nearest = Vec3::new(q.x.clamp(-hx, hx), 0.0, q.z.clamp(-hz, hz));
        let off = Vec3::new(q.x - nearest.x, 0.0, q.z - nearest.z);
        let d = off.length();
        let (normal, depth) = if d > 1e-5 {
            (off / d, radius - d)
        } else {
            // Inside the leaf: out through the nearer face.
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

/// An angle into (-pi, pi].
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

/// HOW HARD A BODY PUSHES A DOOR: a spring on how far it reaches into the
/// leaf, N/m. The client stops a walking player `locomotion`'s 25 cm from a
/// leaf; the body pushes from `BODY_PUSH_RADIUS`, so a player walking into a
/// door presses it with about `(0.32 - 0.25) * 1500` = 105 N -- a firm push at
/// walking pace -- and the door swings away from them as fast as they walk.
pub const BODY_PUSH_STIFFNESS: f32 = 1500.0;
/// Damping on the leaf's speed toward the body, N s/m.
pub const BODY_PUSH_DAMPING: f32 = 120.0;
/// The most a body pushes, N: a shoulder, not a battering ram.
pub const BODY_PUSH_MAX: f32 = 300.0;
/// The body's radius as it pushes a leaf, metres. See `BODY_PUSH_STIFFNESS`.
pub const BODY_PUSH_RADIUS: f32 = 0.32;
/// The body's height under the head that pushes, metres: from the knees to
/// the shoulders.
pub const BODY_PUSH_SPAN: (f32, f32) = (1.3, 0.25);

/// The force a body at `depth` in the leaf pushes with, along the leaf's
/// side away from it, when the leaf's point moves toward the body at
/// `closing` m/s (negative when it moves away).
pub fn body_push_force(depth: f32, closing: f32) -> f32 {
    (BODY_PUSH_STIFFNESS * depth + BODY_PUSH_DAMPING * closing).clamp(0.0, BODY_PUSH_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    /// A leaf in the xy plane at the origin, 80 cm wide, hinged on its -x
    /// edge, opening either way.
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
        // A quarter turn about +y takes the leaf's +x to -z.
        assert!((c - Vec3::new(-0.4, 1.05, -0.4)).length() < 1e-5, "{c}");
        assert!((d.angle_of(r) - FRAC_PI_2).abs() < 1e-5);
        assert!((d.angle_of(d.pose_at(-1.2).1) + 1.2).abs() < 1e-5);
        // The hinge does not move.
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
        // A 4 cm hand 60 cm out along the closed leaf, a centimetre in front
        // of its +z face: it must turn the leaf away from itself, toward -z.
        let hand = (Vec3::new(0.2, 1.0, 0.03), 0.04);
        let a = d.clear_spheres(0.0, &[hand]);
        assert!(a != 0.0, "the hand is inside the closed leaf and must move it");
        let after = d.clear_spheres(a, &[hand]);
        assert!((after - a).abs() < 1e-5, "cleared once, it stays where it was put");
        // The hand is now just outside the turned leaf's slab.
        let (c, r) = d.pose_at(a);
        let local = r.inverse() * (hand.0 - c);
        assert!(local.z.abs() >= d.half.z + hand.1 - 1e-3, "still inside: {local}");
        // Toward -z the leaf turns positive (+x goes to -z).
        assert!(a > 0.0, "{a}");
        // A one-way door against its stop does not move.
        let one_way = DoorGeom { lo: 0.0, hi: 0.0, ..d };
        assert_eq!(one_way.clear_spheres(0.0, &[hand]), 0.0);
        // Out of reach: past the leaf's end, above it, or at the hinge.
        assert_eq!(d.clear_spheres(0.0, &[(Vec3::new(0.6, 1.0, 0.0), 0.04)]), 0.0);
        assert_eq!(d.clear_spheres(0.0, &[(Vec3::new(0.2, 2.5, 0.0), 0.04)]), 0.0);
    }

    #[test]
    fn a_body_touches_the_leaf_only_within_its_radius_and_pushes_it_away() {
        let d = leaf();
        let span = (0.3, 1.6);
        // 25 cm in front of the leaf's middle: inside the 32 cm push radius.
        let (p, n, depth) = d.body_contact(0.0, Vec3::new(0.0, 0.0, 0.27), span, BODY_PUSH_RADIUS).expect("touching");
        assert!((depth - (0.32 - 0.25)).abs() < 1e-4, "{depth}");
        assert!((n - Vec3::Z).length() < 1e-5, "points from the leaf to the body: {n}");
        assert!((p.z - 0.02).abs() < 1e-5);
        // A metre off, or under the leaf, touches nothing.
        assert!(d.body_contact(0.0, Vec3::new(0.0, 0.0, 1.0), span, BODY_PUSH_RADIUS).is_none());
        assert!(d.body_contact(0.0, Vec3::new(0.0, 0.0, 0.27), (2.3, 3.0), BODY_PUSH_RADIUS).is_none());
        // The force only pushes, and is capped.
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
}
