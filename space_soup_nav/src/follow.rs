//! Following a path: what one agent should steer at this tick, and when to plan again.
//!
//! Engine-agnostic: call [`PathFollower::update`] each tick with the agent's position and its goal
//! (a player, a noise, a doorway); steer the agent at the point returned, however the game moves
//! it (an animation, a character controller). Positions are millimetres, like the rest of the
//! crate.
//!
//! It plans again when the goal has moved from where the path was aimed, when a door or link on
//! the path changed (the world's version moved and the path uses it), when the agent has been
//! pushed off its path, and when a path ran out short of a goal that has since become reachable.

use glam::DVec3;

use crate::path::{find_path_guided, Path, Waypoint};
use crate::world::{LinkId, NavWorld, RoomGraph};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Steer {
    /// Head for this point.
    Walk(DVec3),
    /// At the start of a typed link (vent, ladder): traverse it to `to`, then keep calling.
    TakeLink { link: LinkId, to: DVec3 },
    /// At the goal.
    Arrived,
    /// No way to the goal from here: the agent is off the navmesh, or the goal is.
    NoPath,
}

#[derive(Debug, Clone)]
pub struct FollowSettings {
    /// A waypoint is reached within this, mm.
    pub reach_mm: f64,
    /// The goal is reached within this, mm.
    pub arrive_mm: f64,
    /// Plan again when the goal has moved this far from where the path was aimed, mm.
    pub goal_moved_mm: f64,
    /// Plan again when the agent is this far off its current leg, mm.
    pub off_path_mm: f64,
    /// Never plan more often than this, seconds (a goal moving every tick).
    pub min_replan_s: f64,
    /// How far to search for the navmesh around a position, mm.
    pub half_extents: DVec3,
}

impl Default for FollowSettings {
    fn default() -> Self {
        FollowSettings {
            reach_mm: 300.0,
            arrive_mm: 500.0,
            goal_moved_mm: 1000.0,
            off_path_mm: 1000.0,
            min_replan_s: 0.25,
            half_extents: DVec3::new(500.0, 2000.0, 500.0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PathFollower {
    pub settings: FollowSettings,
    path: Option<Path>,
    /// The waypoint being walked to.
    next: usize,
    since_plan: f64,
    /// No-progress watch: the target being steered at, the nearest the agent has got to it, and
    /// for how long it has not got nearer.
    watch: Option<(DVec3, f64, f64)>,
    /// Plans made, for tuning.
    pub plans: usize,
}

fn flat(a: DVec3, b: DVec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

impl PathFollower {
    pub fn new(settings: FollowSettings) -> PathFollower {
        PathFollower { settings, ..Default::default() }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_ref()
    }

    /// Forget the current path (the agent was teleported, or changed its mind).
    pub fn reset(&mut self) {
        self.path = None;
    }

    fn needs_plan(&self, world: &NavWorld, pos: DVec3, goal: DVec3) -> bool {
        let Some(p) = &self.path else { return true };
        if self.since_plan < self.settings.min_replan_s {
            return false;
        }
        let aimed = p.waypoints.last().unwrap().pos();
        if flat(aimed, goal) > self.settings.goal_moved_mm {
            return true;
        }
        if p.version != world.version() && p.links().any(|l| !world.links[l as usize].enabled) {
            return true;
        }
        if !p.complete {
            return true;
        }
        if self.next > 0 && self.next < p.waypoints.len() {
            let (a, b) = (p.waypoints[self.next - 1].pos(), p.waypoints[self.next].pos());
            let (d, _) = crate::query::dist_pt_seg_sqr_2d(pos, a, b);
            if d.sqrt() > self.settings.off_path_mm {
                return true;
            }
        }
        false
    }

    /// What to steer at this tick.
    pub fn update(&mut self, world: &NavWorld, graph: &RoomGraph, pos: DVec3, goal: DVec3, dt: f64) -> Steer {
        self.since_plan += dt;
        if flat(pos, goal) <= self.settings.arrive_mm && (pos.y - goal.y).abs() < 1000.0 {
            return Steer::Arrived;
        }
        if self.needs_plan(world, pos, goal) {
            self.path = find_path_guided(world, graph, pos, goal, self.settings.half_extents);
            self.next = 1;
            self.since_plan = 0.0;
            self.plans += 1;
        }
        let line_clear = |a: DVec3, b: DVec3| world.nearest_poly(a, self.settings.half_extents).is_some_and(|n| world.walkable_line(n.poly, n.point, b));
        let Some(p) = &self.path else { return Steer::NoPath };
        let aim = |k: usize| -> DVec3 {
            let w = p.waypoints[k];
            if let (Waypoint::Walk(c), Some(prev), Some(next)) = (w, k.checked_sub(1).and_then(|j| p.waypoints.get(j)), p.waypoints.get(k + 1)) {
                let d1 = DVec3::new(c.x - prev.pos().x, 0.0, c.z - prev.pos().z).normalize_or_zero();
                let d2 = DVec3::new(next.pos().x - c.x, 0.0, next.pos().z - c.z).normalize_or_zero();
                let away = (d1 - d2).normalize_or_zero();
                if away != DVec3::ZERO {
                    let inside = c + away * 100.0;
                    if world.nearest_poly(inside, DVec3::new(20.0, 500.0, 20.0)).is_some_and(|n| n.over_poly) {
                        return inside;
                    }
                }
            }
            w.pos()
        };
        while self.next < p.waypoints.len() {
            let w = p.waypoints[self.next];
            let target = aim(self.next);
            let close = flat(pos, w.pos()) <= self.settings.reach_mm || flat(pos, target) <= self.settings.reach_mm;
            let at = flat(pos, w.pos()) <= 30.0 || flat(pos, target) <= 30.0;
            let passed = self.next + 1 < p.waypoints.len() && matches!(w, Waypoint::Walk(_)) && {
                let leg = w.pos() - p.waypoints[self.next - 1].pos();
                leg.length_squared() > 0.0 && (pos - w.pos()).dot(leg) >= 0.0
            };
            if let Waypoint::LinkStart(_, link) = w {
                if close {
                    let to = p.waypoints.get(self.next + 1).map_or(w.pos(), super::path::Waypoint::pos);
                    self.next += 2;
                    return Steer::TakeLink { link, to };
                }
                break;
            }
            let following = p.waypoints.get(self.next + 1).map_or(goal, super::path::Waypoint::pos);
            if (at || close || passed) && line_clear(pos, following) {
                self.next += 1;
                continue;
            }
            break;
        }
        if self.next < p.waypoints.len() {
            let target = aim(self.next);
            let d = flat(pos, target);
            let stalled = match self.watch {
                Some((t, best, since)) if flat(t, target) < 1.0 => {
                    if d < best - 50.0 {
                        self.watch = Some((target, d, 0.0));
                        false
                    } else {
                        self.watch = Some((target, best, since + dt));
                        since + dt > 1.0
                    }
                }
                _ => {
                    self.watch = Some((target, d, 0.0));
                    false
                }
            };
            if stalled {
                self.watch = None;
                self.path = None;
                self.since_plan = self.settings.min_replan_s;
                return self.update(world, graph, pos, goal, 0.0);
            }
            return Steer::Walk(target);
        }
        if !p.complete {
            return Steer::NoPath;
        }
        if line_clear(pos, goal) {
            return Steer::Walk(goal);
        }
        self.path = find_path_guided(world, graph, pos, goal, self.settings.half_extents);
        self.next = 1;
        self.since_plan = 0.0;
        self.plans += 1;
        match self.path.as_ref().and_then(|p| p.waypoints.get(1)) {
            Some(w) => Steer::Walk(w.pos()),
            None => Steer::NoPath,
        }
    }
}
