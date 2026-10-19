//! Many agents at once: each tick, a velocity for every agent that follows its path without
//! walking into the others, the players or the walls.
//!
//! The game owns movement: [`Crowd::update`] computes a desired velocity per agent, the game moves
//! the agent however it moves things (animation, character controller, physics), then reports
//! where the agent really is with [`Crowd::set_agent`]. Millimetres and millimetres per second, on
//! the floor plane.
//!
//! Three layers, after the design research (DetourCrowd's sampled avoidance jitters and sees only
//! six neighbours; ORCA alone deadlocks in doorways; shipped games settle crowding above the
//! avoidance layer):
//!
//! 1. **Where to go** — the path follower, plus two rules that decide who goes first:
//!    - **Narrow places take turns.** Wherever two agents cannot pass side by side (a doorway, a
//!      gap: found on the navmesh itself), agents queue first come, first served. Agents going the
//!      same way flow through together; the other side waits beside the opening, out of the way.
//!    - **Attack slots.** A ring of places around each player; the nearest chasers take them and
//!      the rest hold just outside instead of piling in.
//! 2. **How to get there without collisions** — ORCA (optimal reciprocal collision avoidance, van
//!    den Berg et al.; the linear programs follow RVO2, Apache-2.0, Copyright 2008 University of
//!    North Carolina at Chapel Hill). Players never yield, so agents take all of the avoidance
//!    for them; an agent going through a narrow place outranks one waiting. Walls are constraints
//!    too: the navmesh's edges near the agent.
//! 3. **The game moves it** and reports back.

use std::collections::{BTreeMap, VecDeque};

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::asset::EdgeLink;
use crate::follow::{FollowSettings, PathFollower, Steer};
use crate::query::dist_pt_seg_sqr_2d;
use crate::world::{LinkKind, NavWorld, PolyRef, RoomGraph};

pub type AgentId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    Idle,
    Point(DVec3),
    /// Chase a player, by index into [`Crowd::players`].
    Player(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AgentParams {
    pub radius_mm: f64,
    pub max_speed_mm_s: f64,
    /// Higher goes first: a lower one yields to it entirely.
    pub priority: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Moving,
    Arrived,
    /// Queued for a narrow place.
    Waiting,
    /// In an attack slot around its player.
    Attacking,
    /// Chasing, but every slot round its player is taken: holding just outside.
    Holding,
    NoPath,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub pos: DVec3,
    pub vel: DVec3,
    pub params: AgentParams,
    pub goal: Goal,
    pub state: AgentState,
    /// The velocity [`Crowd::update`] wants it to move at.
    pub desired: DVec3,
    /// What it is steering at this tick (a path corner, a slot, a queue spot), for debug views.
    pub target: Option<DVec3>,
    follower: PathFollower,
    /// The narrow place this agent is going through next and the side it comes from.
    plan: Option<(u32, i8)>,
    token: Option<u32>,
}

impl Agent {
    /// The path it is following, for debug views.
    pub fn path(&self) -> Option<&crate::path::Path> {
        self.follower.path()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub pos: DVec3,
    pub vel: DVec3,
    pub radius_mm: f64,
}

#[derive(Debug, Clone)]
pub struct CrowdSettings {
    /// How far ahead other agents are avoided, seconds.
    pub time_horizon_s: f64,
    /// How far ahead walls are avoided, seconds.
    pub wall_time_horizon_s: f64,
    /// Agents farther than this are ignored, mm.
    pub neighbour_mm: f64,
    /// Space between agents round a player and in a queue, mm.
    pub gap_mm: f64,
    /// Within this of its player (and in sight of it) a chaser takes or waits for a slot, mm.
    pub engage_mm: f64,
    /// Within this of a narrow place an agent asks for its turn, mm.
    pub approach_mm: f64,
    /// Below this speed an agent slows down to arrive, mm.
    pub arrive_slow_mm: f64,
}

impl Default for CrowdSettings {
    fn default() -> Self {
        CrowdSettings {
            time_horizon_s: 2.0,
            wall_time_horizon_s: 0.4,
            neighbour_mm: 5000.0,
            gap_mm: 100.0,
            engage_mm: 4000.0,
            approach_mm: 2000.0,
            arrive_slow_mm: 600.0,
        }
    }
}

/// A narrow place: one or more polygon edges, close together, where two agents cannot pass.
#[derive(Debug, Clone)]
pub struct Choke {
    pub centre: DVec3,
    /// Horizontal, across the opening: agents on the positive side are on "side +1".
    pub normal: DVec2,
    /// Along the opening.
    pub along: DVec2,
}

#[derive(Debug, Clone, Default)]
struct Token {
    /// Which side the agents now going through came from (0: nobody inside).
    side: i8,
    inside: Vec<AgentId>,
    queue: VecDeque<(AgentId, i8)>,
}

fn flat(v: DVec3) -> DVec2 {
    DVec2::new(v.x, v.z)
}

fn up(v: DVec2, y: f64) -> DVec3 {
    DVec3::new(v.x, y, v.y)
}

/// Which narrow place each polygon edge belongs to: (island, polygon, edge) to choke index.
pub type ChokeMap = BTreeMap<(u32, u16, u8), u32>;

/// The narrow places of a world for agents of `radius_mm`: polygon edges (and doorway portals)
/// where the walkable width across is under one more agent's width, grouped when within a metre.
pub fn find_chokes(world: &NavWorld, radius_mm: f64, gap_mm: f64) -> (Vec<Choke>, ChokeMap) {
    let narrow = 2.0 * radius_mm + gap_mm;
    let mut edges: Vec<((u32, u16, u8), DVec3, DVec2)> = Vec::new();
    for (ii, isl) in world.islands.iter().enumerate() {
        for (pi, poly) in isl.asset.polys.iter().enumerate() {
            let p = PolyRef { island: ii as u32, poly: pi as u16 };
            let v = world.poly_dverts(p);
            let n = v.len();
            let centroid = v.iter().copied().sum::<DVec3>() / n as f64;
            for (e, link) in poly.edges.iter().enumerate() {
                let (a, b) = (v[e], v[(e + 1) % n]);
                let width = match *link {
                    EdgeLink::Internal(o) if (o as usize) < pi => continue,
                    EdgeLink::Internal(_) => {
                        let mid = (a + b) / 2.0;
                        let dir = (b - a).normalize_or_zero();
                        let start = mid + (centroid - mid).normalize_or_zero();
                        let reach = 3000.0;
                        let span = |d: DVec3| {
                            let hit = world.raycast(p, start, start + d * reach);
                            if hit.t == f64::MAX { reach } else { hit.t * reach }
                        };
                        span(dir) + span(-dir)
                    }
                    EdgeLink::Socket(_) => {
                        let w = isl.links_from[pi]
                            .iter()
                            .map(|&l| &world.links[l as usize])
                            .filter(|l| l.kind == LinkKind::Portal && l.from_edge == Some(e as u8))
                            .map(|l| (l.from_segment[0] - l.from_segment[1]).as_dvec3().length())
                            .fold(0.0, f64::max);
                        if w == 0.0 {
                            continue;
                        }
                        w
                    }
                    EdgeLink::Border => continue,
                };
                if width < narrow {
                    let d = flat(b - a).normalize_or_zero();
                    edges.push(((ii as u32, pi as u16, e as u8), (a + b) / 2.0, d));
                }
            }
        }
    }
    let mut group: Vec<usize> = (0..edges.len()).collect();
    fn root(g: &mut [usize], mut x: usize) -> usize {
        while g[x] != x {
            g[x] = g[g[x]];
            x = g[x];
        }
        x
    }
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            if (edges[i].1 - edges[j].1).length() < 1000.0 {
                let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                if ri != rj {
                    group[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut chokes = Vec::new();
    let mut id_of_root: BTreeMap<usize, u32> = BTreeMap::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut map = BTreeMap::new();
    for (i, edge) in edges.iter().enumerate() {
        let r = root(&mut group, i);
        let id = *id_of_root.entry(r).or_insert_with(|| {
            members.push(Vec::new());
            (members.len() - 1) as u32
        });
        members[id as usize].push(i);
        map.insert(edge.0, id);
    }
    for m in &members {
        let centre = m.iter().map(|&i| edges[i].1).sum::<DVec3>() / m.len() as f64;
        let along = edges[m[0]].2;
        chokes.push(Choke { centre, normal: DVec2::new(-along.y, along.x), along });
    }
    (chokes, map)
}

#[derive(Debug, Clone, Copy)]
struct Line {
    point: DVec2,
    direction: DVec2,
}

/// Leans a preferred velocity a little to the agent's right. Two agents meeting exactly head on
/// otherwise have no reason to pick a side and ORCA slows them to a stop face to face (its known
/// symmetric deadlock); with everyone keeping right they always pass the same way. Deterministic,
/// unlike the random nudges RVO2's examples use.
fn keep_right(v: DVec2) -> DVec2 {
    v + DVec2::new(-v.y, v.x) * 0.05
}

fn det(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// RVO2's `linearProgram1`: the best velocity on line `i` within the speed circle and lines
/// before `i`.
fn lp1(lines: &[Line], i: usize, radius: f64, opt: DVec2, direction_opt: bool, result: &mut DVec2) -> bool {
    let dot = lines[i].point.dot(lines[i].direction);
    let disc = dot * dot + radius * radius - lines[i].point.length_squared();
    if disc < 0.0 {
        return false;
    }
    let sq = disc.sqrt();
    let (mut tl, mut tr) = (-dot - sq, -dot + sq);
    for j in 0..i {
        let denom = det(lines[i].direction, lines[j].direction);
        let numer = det(lines[j].direction, lines[i].point - lines[j].point);
        if denom.abs() <= 1e-9 {
            if numer < 0.0 {
                return false;
            }
            continue;
        }
        let t = numer / denom;
        if denom >= 0.0 {
            tr = tr.min(t);
        } else {
            tl = tl.max(t);
        }
        if tl > tr {
            return false;
        }
    }
    *result = if direction_opt {
        if opt.dot(lines[i].direction) > 0.0 { lines[i].point + tr * lines[i].direction } else { lines[i].point + tl * lines[i].direction }
    } else {
        let t = lines[i].direction.dot(opt - lines[i].point);
        lines[i].point + t.clamp(tl, tr) * lines[i].direction
    };
    true
}

/// RVO2's `linearProgram2`: the velocity nearest `opt` inside the speed circle and every line;
/// returns how many lines it satisfied before failing.
fn lp2(lines: &[Line], radius: f64, opt: DVec2, direction_opt: bool, result: &mut DVec2) -> usize {
    *result = if direction_opt {
        opt * radius
    } else if opt.length_squared() > radius * radius {
        opt.normalize() * radius
    } else {
        opt
    };
    for i in 0..lines.len() {
        if det(lines[i].direction, lines[i].point - *result) > 0.0 {
            let temp = *result;
            if !lp1(lines, i, radius, opt, direction_opt, result) {
                *result = temp;
                return i;
            }
        }
    }
    lines.len()
}

/// RVO2's `linearProgram3`: when no velocity satisfies every line, the one that violates the
/// agent lines least, keeping the first `num_obstacle` (wall) lines hard.
fn lp3(lines: &[Line], num_obstacle: usize, begin: usize, radius: f64, result: &mut DVec2) {
    let mut distance = 0.0;
    for i in begin..lines.len() {
        if det(lines[i].direction, lines[i].point - *result) > distance {
            let mut proj: Vec<Line> = lines[..num_obstacle].to_vec();
            for j in num_obstacle..i {
                let d = det(lines[i].direction, lines[j].direction);
                let point = if d.abs() <= 1e-9 {
                    if lines[i].direction.dot(lines[j].direction) > 0.0 {
                        continue;
                    }
                    0.5 * (lines[i].point + lines[j].point)
                } else {
                    lines[i].point + (det(lines[j].direction, lines[i].point - lines[j].point) / d) * lines[i].direction
                };
                proj.push(Line { point, direction: (lines[j].direction - lines[i].direction).normalize_or_zero() });
            }
            let temp = *result;
            let opt = DVec2::new(-lines[i].direction.y, lines[i].direction.x);
            if lp2(&proj, radius, opt, true, result) < proj.len() {
                *result = temp;
            }
            distance = det(lines[i].direction, lines[i].point - *result);
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// The ORCA half-plane agent `a` must keep its velocity in to avoid `b` for `horizon` seconds,
/// taking `share` of the avoidance (0.5 between equals, 1 against a player who will not yield).
fn orca_line(pos: DVec2, vel: DVec2, other_pos: DVec2, other_vel: DVec2, combined: f64, horizon: f64, dt: f64, share: f64) -> Line {
    let rel_pos = other_pos - pos;
    let rel_vel = vel - other_vel;
    let dist_sq = rel_pos.length_squared();
    let r_sq = combined * combined;
    let inv_h = 1.0 / horizon;
    let (direction, u);
    if dist_sq > r_sq {
        let w = rel_vel - inv_h * rel_pos;
        let w_len_sq = w.length_squared();
        let dot1 = w.dot(rel_pos);
        if dot1 < 0.0 && dot1 * dot1 > r_sq * w_len_sq {
            let w_len = w_len_sq.sqrt();
            let unit_w = w / w_len;
            direction = DVec2::new(unit_w.y, -unit_w.x);
            u = (combined * inv_h - w_len) * unit_w;
        } else {
            let leg = (dist_sq - r_sq).sqrt();
            direction = if det(rel_pos, w) > 0.0 {
                DVec2::new(rel_pos.x * leg - rel_pos.y * combined, rel_pos.x * combined + rel_pos.y * leg) / dist_sq
            } else {
                -DVec2::new(rel_pos.x * leg + rel_pos.y * combined, -rel_pos.x * combined + rel_pos.y * leg) / dist_sq
            };
            let dot2 = rel_vel.dot(direction);
            u = dot2 * direction - rel_vel;
        }
    } else {
        let inv_dt = 1.0 / dt;
        let w = rel_vel - inv_dt * rel_pos;
        let w_len = w.length().max(1e-9);
        let unit_w = w / w_len;
        direction = DVec2::new(unit_w.y, -unit_w.x);
        u = (combined * inv_dt - w_len) * unit_w;
    }
    Line { point: vel + share * u, direction }
}

/// What another headset needs to carry on an agent: what the host replicates anyway, plus who
/// is whose turn at each narrow place and who holds which attack slot. Paths are left out: the new
/// host finds the same ones itself. The turns are what cannot be found again (who came first is
/// history), and the level digest ([`crate::net::LevelDigest`]) has already made sure both
/// headsets number the narrow places the same way.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub pos: DVec3,
    pub vel: DVec3,
    pub params: AgentParams,
    pub goal: Goal,
    #[serde(default = "idle")]
    pub state: AgentState,
    #[serde(default)]
    pub turn: Option<TurnSnapshot>,
    #[serde(default)]
    pub slot: Option<SlotSnapshot>,
}

fn idle() -> AgentState {
    AgentState::Idle
}

/// An agent's place at a narrow place: going through it, or queued for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub choke: u32,
    /// The side it comes from.
    pub from: i8,
    pub inside: bool,
    /// Order among those inside, or in the queue: 0 is first.
    pub place: u32,
}

/// An attack slot: `index` of the `of` slots round player `player`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSnapshot {
    pub player: u32,
    pub index: u32,
    pub of: u32,
}

pub struct Crowd {
    pub settings: CrowdSettings,
    pub agents: Vec<Agent>,
    pub players: Vec<Player>,
    pub chokes: Vec<Choke>,
    choke_of: ChokeMap,
    tokens: Vec<Token>,
    /// Per player: which agent holds each slot.
    slots: Vec<Vec<Option<AgentId>>>,
}

impl Crowd {
    /// A crowd on `world`, for agents of the size the world was baked for.
    pub fn new(world: &NavWorld, settings: CrowdSettings) -> Crowd {
        let radius = world.islands.first().map_or(400.0, |i| i.asset.agent.radius_mm as f64);
        let (chokes, choke_of) = find_chokes(world, radius, settings.gap_mm);
        let tokens = vec![Token::default(); chokes.len()];
        Crowd { settings, agents: Vec::new(), players: Vec::new(), chokes, choke_of, tokens, slots: Vec::new() }
    }

    pub fn add_agent(&mut self, params: AgentParams, pos: DVec3) -> AgentId {
        self.agents.push(Agent {
            pos,
            vel: DVec3::ZERO,
            params,
            goal: Goal::Idle,
            state: AgentState::Idle,
            desired: DVec3::ZERO,
            target: None,
            follower: PathFollower::new(FollowSettings::default()),
            plan: None,
            token: None,
        });
        self.agents.len() - 1
    }

    pub fn set_goal(&mut self, id: AgentId, goal: Goal) {
        if self.agents[id].goal != goal {
            self.agents[id].goal = goal;
            self.agents[id].follower.reset();
            self.leave(id);
        }
    }

    /// The agents as a host replicates them, in id order.
    pub fn snapshot(&self) -> Vec<AgentSnapshot> {
        let mut out: Vec<AgentSnapshot> = self
            .agents
            .iter()
            .map(|a| AgentSnapshot { pos: a.pos, vel: a.vel, params: a.params, goal: a.goal, state: a.state, turn: None, slot: None })
            .collect();
        for (c, t) in self.tokens.iter().enumerate() {
            for (place, &i) in t.inside.iter().enumerate() {
                let from = self.agents[i].plan.map_or(t.side, |p| p.1);
                out[i].turn = Some(TurnSnapshot { choke: c as u32, from, inside: true, place: place as u32 });
            }
            for (place, &(i, from)) in t.queue.iter().enumerate() {
                out[i].turn = Some(TurnSnapshot { choke: c as u32, from, inside: false, place: place as u32 });
            }
        }
        for (p, ring) in self.slots.iter().enumerate() {
            for (k, holder) in ring.iter().enumerate() {
                if let Some(i) = *holder {
                    out[i].slot = Some(SlotSnapshot { player: p as u32, index: k as u32, of: ring.len() as u32 });
                }
            }
        }
        out
    }

    /// A crowd taking over from another host: the same agents under the same ids, in the same
    /// turns and slots, each finding its path again. Two headsets building from the same snapshot
    /// and world get the same crowd, tick for tick. A turn or slot naming a narrow place or player
    /// this crowd does not have is dropped, and that agent simply asks again.
    pub fn from_snapshot(world: &NavWorld, settings: CrowdSettings, agents: &[AgentSnapshot], players: &[Player]) -> Crowd {
        let mut c = Crowd::new(world, settings);
        for a in agents {
            let id = c.add_agent(a.params, a.pos);
            c.set_agent(id, a.pos, a.vel);
            c.set_goal(id, a.goal);
            c.agents[id].state = a.state;
        }
        c.players = players.to_vec();
        let mut turns: Vec<(u32, bool, u32, AgentId, i8)> = agents
            .iter()
            .enumerate()
            .filter_map(|(i, a)| a.turn.filter(|t| (t.choke as usize) < c.tokens.len() && (t.from == 1 || t.from == -1)).map(|t| (t.choke, !t.inside, t.place, i, t.from)))
            .collect();
        turns.sort_unstable();
        for (ch, queued, _, i, from) in turns {
            let t = &mut c.tokens[ch as usize];
            if queued {
                t.queue.push_back((i, from));
            } else if t.inside.is_empty() || t.side == from {
                t.side = from;
                t.inside.push(i);
                c.agents[i].token = Some(ch);
            } else {
                t.queue.push_front((i, from));
            }
            c.agents[i].plan = Some((ch, from));
        }
        c.slots = vec![Vec::new(); c.players.len()];
        for (i, a) in agents.iter().enumerate() {
            let Some(s) = a.slot else { continue };
            let Some(ring) = c.slots.get_mut(s.player as usize) else { continue };
            if ring.is_empty() {
                *ring = vec![None; s.of as usize];
            }
            if ring.len() == s.of as usize && (s.index as usize) < ring.len() && ring[s.index as usize].is_none() {
                ring[s.index as usize] = Some(i);
            }
        }
        c
    }

    /// Drops every path, queue place and attack slot, so each agent plans again on its next
    /// update. For when the world may have changed under the crowd in ways no event reported
    /// (door states replaced wholesale after a host change).
    pub fn replan_all(&mut self) {
        for id in 0..self.agents.len() {
            self.leave(id);
            self.agents[id].follower.reset();
        }
        for t in &mut self.tokens {
            *t = Token::default();
        }
        self.slots.clear();
    }

    /// Where the game actually put the agent, and how fast it is really moving.
    pub fn set_agent(&mut self, id: AgentId, pos: DVec3, vel: DVec3) {
        self.agents[id].pos = pos;
        self.agents[id].vel = vel;
    }

    pub fn set_player(&mut self, i: usize, player: Player) {
        if self.players.len() <= i {
            self.players.resize(i + 1, player);
        }
        self.players[i] = player;
    }

    /// A narrow place's turn state: the side now going through (0: none), who is going through,
    /// and the queue. For debug views.
    pub fn choke_state(&self, c: usize) -> (i8, Vec<AgentId>, Vec<(AgentId, i8)>) {
        let t = &self.tokens[c];
        (t.side, t.inside.clone(), t.queue.iter().copied().collect())
    }

    /// Takes an agent out of any narrow place it was queued for or going through.
    fn leave(&mut self, id: AgentId) {
        if let Some((c, _)) = self.agents[id].plan.take() {
            let t = &mut self.tokens[c as usize];
            t.inside.retain(|&x| x != id);
            t.queue.retain(|q| q.0 != id);
            if t.inside.is_empty() {
                t.side = 0;
            }
        }
        self.agents[id].token = None;
    }

    /// The narrow place an agent is queued for or going through, and the side it comes from
    /// (debug views).
    pub fn plan_of(&self, id: AgentId) -> Option<(u32, i8)> {
        self.agents[id].plan
    }

    /// The narrow place an agent currently has its turn at.
    pub fn token_of(&self, id: AgentId) -> Option<u32> {
        self.agents[id].token
    }

    /// The narrow places the agent's path still goes through, nearest first.
    fn chokes_ahead(&self, world: &NavWorld, id: AgentId) -> Vec<u32> {
        self.chokes_on_path(world, id).map(|v| v.into_iter().map(|x| x.0).collect()).unwrap_or_default()
    }

    /// [`Self::chokes_ahead`], each with the corridor step that crosses it (the polygon left and
    /// the one entered); `None` when it cannot tell: no path, or off its corridor.
    fn chokes_on_path(&self, world: &NavWorld, id: AgentId) -> Option<Vec<(u32, PolyRef, PolyRef)>> {
        let a = &self.agents[id];
        let path = a.follower.path()?;
        let here = world.nearest_poly(a.pos, DVec3::new(500.0, 2000.0, 500.0)).map(|n| n.poly);
        let start = here.and_then(|h| path.corridor.iter().position(|c| c.0 == h))?;
        let mut out = Vec::new();
        for k in start..path.corridor.len().saturating_sub(1) {
            let (from, to) = (path.corridor[k].0, path.corridor[k + 1]);
            let isl = &world.islands[from.island as usize];
            let edge = match to.1 {
                crate::path::Via::Link(l) => world.links[l as usize].from_edge,
                _ => isl.asset.polys[from.poly as usize].edges.iter().position(|&e| e == EdgeLink::Internal(to.0.poly)).map(|e| e as u8),
            };
            let Some(edge) = edge else { continue };
            let key_there = match to.1 {
                crate::path::Via::Edge => world.islands[to.0.island as usize].asset.polys[to.0.poly as usize]
                    .edges
                    .iter()
                    .position(|&e| e == EdgeLink::Internal(from.poly))
                    .map(|e| (to.0.island, to.0.poly, e as u8)),
                _ => None,
            };
            if let Some(&c) = self.choke_of.get(&(from.island, from.poly, edge)).or_else(|| key_there.and_then(|k| self.choke_of.get(&k))) {
                if !out.iter().any(|x: &(u32, PolyRef, PolyRef)| x.0 == c) {
                    out.push((c, from, to.0));
                }
            }
        }
        Some(out)
    }

    fn side(&self, c: u32, pos: DVec3) -> i8 {
        let ch = &self.chokes[c as usize];
        if (flat(pos) - flat(ch.centre)).dot(ch.normal) >= 0.0 { 1 } else { -1 }
    }

    /// Takes turns at narrow places; returns, per agent, whether it must wait and where.
    ///
    /// One agent goes into a narrow place at a time: the next is let in once the one ahead is
    /// past the middle, and only from the same side while any are still in it. The other side
    /// queues, first come first served, beside the opening. A player near it pauses new turns:
    /// players never wait.
    fn take_turns(&mut self, world: &NavWorld) -> Vec<Option<DVec3>> {
        let n = self.agents.len();
        let mut joining: Vec<(f64, AgentId, u32)> = Vec::new();
        for i in 0..n {
            let a = &self.agents[i];
            let (pos, r) = (a.pos, a.params.radius_mm);
            if let Some((c, from)) = a.plan {
                let ch = &self.chokes[c as usize];
                let dist = (flat(pos) - flat(ch.centre)).length();
                let through = self.side(c, pos) != from && dist > r + 300.0;
                let gave_up = dist > self.settings.approach_mm * 2.0 || matches!(a.state, AgentState::Arrived | AgentState::Idle | AgentState::NoPath);
                let changed_mind = a.token == Some(c) && self.side(c, pos) == from && self.chokes_on_path(world, i).is_some_and(|ahead| !ahead.iter().any(|x| x.0 == c));
                if through || gave_up || changed_mind {
                    self.leave(i);
                }
                continue;
            }
            if a.state != AgentState::Moving {
                continue;
            }
            let ahead = self.chokes_ahead(world, i);
            if let Some(&c) = ahead.first() {
                let ch = &self.chokes[c as usize];
                let dist = (flat(pos) - flat(ch.centre)).length();
                if dist < self.settings.approach_mm {
                    joining.push((dist, i, c));
                }
            }
        }
        joining.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, i, c) in joining {
            let a = &self.agents[i];
            let mid = |p: PolyRef| {
                let v = world.poly_dverts(p);
                v.iter().copied().sum::<DVec3>() / v.len().max(1) as f64
            };
            let crossing = self.chokes_on_path(world, i).and_then(|v| v.into_iter().find(|x| x.0 == c)).map(|(_, a, b)| flat(mid(b)) - flat(mid(a)));
            let normal = self.chokes[c as usize].normal;
            let from = match crossing {
                Some(dir) if dir.dot(normal).abs() > 1.0 => {
                    if dir.dot(normal) > 0.0 { -1 } else { 1 }
                }
                _ => {
                    let speed = flat(a.vel).length();
                    let back = if speed > 1.0 { a.pos - a.vel * ((a.params.radius_mm + 300.0) / speed) } else { a.pos };
                    self.side(c, back)
                }
            };
            self.agents[i].plan = Some((c, from));
            self.tokens[c as usize].queue.push_back((i, from));
        }
        for c in 0..self.tokens.len() {
            let ch = self.chokes[c].clone();
            let player_near = self.players.iter().any(|p| (flat(p.pos) - flat(ch.centre)).length() < p.radius_mm + 1500.0);
            let Some(&(i, s)) = self.tokens[c].queue.front() else { continue };
            let t = &self.tokens[c];
            let clear = t.inside.iter().all(|&j| {
                let ahead = (flat(self.agents[j].pos) - flat(ch.centre)).dot(ch.normal) * t.side as f64;
                ahead < self.agents[j].params.radius_mm
            });
            if !player_near && (t.inside.is_empty() || (s == t.side && clear)) {
                let t = &mut self.tokens[c];
                t.queue.pop_front();
                t.side = s;
                t.inside.push(i);
                self.agents[i].token = Some(c as u32);
            }
        }
        let mut wait = vec![None; n];
        for (c, t) in self.tokens.iter().enumerate() {
            let ch = &self.chokes[c];
            let mut per_side = [0usize; 2];
            for &(i, s) in t.queue.iter() {
                let k = &mut per_side[(s > 0) as usize];
                let (column, row) = (*k % 2, *k / 2);
                *k += 1;
                let r = self.agents[i].params.radius_mm;
                let spacing = 2.0 * r + self.settings.gap_mm + 200.0;
                let back = s as f64 * (2.0 * r + 600.0 + row as f64 * spacing);
                let lateral = if column == 0 { 1.0 } else { -1.0 } * spacing;
                let want = flat(ch.centre) + ch.normal * back + ch.along * lateral;
                let spot = [want, flat(ch.centre) + ch.normal * back - ch.along * lateral]
                    .into_iter()
                    .filter_map(|w| world.nearest_poly(up(w, ch.centre.y), DVec3::new(600.0, 1000.0, 600.0)).map(|nn| nn.point))
                    .find(|p| (flat(*p) - flat(ch.centre)).dot(ch.along).abs() >= r + self.settings.gap_mm);
                wait[i] = Some(spot.unwrap_or_else(|| up(flat(ch.centre) + ch.normal * back * 2.0, ch.centre.y)));
            }
        }
        wait
    }

    /// Hands out the slots round each player; returns, per agent, where it should stand if it is
    /// close enough to its player to stop path-following.
    fn assign_slots(&mut self, world: &NavWorld) -> Vec<Option<(DVec3, bool)>> {
        let n = self.agents.len();
        let mut out = vec![None; n];
        self.slots.resize(self.players.len(), Vec::new());
        for (p, player) in self.players.iter().enumerate() {
            let chasers: Vec<AgentId> = (0..n).filter(|&i| self.agents[i].goal == Goal::Player(p)).collect();
            let Some(&first) = chasers.first() else { continue };
            let r = self.agents[first].params.radius_mm;
            let ring = player.radius_mm + r + self.settings.gap_mm;
            let count = ((std::f64::consts::TAU * ring / (2.0 * r + self.settings.gap_mm)).floor() as usize).max(1);
            if self.slots[p].len() != count {
                self.slots[p] = vec![None; count];
            }
            let engaged = |a: &Agent| {
                let d = (flat(a.pos) - flat(player.pos)).length();
                d < self.settings.engage_mm
                    && world.nearest_poly(a.pos, DVec3::new(500.0, 2000.0, 500.0)).is_some_and(|s| world.walkable_line(s.poly, s.point, player.pos))
            };
            let engaged_ids: Vec<AgentId> = chasers.iter().copied().filter(|&i| engaged(&self.agents[i])).collect();
            for s in self.slots[p].iter_mut() {
                if s.is_some_and(|i| !engaged_ids.contains(&i)) {
                    *s = None;
                }
            }
            let slot_pos = |k: usize| player.pos + DVec3::new(ring * (std::f64::consts::TAU * k as f64 / count as f64).cos(), 0.0, ring * (std::f64::consts::TAU * k as f64 / count as f64).sin());
            let mut waiting: Vec<AgentId> = engaged_ids.iter().copied().filter(|i| !self.slots[p].contains(&Some(*i))).collect();
            waiting.sort_by(|&a, &b| (flat(self.agents[a].pos) - flat(player.pos)).length().total_cmp(&(flat(self.agents[b].pos) - flat(player.pos)).length()).then(a.cmp(&b)));
            for i in waiting {
                let free = (0..count).filter(|&k| self.slots[p][k].is_none()).min_by(|&x, &y| {
                    (flat(slot_pos(x)) - flat(self.agents[i].pos)).length().total_cmp(&(flat(slot_pos(y)) - flat(self.agents[i].pos)).length())
                });
                match free {
                    Some(k) => self.slots[p][k] = Some(i),
                    None => {
                        let away = (flat(self.agents[i].pos) - flat(player.pos)).normalize_or_zero();
                        let hold = flat(player.pos) + away * (ring + 2.0 * r + self.settings.gap_mm);
                        out[i] = Some((up(hold, player.pos.y), false));
                    }
                }
            }
            for k in 0..count {
                if let Some(i) = self.slots[p][k] {
                    out[i] = Some((slot_pos(k), true));
                }
            }
        }
        out
    }

    /// Walls near `pos` as half-planes: the navmesh's outer edges (and doorways whose door is
    /// shut) the agent must not cross within the wall horizon.
    fn wall_lines(&self, world: &NavWorld, pos: DVec3, vel_reach: f64) -> Vec<Line> {
        let reach = vel_reach.max(500.0);
        let mut lines = Vec::new();
        let ext = DVec3::new(reach, 1500.0, reach);
        for p in world.polys_in_box(pos - ext, pos + ext) {
            let isl = &world.islands[p.island as usize];
            let poly = &isl.asset.polys[p.poly as usize];
            let v = world.poly_dverts(p);
            let n = v.len();
            if (v[0].y - pos.y).abs() > 1500.0 {
                continue;
            }
            for (e, link) in poly.edges.iter().enumerate() {
                let wall = match *link {
                    EdgeLink::Border => true,
                    EdgeLink::Internal(_) => false,
                    EdgeLink::Socket(_) => !isl.links_from[p.poly as usize]
                        .iter()
                        .any(|&l| world.links[l as usize].enabled && world.links[l as usize].from_edge == Some(e as u8)),
                };
                if !wall {
                    continue;
                }
                let (a, b) = (v[e], v[(e + 1) % n]);
                let (d2, t) = dist_pt_seg_sqr_2d(pos, a, b);
                let len = (b - a).length().max(1.0);
                let end_margin = (50.0 / len).min(0.5);
                if d2.sqrt() > reach || t <= end_margin || t >= 1.0 - end_margin {
                    continue;
                }
                let q = flat(a.lerp(b, t));
                let to_wall = q - flat(pos);
                let dist = to_wall.length();
                let normal = if dist > 1e-6 {
                    to_wall / dist
                } else {
                    let e2 = flat(b - a).normalize_or_zero();
                    DVec2::new(e2.y, -e2.x)
                };
                let limit = dist / self.settings.wall_time_horizon_s;
                lines.push(Line { point: normal * limit, direction: DVec2::new(-normal.y, normal.x) });
            }
        }
        lines
    }

    /// Computes every agent's desired velocity for this tick. Agents and players must have been
    /// updated with where they are now.
    pub fn update(&mut self, world: &NavWorld, graph: &RoomGraph, dt: f64) {
        let n = self.agents.len();
        let slot = self.assign_slots(world);
        let mut targets: Vec<Option<DVec3>> = vec![None; n];
        for i in 0..n {
            let a = &mut self.agents[i];
            let goal = match a.goal {
                Goal::Idle => {
                    a.state = AgentState::Idle;
                    continue;
                }
                Goal::Point(p) => p,
                Goal::Player(p) => match self.players.get(p) {
                    Some(pl) => pl.pos,
                    None => {
                        a.state = AgentState::Idle;
                        continue;
                    }
                },
            };
            if let Some((spot, attacking)) = slot[i] {
                a.state = if attacking { AgentState::Attacking } else { AgentState::Holding };
                targets[i] = Some(spot);
                continue;
            }
            match a.follower.update(world, graph, a.pos, goal, dt) {
                Steer::Walk(t) => {
                    a.state = AgentState::Moving;
                    targets[i] = Some(t);
                }
                Steer::TakeLink { to, .. } => {
                    a.state = AgentState::Moving;
                    targets[i] = Some(to);
                }
                Steer::Arrived => a.state = AgentState::Arrived,
                Steer::NoPath => a.state = AgentState::NoPath,
            }
        }
        let wait = self.take_turns(world);
        for i in 0..n {
            if let Some(spot) = wait[i] {
                self.agents[i].state = AgentState::Waiting;
                targets[i] = Some(spot);
            }
        }
        let mut lining_up = vec![false; n];
        for i in 0..n {
            let (Some(c), Some((_, from))) = (self.agents[i].token, self.agents[i].plan) else { continue };
            let a = &self.agents[i];
            if self.side(c, a.pos) != from {
                continue;
            }
            let ch = &self.chokes[c as usize];
            let r = a.params.radius_mm;
            let rel = flat(a.pos) - flat(ch.centre);
            let off_axis = rel.dot(ch.along).abs();
            let ahead = rel.dot(ch.normal) * from as f64;
            let beyond = up(flat(ch.centre) - ch.normal * from as f64 * (r + 400.0), ch.centre.y);
            let onto_axis = up(flat(ch.centre) + ch.normal * from as f64 * ahead.clamp(r + 100.0, r + 300.0), ch.centre.y);
            let on_floor = |p: DVec3| world.nearest_poly(p, DVec3::new(30.0, 1000.0, 30.0)).is_some_and(|n| n.over_poly);
            if !(on_floor(onto_axis) && on_floor(beyond)) {
                continue;
            }
            if off_axis > 80.0 && ahead > r + 50.0 {
                targets[i] = Some(onto_axis);
                lining_up[i] = true;
            } else {
                targets[i] = Some(beyond);
            }
        }

        let preferred: Vec<DVec2> = (0..n)
            .map(|i| -> DVec2 {
                let a = &self.agents[i];
                let Some(t) = targets[i] else { return DVec2::ZERO };
                let d = flat(t) - flat(a.pos);
                let dist = d.length();
                let stopping = lining_up[i]
                    || matches!(a.state, AgentState::Attacking | AgentState::Holding | AgentState::Waiting)
                    || a.follower.path().is_some_and(|p| p.waypoints.last().is_some_and(|w| (flat(w.pos()) - flat(t)).length() < 1.0));
                let speed = if stopping { a.params.max_speed_mm_s.min(dist / self.settings.arrive_slow_mm * a.params.max_speed_mm_s) } else { a.params.max_speed_mm_s };
                if dist < 20.0 {
                    return DVec2::ZERO;
                }
                let pref = d / dist * speed;
                let oncoming = a.token.is_none() && self.agents.iter().enumerate().any(|(j, b)| {
                    let rel = flat(b.pos) - flat(a.pos);
                    j != i && rel.length() < 3000.0 && rel.dot(pref) > 0.0 && flat(b.vel).dot(pref) < 0.0
                }) || self.players.iter().any(|p| {
                    let rel = flat(p.pos) - flat(a.pos);
                    rel.length() < 3000.0 && rel.dot(pref) > 0.0 && flat(p.vel).dot(pref) < 0.0
                });
                if oncoming { keep_right(pref) } else { pref }
            })
            .collect();
        let mut preferred = preferred;
        for i in 0..n {
            let a = &self.agents[i];
            if !matches!(a.state, AgentState::Arrived | AgentState::Waiting | AgentState::Holding | AgentState::Idle) {
                continue;
            }
            let mut step = DVec2::ZERO;
            for (j, b) in self.agents.iter().enumerate() {
                let Some(bt) = targets[j] else { continue };
                if j == i || b.state != AgentState::Moving {
                    continue;
                }
                let (bp, ap) = (flat(b.pos), flat(a.pos));
                let dir = (flat(bt) - bp).normalize_or_zero();
                let along = (ap - bp).dot(dir);
                if along <= 0.0 || along > 2500.0 {
                    continue;
                }
                let off = ap - (bp + dir * along);
                let clear = a.params.radius_mm + b.params.radius_mm + self.settings.gap_mm;
                if off.length() < clear {
                    let side = if off.length() > 1.0 { off.normalize() } else { DVec2::new(-dir.y, dir.x) };
                    step += side * a.params.max_speed_mm_s * 0.5;
                }
            }
            if step != DVec2::ZERO {
                preferred[i] = step.clamp_length_max(a.params.max_speed_mm_s * 0.5);
                if self.agents[i].state == AgentState::Arrived {
                    targets[i] = Some(self.agents[i].pos);
                }
            }
        }
        let rank = |a: &Agent| -> i32 {
            a.params.priority as i32 * 10
                + if a.token.is_some() { 3 } else { 0 }
                + match a.state {
                    AgentState::Moving | AgentState::Attacking => 1,
                    _ => 0,
                }
        };
        let mut ranks: Vec<i32> = self.agents.iter().map(rank).collect();
        for t in &self.tokens {
            for (k, &i) in t.inside.iter().enumerate() {
                ranks[i] += (t.inside.len() - k) as i32;
            }
        }
        let mut desired = vec![DVec2::ZERO; n];
        for i in 0..n {
            let a = &self.agents[i];
            if targets[i].is_none() {
                continue;
            }
            let (pos, vel) = (flat(a.pos), flat(a.vel));
            let mut lines = self.wall_lines(world, a.pos, a.params.max_speed_mm_s * self.settings.wall_time_horizon_s + a.params.radius_mm);
            let walls = lines.len();
            for j in 0..n {
                if j == i {
                    continue;
                }
                let b = &self.agents[j];
                if (b.pos.y - a.pos.y).abs() > 1500.0 || (flat(b.pos) - pos).length() > self.settings.neighbour_mm {
                    continue;
                }
                let share = match ranks[i].cmp(&ranks[j]) {
                    std::cmp::Ordering::Equal => 0.5,
                    std::cmp::Ordering::Greater => 0.1,
                    std::cmp::Ordering::Less => 0.9,
                };
                let combined = a.params.radius_mm + b.params.radius_mm + self.settings.gap_mm * 0.25;
                lines.push(orca_line(pos, vel, flat(b.pos), flat(b.vel), combined, self.settings.time_horizon_s, dt, share));
            }
            for p in &self.players {
                if (p.pos.y - a.pos.y).abs() > 1500.0 || (flat(p.pos) - pos).length() > self.settings.neighbour_mm {
                    continue;
                }
                let combined = a.params.radius_mm + p.radius_mm + self.settings.gap_mm * 0.25;
                lines.push(orca_line(pos, vel, flat(p.pos), flat(p.vel), combined, self.settings.time_horizon_s, dt, 1.0));
            }
            let mut result = DVec2::ZERO;
            let fail = lp2(&lines, a.params.max_speed_mm_s, preferred[i], false, &mut result);
            if fail < lines.len() {
                lp3(&lines, walls, fail, a.params.max_speed_mm_s, &mut result);
            }
            desired[i] = result;
        }
        for ((a, d), t) in self.agents.iter_mut().zip(desired).zip(targets) {
            a.desired = up(d, 0.0);
            a.target = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_agents_head_on_pass_each_other() {
        let (mut p, mut v) = ([DVec2::new(0.0, 0.0), DVec2::new(10_000.0, 0.0)], [DVec2::ZERO; 2]);
        let goal = [DVec2::new(10_000.0, 0.0), DVec2::new(0.0, 0.0)];
        let (r, speed, dt): (f64, f64, f64) = (400.0, 1500.0, 1.0 / 36.0);
        let mut closest = f64::MAX;
        for _ in 0..(36 * 15) {
            let mut nv = [DVec2::ZERO; 2];
            for i in 0..2 {
                let j = 1 - i;
                let to = goal[i] - p[i];
                let pref = if to.length() < 20.0 { DVec2::ZERO } else { keep_right(to.normalize() * speed.min(to.length() / dt)) };
                let lines = vec![orca_line(p[i], v[i], p[j], v[j], 2.0 * r, 2.0, dt, 0.5)];
                let mut res = DVec2::ZERO;
                let fail = lp2(&lines, speed, pref, false, &mut res);
                if fail < lines.len() {
                    lp3(&lines, 0, fail, speed, &mut res);
                }
                nv[i] = res;
            }
            v = nv;
            for (pi, vi) in p.iter_mut().zip(v) {
                *pi += vi * dt;
            }
            closest = closest.min((p[0] - p[1]).length());
        }
        assert!(closest >= 2.0 * r - 5.0, "they came within {closest} mm");
        assert!((p[0] - goal[0]).length() < 50.0 && (p[1] - goal[1]).length() < 50.0, "{p:?}");
    }

    #[test]
    fn a_wall_line_stops_an_agent_at_the_wall() {
        let normal = DVec2::new(1.0, 0.0);
        let mut x = 0.0;
        for _ in 0..200 {
            let dist = 1000.0 - x;
            let lines = vec![Line { point: normal * (dist / 0.4), direction: DVec2::new(-normal.y, normal.x) }];
            let mut res = DVec2::ZERO;
            lp2(&lines, 1500.0, DVec2::new(1500.0, 0.0), false, &mut res);
            x += res.x / 36.0;
            assert!(x <= 1000.0 + 1e-6);
        }
        assert!(x > 990.0, "it should get right up to the wall, got to {x}");
    }
}
