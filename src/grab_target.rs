//! WHAT EACH HAND WILL TAKE WHEN ITS GRIP CLOSES, chosen before it closes so
//! the thing can be lit (the glow most VR games give it) and the press takes
//! exactly what was lit.
//!
//! A hand reaches three ways, and what is offered is scored by one cost, in
//! metres:
//!
//! - TOUCHING: anything within [`TOUCH`] of the grip, whichever way the hand
//!   points -- a rock under the palm. Chosen by distance alone.
//! - IN FRONT OF THE PALM: within [`NEAR`], no more than [`NEAR_HALF_DEG`]
//!   off the hand's aim -- the forearm's length, not behind the wrist.
//! - ALONG THE HAND: within [`CONE_REACH`] and [`CONE_HALF_DEG`] of the aim --
//!   a rock a short lean away that the hand points at. NOT a force grab: the
//!   reach stays under the hold's break distance (`HOLD_BREAK` 0.75 m), so
//!   the spring that brings the body to the hand (`grab_free`) never lets go
//!   of it on the way.
//!
//! Cost = surface distance + [`AIM_METRES`] x (1 - cos of the angle off the
//! aim), the angle's share fading in from [`CONTACT`] to [`NEAR`]: under the
//! palm the nearest wins, further out the one pointed at. The angle is taken
//! to the thing's bounding sphere, not its centre, so a crate is as easy to
//! point at as it is wide.
//!
//! HYSTERESIS, so the light does not flicker between two rocks: the thing a
//! hand has keeps it while it is within looser limits ([`KEEP_SLACK`],
//! [`KEEP_DEG`]), and another takes over only when it is better by
//! [`SWITCH_MARGIN`].
//!
//! TWO HANDS: a hand that holds something offers nothing; neither hand's
//! choice is what either hand holds; and when both would take the same thing
//! the better-placed hand has it and the other its next best.
//!
//! What is offered comes from the caller -- a grid query round the hand
//! (`GrabGrid::each_within`), never a walk over the level's objects -- and
//! this module is pure arithmetic over it, host-tested.

use glam::{Quat, Vec3};

/// Anything this close to the grip, metres, whichever way the hand points:
/// a palm's width past the old touch-only grab's 15 cm (headset 2026-10-14:
/// "we need the pickup radius further").
pub const TOUCH: f32 = 0.20;
/// In front of the palm: this far, metres, within [`NEAR_HALF_DEG`] of the aim.
pub const NEAR: f32 = 0.45;
pub const NEAR_HALF_DEG: f32 = 75.0;
/// Along the hand: this far, metres, within [`CONE_HALF_DEG`] of the aim.
pub const CONE_REACH: f32 = 0.62;
pub const CONE_HALF_DEG: f32 = 22.0;
/// What pointing away costs, metres at a right angle (see the module).
pub const AIM_METRES: f32 = 1.2;
/// Within this of the grip, metres, the angle costs nothing: in contact.
pub const CONTACT: f32 = 0.06;
/// The thing a hand already has keeps it out to these looser limits.
pub const KEEP_SLACK: f32 = 1.2;
pub const KEEP_DEG: f32 = 6.0;
/// Another thing takes over only when it is better by this, metres of cost.
pub const SWITCH_MARGIN: f32 = 0.05;
/// How far round a hand a caller must ask: the farthest any reach goes.
pub const QUERY_REACH: f32 = CONE_REACH * KEEP_SLACK;

/// How a hand takes what it is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeKind {
    /// A loose body, held where the hand meets it (`grab_free`).
    Loose,
    /// A door, by its handle (`grab_door`): only within the handle's own
    /// reach, which the server holds the hand to.
    Handle,
}

/// One thing a hand could take, as the caller measured it.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    /// What a press grabs.
    pub id: String,
    /// What is lit while it is the choice: the thing itself, or for a door
    /// the lever on its handle.
    pub glow: String,
    pub kind: TakeKind,
    /// From the grip to its surface, metres (zero inside it).
    pub distance: f32,
    /// Radians off the hand's aim, to its bounding sphere (zero inside it).
    pub angle: f32,
    /// A reach of its own, metres, any direction, instead of the hand's
    /// three (a door's handle: `door::HANDLE_REACH`).
    pub reach: Option<f32>,
}

impl Offer {
    /// A box offered to a hand at `grip` aiming along `aim` (unit): its
    /// surface distance `distance` (as `distance_to_oriented_box` gives it)
    /// and the angle to its bounding sphere.
    pub fn of_box(id: &str, kind: TakeKind, grip: Vec3, aim: Vec3, centre: Vec3, half: Vec3, distance: f32) -> Self {
        Self { id: id.to_string(), glow: id.to_string(), kind, distance, angle: angle_to_sphere(grip, aim, centre, half.length()), reach: None }
    }
}

/// Radians between `aim` (unit) from `from` and the nearest edge of a sphere
/// at `centre` of `radius`: zero when the aim passes through it or `from` is
/// inside it.
pub fn angle_to_sphere(from: Vec3, aim: Vec3, centre: Vec3, radius: f32) -> f32 {
    let to = centre - from;
    let d = to.length();
    if d <= radius || d < 1e-6 {
        return 0.0;
    }
    let off = (to.dot(aim) / d).clamp(-1.0, 1.0).acos();
    (off - (radius / d).clamp(0.0, 1.0).asin()).max(0.0)
}

/// What taking `offer` costs a hand, metres (lower is better), or none if it
/// is out of reach; `slack` 1 for a newcomer, [`KEEP_SLACK`] (with
/// [`KEEP_DEG`] more of cone) for the thing the hand already has.
pub fn cost(offer: &Offer, keep: bool) -> Option<f32> {
    let (slack, more) = if keep { (KEEP_SLACK, KEEP_DEG.to_radians()) } else { (1.0, 0.0) };
    let d = offer.distance.max(0.0);
    if let Some(r) = offer.reach {
        return (d <= r * if keep { slack } else { 1.0 }).then_some(d);
    }
    let a = offer.angle.max(0.0);
    let inside = d <= TOUCH * slack
        || (d <= NEAR * slack && a <= NEAR_HALF_DEG.to_radians() + more)
        || (d <= CONE_REACH * slack && a <= CONE_HALF_DEG.to_radians() + more);
    inside.then(|| d + AIM_METRES * (1.0 - a.cos()) * ((d - CONTACT) / (NEAR - CONTACT)).clamp(0.0, 1.0))
}

/// A hand's choice: what it will take, and what is lit.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub id: String,
    pub glow: String,
    pub kind: TakeKind,
    pub cost: f32,
}

/// Each hand's choice, kept from frame to frame: `[left, right]`.
#[derive(Debug, Clone, Default)]
pub struct GrabTargets {
    pub hands: [Option<Choice>; 2],
    /// How many times a hand took a new choice (for the log, and for a tick
    /// of the controller when haptics are wired).
    pub acquired: u64,
}

impl GrabTargets {
    /// This frame's choices from what each hand is offered, `[left, right]`
    /// (`None`: no hand there -- untracked), and what each holds. Returns the
    /// hands whose choice changed to a new thing this frame.
    pub fn update(&mut self, offers: [Option<&[Offer]>; 2], held: [Option<&str>; 2]) -> [bool; 2] {
        let is_held = |id: &str| held.iter().flatten().any(|h| *h == id);
        let ranked: [Vec<(f32, &Offer)>; 2] = [0, 1].map(|h| {
            let (Some(offers), None) = (offers[h], held[h]) else { return Vec::new() };
            let had = self.hands[h].as_ref().map(|c| c.id.as_str());
            let mut list: Vec<(f32, &Offer)> = offers
                .iter()
                .filter(|o| !is_held(&o.id))
                .filter_map(|o| Some((cost(o, had == Some(o.id.as_str()))?, o)))
                .collect();
            list.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
            list
        });
        let pick = |h: usize, not: Option<&str>| -> Option<(f32, &Offer)> {
            let list = &ranked[h];
            let best = list.iter().find(|(_, o)| Some(o.id.as_str()) != not).copied()?;
            let had = self.hands[h].as_ref().map(|c| c.id.as_str());
            match list.iter().find(|(_, o)| Some(o.id.as_str()) == had && Some(o.id.as_str()) != not) {
                Some(&kept) if best.0 >= kept.0 - SWITCH_MARGIN => Some(kept),
                _ => Some(best),
            }
        };
        let mut picks = [pick(0, None), pick(1, None)];
        if let (Some(l), Some(r)) = (picks[0], picks[1]) {
            if l.1.id == r.1.id {
                let had_left = self.hands[0].as_ref().is_some_and(|c| c.id == l.1.id);
                let left_keeps = l.0 < r.0 || (l.0 == r.0 && had_left);
                let other = if left_keeps { 1 } else { 0 };
                picks[other] = pick(other, Some(&l.1.id));
            }
        }
        let mut changed = [false; 2];
        for h in 0..2 {
            let next = picks[h].map(|(c, o)| Choice { id: o.id.clone(), glow: o.glow.clone(), kind: o.kind, cost: c });
            changed[h] = next.as_ref().is_some_and(|n| self.hands[h].as_ref().map(|c| &c.id) != Some(&n.id));
            self.acquired += changed[h] as u64;
            self.hands[h] = next;
        }
        changed
    }

    /// What a hand will take, `0` left, `1` right.
    pub fn choice(&self, hand: usize) -> Option<&Choice> {
        self.hands.get(hand)?.as_ref()
    }
}

/// How fast the glow comes up and goes, seconds (time constants), and how it
/// breathes while lit: a slow swell between [`GLOW_LOW`] and 1, which no
/// lamp does, so it is not taken for the object's own light.
pub const GLOW_IN: f32 = 0.05;
pub const GLOW_OUT: f32 = 0.08;
pub const GLOW_LOW: f32 = 0.7;
pub const GLOW_HZ: f32 = 1.1;
/// Below this a fading glow is put out: written zero, and forgotten.
pub const GLOW_OFF: f32 = 0.01;

/// THE GLOW ON EACH LIT THING, `0..1`: up while some hand has it as its
/// choice, down when none has. Only what is lit or fading is kept, so with
/// no choice there is nothing to do and nothing written.
#[derive(Debug, Clone, Default)]
pub struct GlowFade {
    /// (what is lit, its level without the breathing).
    lit: Vec<(String, f32)>,
}

impl GlowFade {
    /// A step of `dt` seconds at `time`; `wanted` the things lit now. Returns
    /// what to write: each lit or fading thing's level this frame (breathing
    /// included), and a zero once for each that has gone out.
    pub fn step(&mut self, dt: f32, time: f64, wanted: &[&str]) -> Vec<(String, f32)> {
        for w in wanted {
            if !self.lit.iter().any(|(id, _)| id == w) {
                self.lit.push((w.to_string(), 0.0));
            }
        }
        let breathe = GLOW_LOW + (1.0 - GLOW_LOW) * (0.5 + 0.5 * (std::f64::consts::TAU * GLOW_HZ as f64 * time).cos() as f32);
        let mut out = Vec::with_capacity(self.lit.len());
        self.lit.retain_mut(|(id, level)| {
            let on = wanted.contains(&id.as_str());
            let (to, tau) = if on { (1.0, GLOW_IN) } else { (0.0, GLOW_OUT) };
            *level += (to - *level) * if dt > 0.0 { 1.0 - (-dt / tau).exp() } else { 1.0 };
            if !on && *level < GLOW_OFF {
                out.push((id.clone(), 0.0));
                return false;
            }
            out.push((id.clone(), *level * breathe));
            true
        });
        out
    }

    pub fn is_idle(&self) -> bool {
        self.lit.is_empty()
    }
}

/// A hand's aim from its aim pose's rotation: along its -z.
pub fn aim_of(rotation: Quat) -> Vec3 {
    (rotation * Vec3::NEG_Z).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rock(id: &str, distance: f32, angle_deg: f32) -> Offer {
        Offer {
            id: id.into(),
            glow: id.into(),
            kind: TakeKind::Loose,
            distance,
            angle: angle_deg.to_radians(),
            reach: None,
        }
    }

    #[test]
    fn a_hand_reaches_by_touch_in_front_and_along_its_aim_and_no_further() {
        assert!(cost(&rock("a", 0.1, 170.0), false).is_some());
        assert!(cost(&rock("a", 0.3, 60.0), false).is_some());
        assert!(cost(&rock("a", 0.3, 110.0), false).is_none());
        assert!(cost(&rock("a", 0.55, 15.0), false).is_some());
        assert!(cost(&rock("a", 0.55, 40.0), false).is_none());
        assert!(cost(&rock("a", 0.65, 0.0), false).is_none());
        assert!(CONE_REACH * KEEP_SLACK < crate::rigid_physics_grab::HOLD_BREAK, "a choice must be within the hold's reach");
    }

    #[test]
    fn under_the_palm_the_nearest_wins_and_further_out_the_one_pointed_at() {
        assert!(cost(&rock("a", 0.02, 80.0), false).unwrap() < cost(&rock("b", 0.08, 0.0), false).unwrap());
        assert!(cost(&rock("a", 0.45, 2.0), false).unwrap() < cost(&rock("b", 0.40, 20.0), false).unwrap());
    }

    #[test]
    fn the_choice_does_not_flicker_between_two_rocks() {
        let mut t = GrabTargets::default();
        let mut switches = 0;
        let mut last = None;
        for k in 0..200 {
            let s = (k as f32 * 0.37).sin() * 0.02;
            let offers = [rock("a", 0.30 + s, 10.0), rock("b", 0.30 - s, 10.0)];
            t.update([None, Some(&offers)], [None, None]);
            let now = t.choice(1).map(|c| c.id.clone());
            if last.is_some() && now != last {
                switches += 1;
            }
            last = now;
        }
        assert_eq!(switches, 0, "the choice flickered");
        let offers = [rock("a", 0.30, 10.0), rock("b", 0.10, 0.0)];
        let had = t.choice(1).unwrap().id.clone();
        t.update([None, Some(&offers)], [None, None]);
        assert_eq!(t.choice(1).unwrap().id, "b", "had {had}");
        let offers = [rock("b", CONE_REACH * 1.1, 5.0)];
        t.update([None, Some(&offers)], [None, None]);
        assert_eq!(t.choice(1).map(|c| c.id.as_str()), Some("b"));
        let mut fresh = GrabTargets::default();
        fresh.update([None, Some(&offers)], [None, None]);
        assert!(fresh.choice(1).is_none());
    }

    #[test]
    fn neither_hand_is_offered_what_a_hand_holds_and_two_hands_share_out() {
        let mut t = GrabTargets::default();
        let offers = [rock("a", 0.05, 0.0), rock("b", 0.2, 5.0)];
        t.update([Some(&offers), Some(&offers)], [Some("a"), None]);
        assert!(t.choice(0).is_none(), "a full hand offers nothing");
        assert_eq!(t.choice(1).unwrap().id, "b");
        let left = [rock("a", 0.10, 0.0), rock("b", 0.25, 5.0)];
        let right = [rock("a", 0.05, 0.0), rock("b", 0.30, 5.0)];
        t.update([Some(&left), Some(&right)], [None, None]);
        assert_eq!(t.choice(1).unwrap().id, "a");
        assert_eq!(t.choice(0).unwrap().id, "b");
        let one = [rock("a", 0.05, 0.0)];
        t.update([Some(&one), Some(&one)], [None, None]);
        assert!(t.choice(0).is_none() ^ t.choice(1).is_none());
    }

    #[test]
    fn a_handle_is_taken_only_within_its_own_reach() {
        let handle = |d: f32| Offer { id: "door".into(), glow: "lever".into(), kind: TakeKind::Handle, distance: d, angle: 1.0, reach: Some(0.14) };
        assert!(cost(&handle(0.1), false).is_some());
        assert!(cost(&handle(0.2), false).is_none());
        let mut t = GrabTargets::default();
        let offers = [handle(0.1), rock("a", 0.12, 0.0)];
        t.update([Some(&offers), None], [None, None]);
        let c = t.choice(0).unwrap();
        assert_eq!((c.id.as_str(), c.glow.as_str(), c.kind), ("door", "lever", TakeKind::Handle));
    }

    #[test]
    fn the_glow_comes_up_breathes_goes_out_and_then_costs_nothing() {
        let mut g = GlowFade::default();
        assert!(g.step(1.0 / 72.0, 0.0, &[]).is_empty(), "nothing lit: nothing written");
        let mut t = 0.0;
        let mut level = 0.0;
        for _ in 0..36 {
            t += 1.0 / 72.0;
            let out = g.step(1.0 / 72.0, t, &["rock"]);
            assert_eq!(out.len(), 1);
            level = out[0].1;
        }
        assert!(level > GLOW_LOW * 0.95 && level <= 1.0, "lit at {level}");
        let mut frames = 0;
        loop {
            t += 1.0 / 72.0;
            let out = g.step(1.0 / 72.0, t, &[]);
            frames += 1;
            if out.iter().any(|(_, l)| *l == 0.0) {
                break;
            }
            assert!(frames < 72, "the glow did not go out within a second");
        }
        assert!(g.is_idle());
        assert!(g.step(1.0 / 72.0, t, &[]).is_empty(), "out: nothing written");
    }

    #[test]
    fn the_angle_is_to_the_things_edge() {
        assert_eq!(angle_to_sphere(Vec3::ZERO, Vec3::NEG_Z, Vec3::new(0.1, 0.0, -0.5), 0.2), 0.0);
        let a = angle_to_sphere(Vec3::ZERO, Vec3::NEG_Z, Vec3::new(0.5, 0.0, -0.5), 0.1);
        assert!((a.to_degrees() - (45.0 - (0.1f32 / 0.5f32.hypot(0.5)).asin().to_degrees())).abs() < 0.01);
    }
}
