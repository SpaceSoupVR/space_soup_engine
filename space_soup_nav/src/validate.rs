//! Checking bakes and levels before they reach the game, and reporting what an agent can reach.
//!
//! - [`check_piece`]: one baked piece. Is the asset sound; can this agent reach each doorway; is
//!   there floor it can never get to (a catwalk with no ladder, half a room behind a vent); do its
//!   vents and ladders land on floor.
//! - [`check_kit`]: the pieces together. Do doorways of one size class come out the same width
//!   (two pieces' doorways only join as wide as the narrower one).
//! - [`check_level`]: an assembled level. What share of it an agent can reach from the start, which
//!   joins it cannot pass, which doors cut the level in two, where doorways were welded with a gap.
//!
//! Issues carry a severity and, where they have one, a place, so a debug view can mark them
//! ([`crate::debug::DebugDraw::add_problem`]).

use std::fmt::Write as _;
use std::sync::Arc;

use glam::{DVec3, IVec3};

use crate::asset::{NavPieceAsset, Turn};
use crate::world::{JoinIssue, NavWorld, PolyRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something is broken and will misbehave in the game.
    Error,
    /// Allowed, but worth a look: often a design choice, sometimes a mistake.
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    /// The piece (or level) it is about.
    pub subject: String,
    pub message: String,
    /// Where, in the piece's own space (or the world for a level), if it has a place.
    pub at: Option<DVec3>,
}

fn issue(severity: Severity, subject: &str, message: String, at: Option<DVec3>) -> Issue {
    Issue { severity, subject: subject.to_string(), message, at }
}

/// Checks one baked piece.
pub fn check_piece(asset: &Arc<NavPieceAsset>) -> Vec<Issue> {
    let name = asset.name.as_str();
    let mut out = Vec::new();
    if let Err(e) = asset.validate() {
        out.push(issue(Severity::Error, name, format!("the asset is malformed: {e}"), None));
        return out;
    }
    if asset.polys.is_empty() {
        out.push(issue(Severity::Info, name, "no floor at all (a cap or blocker)".into(), None));
        return out;
    }
    let r = asset.agent.radius_mm as f64 / 1000.0;
    for s in &asset.sockets {
        if s.edges.is_empty() {
            out.push(issue(
                Severity::Warning,
                name,
                format!("doorway {} cannot be passed by a {r:.2} m radius agent (too narrow or too low)", s.name),
                Some(s.a.as_dvec3()),
            ));
        }
    }

    let mut w = NavWorld::new();
    w.add_island(asset.clone(), Turn(0), IVec3::ZERO);
    for &(_, li, which) in &w.unattached_links {
        let l = &asset.links[li];
        let at = if which == "end" { (l.end[0] + l.end[1]).as_dvec3() / 2.0 } else { (l.start[0] + l.start[1]).as_dvec3() / 2.0 };
        out.push(issue(Severity::Error, name, format!("{:?} link {li}: its {which} is not on any floor", l.kind), Some(at)));
    }

    // Floor that no doorway leads to. Typed links count as ways in.
    if !asset.sockets.is_empty() {
        let doorway_polys: Vec<PolyRef> =
            asset.sockets.iter().flat_map(|s| s.edges.iter().map(|&(p, _)| PolyRef { island: 0, poly: p })).collect();
        let reached: Vec<u32> = doorway_polys.iter().map(|&p| w.component(p)).collect();
        // Group the unreached polygons by area so each cut-off part is reported once.
        let mut parts: Vec<(u32, f64, DVec3, f64)> = Vec::new();
        for pi in 0..asset.polys.len() {
            let p = PolyRef { island: 0, poly: pi as u16 };
            let c = w.component(p);
            if reached.contains(&c) {
                continue;
            }
            let tris = w.floor_triangles(p);
            let area: f64 = tris.iter().map(|t| ((t[1] - t[0]).cross(t[2] - t[0])).y.abs() / 2.0).sum();
            let centre = tris.iter().flatten().copied().sum::<DVec3>() / (tris.len() * 3).max(1) as f64;
            match parts.iter_mut().find(|x| x.0 == c) {
                Some(x) => {
                    x.2 = (x.2 * x.1 + centre * area) / (x.1 + area).max(1e-9);
                    x.1 += area;
                    x.3 = x.3.max(centre.y);
                }
                None => parts.push((c, area, centre, centre.y)),
            }
        }
        for (_, area, centre, _) in parts {
            let m2 = area / 1e6;
            if m2 < 0.05 {
                continue;
            }
            out.push(issue(
                Severity::Warning,
                name,
                format!(
                    "{m2:.1} m² of floor at ({:.1}, {:.1}, {:.1}) m cannot be reached from any doorway (needs a ladder, vent or ramp link?)",
                    centre.x / 1000.0,
                    centre.y / 1000.0,
                    centre.z / 1000.0
                ),
                Some(centre),
            ));
        }
    }

    let tiny = (0..asset.polys.len())
        .filter(|&p| {
            let tris = w.floor_triangles(PolyRef { island: 0, poly: p as u16 });
            tris.iter().map(|t| ((t[1] - t[0]).cross(t[2] - t[0])).y.abs() / 2.0).sum::<f64>() < 10_000.0
        })
        .count();
    if tiny > 0 {
        out.push(issue(Severity::Info, name, format!("{tiny} polygon(s) under 0.01 m²"), None));
    }
    out
}

/// Checks a kit's pieces against each other (all baked for one agent class).
pub fn check_kit(assets: &[Arc<NavPieceAsset>]) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut classes: Vec<&str> = assets.iter().flat_map(|a| a.sockets.iter().map(|s| s.size_class.as_str())).collect();
    classes.sort_unstable();
    classes.dedup();
    for class in classes {
        // Walkable width of each passable doorway of this class.
        let mut widths: Vec<(f64, String)> = assets
            .iter()
            .flat_map(|a| a.sockets.iter().filter(|s| s.size_class == class && !s.edges.is_empty()).map(move |s| ((s.b - s.a).as_dvec3().length() / 1000.0, format!("{}.{}", a.name, s.name))))
            .collect();
        if widths.len() < 2 {
            continue;
        }
        widths.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (lo, hi) = (widths[0].0, widths[widths.len() - 1].0);
        if hi - lo > 0.15 {
            let narrow: Vec<String> = widths.iter().take_while(|w| w.0 < lo + 0.05).map(|w| format!("{} ({:.2} m)", w.1, w.0)).collect();
            out.push(issue(
                Severity::Warning,
                &format!("size class {class}"),
                format!(
                    "walkable doorway widths range {lo:.2}-{hi:.2} m; a join is as wide as its narrower side. Narrowest: {}",
                    narrow.join(", ")
                ),
                None,
            ));
        }
    }
    out
}

/// What an agent can do in an assembled level.
#[derive(Debug, Clone, Default)]
pub struct LevelReport {
    /// Pieces with floor, and how many of them the agent can reach from the start.
    pub rooms: usize,
    pub reachable_rooms: usize,
    /// Pieces it cannot reach: (island, where).
    pub unreachable: Vec<(u32, DVec3)>,
    /// Joins between pieces that both have floor, which this agent cannot pass.
    pub impassable_joins: Vec<(u32, DVec3)>,
    /// Doorways meeting other doorways that the level did not join.
    pub missed_joins: usize,
    /// Doorways joined to nothing.
    pub open_doorways: usize,
    pub doors: usize,
    /// Doors whose closing cuts the level in two.
    pub cutting_doors: Vec<u32>,
    /// Joins welded with the planes apart by more than 1 mm, and the widest gap, mm.
    pub welded: usize,
    pub widest_weld_mm: i32,
}

/// Reports on `world` from the start polygon `start`. Doors are left as they were found.
pub fn check_level(world: &mut NavWorld, start: PolyRef) -> LevelReport {
    let mut r = LevelReport::default();
    let start_area = world.component(start);
    r.rooms = world.islands.iter().filter(|i| !i.asset.polys.is_empty()).count();
    for ii in 0..world.islands.len() {
        let n = world.islands[ii].asset.polys.len();
        if n == 0 {
            continue;
        }
        let reach = (0..n).any(|p| world.component(PolyRef { island: ii as u32, poly: p as u16 }) == start_area);
        if reach {
            r.reachable_rooms += 1;
        } else {
            let isl = &world.islands[ii];
            r.unreachable.push((ii as u32, ((isl.min + isl.max) / 2).as_dvec3()));
        }
    }
    let has_floor = |w: &NavWorld, i: u32| !w.islands[i as usize].asset.polys.is_empty();
    for issue in world.check_joins() {
        match issue {
            JoinIssue::Impassable(c) => {
                let conn = &world.connections[c as usize];
                if has_floor(world, conn.a.island) && has_floor(world, conn.b.island) {
                    r.impassable_joins.push((c, conn.midpoint.as_dvec3()));
                }
            }
            JoinIssue::MissedJoin(..) => r.missed_joins += 1,
            JoinIssue::Unjoined(_) => r.open_doorways += 1,
        }
    }
    for c in &world.connections {
        if c.gap_mm.abs() > 1 {
            r.welded += 1;
            r.widest_weld_mm = r.widest_weld_mm.max(c.gap_mm.abs());
        }
    }
    // Which doors cut the level: close each in turn and see if its two sides part.
    r.doors = world.gates.len();
    for g in 0..world.gates.len() as u32 {
        let Some(&l) = world.gates[g as usize].links.first() else { continue };
        if !world.gates[g as usize].open {
            continue;
        }
        let (a, b) = (world.links[l as usize].from, world.links[l as usize].to);
        world.set_gate(g, false);
        if !world.same_component(a, b) {
            r.cutting_doors.push(g);
        }
        world.set_gate(g, true);
    }
    // Opening and closing doors to test them is not a change anyone needs to hear about.
    world.drain_events();
    r
}

impl LevelReport {
    pub fn to_markdown(&self, title: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "### {title}\n");
        let pct = 100.0 * self.reachable_rooms as f64 / self.rooms.max(1) as f64;
        let _ = writeln!(s, "- Reaches **{} of {} rooms** ({pct:.0}%) from the start.", self.reachable_rooms, self.rooms);
        if !self.unreachable.is_empty() {
            let list: Vec<String> = self.unreachable.iter().take(8).map(|(i, at)| format!("#{i} at ({:.0}, {:.0}) m", at.x / 1000.0, at.z / 1000.0)).collect();
            let _ = writeln!(s, "- Cut off: {}{}", list.join(", "), if self.unreachable.len() > 8 { ", ..." } else { "" });
        }
        let _ = writeln!(s, "- Joins it cannot pass: {}.", self.impassable_joins.len());
        let _ = writeln!(s, "- Doors: {}, of which {} cut the level in two when shut.", self.doors, self.cutting_doors.len());
        let _ = writeln!(s, "- Doorways joined to nothing: {}; doorways that meet but were not joined: {}.", self.open_doorways, self.missed_joins);
        let _ = writeln!(s, "- Welded joins (planes apart > 1 mm): {}, widest {} mm.", self.welded, self.widest_weld_mm);
        s
    }
}
