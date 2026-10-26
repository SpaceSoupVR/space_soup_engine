//! GROUND HITS: what struck the level's ground hard, told to every client so
//! each raises the same mud splash (`space_soup::renderer::effects::Splash::mud`)
//! where the ground is mud, and nothing where it is not. The server knows
//! the true speeds and masses, so the headsets do not guess them.
//!
//! Three things hit the ground:
//! - a BODY landing on it, seen by physics after each step
//!   ([`GroundState::see`]): its bottom coming down onto the terrain from
//!   above, falling faster than [`LAND_SPEED`], at the speed it had in the
//!   air (the step that finds it on the ground has already stopped it);
//! - a SHOT, and
//! - a BLAST, each told by the code that fires or sets it off
//!   (`GameRuntime::ground_hit`).
//!
//! Which ground is mud is the client's to know (its splat and material);
//! the server only says where and how hard. See docs/ground-hits.md in
//! space_soup_protocol.

use glam::Vec3;

/// The slowest landing that is told of, m/s: a drop of about 5 cm.
pub const LAND_SPEED: f32 = 1.0;
/// A body's bottom this far over the ground is in the air, metres.
pub const ALOFT: f32 = 0.04;
/// How long a hit is told of, seconds: every snapshot carries it this long,
/// so a client that misses one still hears of it.
pub const GROUND_HIT_KEPT: f32 = 0.5;
/// The most told of at once; the least energetic go first.
pub const MAX_GROUND_HITS_TOLD: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundHitKind {
    Landing,
    Shot,
    Blast,
}

impl GroundHitKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Landing => "landing",
            Self::Shot => "shot",
            Self::Blast => "blast",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroundHit {
    /// The body's object id; empty for a shot or a blast.
    pub id: String,
    pub kind: GroundHitKind,
    /// Where it met the ground, world.
    pub position: Vec3,
    /// How it was moving as it did, m/s (zero for a blast).
    pub velocity: Vec3,
    /// The radius of a disc as big as its footprint, metres (a shot's: half
    /// its calibre).
    pub radius: f32,
    pub mass: f32,
    /// A blast's crater radius, metres; 0 otherwise.
    pub blast_radius: f32,
}

impl GroundHit {
    /// What decides which are told of when there are too many: a body's or
    /// a shot's energy, and a blast's crater by the energy that digs it
    /// (as big as a 30 kg body at 10 m/s per metre of radius cubed).
    pub fn energy(&self) -> f32 {
        if self.kind == GroundHitKind::Blast {
            1500.0 * self.blast_radius.powi(3)
        } else {
            0.5 * self.mass.max(0.0) * self.velocity.length_squared()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecentGroundHit {
    pub serial: u64,
    /// Seconds since it hit, as of the simulation's last step.
    pub age: f32,
    pub hit: GroundHit,
}

/// One body's landings: whether it was in the air and how fast it was
/// falling there.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GroundState {
    aloft: Option<bool>,
    falling: Vec3,
}

impl GroundState {
    /// The body as it is now: its `centre`, the half extents of the box round
    /// it along the world's axes, how it moves and its mass; `ground` the
    /// terrain's height under it (none off the terrain). A landing when it
    /// was in the air and is now down on the ground, falling fast enough.
    pub fn see(&mut self, centre: Vec3, half: Vec3, velocity: Vec3, mass: f32, ground: Option<f32>) -> Option<GroundHit> {
        let Some(g) = ground else {
            self.aloft = None;
            return None;
        };
        let aloft = centre.y - half.y.abs() > g + ALOFT;
        let was = self.aloft.replace(aloft);
        if aloft {
            self.falling = velocity;
            return None;
        }
        let falling = if velocity.y < self.falling.y { velocity } else { self.falling };
        self.falling = Vec3::ZERO;
        (was == Some(true) && -falling.y >= LAND_SPEED).then(|| GroundHit {
            id: String::new(),
            kind: GroundHitKind::Landing,
            position: Vec3::new(centre.x, g, centre.z),
            velocity: falling,
            radius: (half.x.abs() * half.z.abs()).sqrt().clamp(0.01, 2.0),
            mass,
            blast_radius: 0.0,
        })
    }
}

/// Keep the `cap` most energetic.
pub fn keep_most_energetic(hits: &mut Vec<GroundHit>, cap: usize) {
    if hits.len() > cap {
        hits.sort_by(|a, b| b.energy().total_cmp(&a.energy()));
        hits.truncate(cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_landing_is_told_once_at_the_speed_it_fell() {
        let mut s = GroundState::default();
        let half = Vec3::splat(0.25);
        assert!(s.see(Vec3::new(0.0, 2.0, 0.0), half, Vec3::new(0.0, -5.0, 0.0), 30.0, Some(0.0)).is_none());
        assert!(s.see(Vec3::new(0.0, 1.0, 0.0), half, Vec3::new(0.0, -6.0, 0.0), 30.0, Some(0.0)).is_none());
        let hit = s.see(Vec3::new(0.0, 0.25, 0.0), half, Vec3::new(0.0, -0.2, 0.0), 30.0, Some(0.0)).expect("it landed");
        assert_eq!(hit.kind, GroundHitKind::Landing);
        assert!((hit.velocity.y + 6.0).abs() < 1e-3 && hit.position.y == 0.0 && hit.mass == 30.0);
        assert!(s.see(Vec3::new(0.0, 0.25, 0.0), half, Vec3::ZERO, 30.0, Some(0.0)).is_none(), "resting");
    }

    #[test]
    fn a_body_set_down_slowly_or_off_the_ground_is_not_told() {
        let mut s = GroundState::default();
        let half = Vec3::splat(0.25);
        s.see(Vec3::new(0.0, 0.4, 0.0), half, Vec3::new(0.0, -0.3, 0.0), 30.0, Some(0.0));
        assert!(s.see(Vec3::new(0.0, 0.25, 0.0), half, Vec3::ZERO, 30.0, Some(0.0)).is_none(), "set down");
        let mut s = GroundState::default();
        s.see(Vec3::new(0.0, 2.0, 0.0), half, Vec3::new(0.0, -6.0, 0.0), 30.0, None);
        assert!(s.see(Vec3::new(0.0, 0.25, 0.0), half, Vec3::new(0.0, -6.0, 0.0), 30.0, None).is_none(), "no ground");
    }

    #[test]
    fn the_most_energetic_are_kept() {
        let hit = |m: f32| GroundHit { id: String::new(), kind: GroundHitKind::Landing, position: Vec3::ZERO, velocity: Vec3::new(0.0, -5.0, 0.0), radius: 0.2, mass: m, blast_radius: 0.0 };
        let mut v = vec![hit(1.0), hit(30.0), hit(5.0)];
        keep_most_energetic(&mut v, 2);
        assert_eq!(v.iter().map(|h| h.mass).collect::<Vec<_>>(), [30.0, 5.0]);
    }
}
