//! Path search: which polygons to walk through (A*), then the straight line through them (the
//! funnel, "string pulling").
//!
//! Detour's A* puts each polygon's node at the middle of the first edge it was reached through and
//! never moves it, so it can prefer a worse corridor (the research found its paths measurably
//! longer than the shortest on long thin polygons). Here a node sits at the point of its entry edge
//! nearest the node it was reached from, and moves whenever a cheaper way in is found (Godot's
//! approach). That makes each step's cost the length actually walked, so the corridor found is
//! close to the shortest; `tests/path.rs` measures how close against the exact shortest path.
//!
//! The search runs in slices ([`PathQuery::step`]) on tables allocated once per world, so many
//! agents can share a per-frame budget ([`PathScheduler`]) and no search allocates.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use glam::DVec3;

use crate::asset::EdgeLink;
use crate::query::dist_pt_seg_sqr_2d;
use crate::world::{LinkId, LinkKind, NavWorld, PolyRef};

/// How a corridor step was entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Start,
    /// Across an edge shared inside one piece.
    Edge,
    /// Over a link: a doorway portal or a typed link (vent, ladder, ...).
    Link(LinkId),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waypoint {
    /// Walk to here.
    Walk(DVec3),
    /// Walk to here, then take the typed link (climb the ladder, crawl the vent)...
    LinkStart(DVec3, LinkId),
    /// ...arriving here.
    LinkEnd(DVec3, LinkId),
}

impl Waypoint {
    pub fn pos(&self) -> DVec3 {
        match *self {
            Waypoint::Walk(p) | Waypoint::LinkStart(p, _) | Waypoint::LinkEnd(p, _) => p,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// The polygons walked through, start to goal, and how each was entered.
    pub corridor: Vec<(PolyRef, Via)>,
    /// The straight path: corners to walk to, start and goal included.
    pub waypoints: Vec<Waypoint>,
    /// Whether it reaches the goal, or only the reachable polygon nearest it.
    pub complete: bool,
    /// The world version it was found at: a later gate change may have invalidated it.
    pub version: u64,
}

impl Path {
    /// Walking length along the waypoints, millimetres, link costs not included.
    pub fn length(&self) -> f64 {
        self.waypoints.windows(2).map(|w| (w[1].pos() - w[0].pos()).length()).sum()
    }

    /// The links (doors included) the path depends on, to replan when one changes.
    pub fn links(&self) -> impl Iterator<Item = LinkId> + '_ {
        self.corridor.iter().filter_map(|(_, v)| if let Via::Link(l) = v { Some(*l) } else { None })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    InProgress,
    Done,
    /// The start or goal is not on the navmesh.
    Failed,
}

#[derive(Clone, Copy)]
struct Node {
    /// The search this entry belongs to; older entries count as unvisited.
    search: u32,
    g: f64,
    pos: DVec3,
    parent: u32,
    via: Via,
    closed: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Open {
    f: f64,
    node: u32,
}

impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then_with(|| other.node.cmp(&self.node))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const NONE: u32 = u32::MAX;

/// One path search, run in slices. Allocate one per world (or per worker) and reuse it.
pub struct PathQuery {
    nodes: Vec<Node>,
    open: BinaryHeap<Open>,
    search: u32,
    start: DVec3,
    goal: DVec3,
    start_poly: PolyRef,
    goal_poly: PolyRef,
    status: Status,
    /// The node nearest the goal so far, for a partial path when the goal cannot be reached.
    best: (f64, u32),
    found: u32,
    refs: Vec<PolyRef>,
    /// Islands the search may enter; empty means all.
    allowed: Vec<bool>,
    /// Polygons expanded in the current search.
    pub expanded: usize,
}

impl PathQuery {
    pub fn new(world: &NavWorld) -> PathQuery {
        let n = world.poly_count();
        let mut refs = vec![PolyRef { island: 0, poly: 0 }; n];
        for (ii, isl) in world.islands.iter().enumerate() {
            for p in 0..isl.asset.polys.len() {
                let r = PolyRef { island: ii as u32, poly: p as u16 };
                refs[world.poly_index(r)] = r;
            }
        }
        PathQuery {
            nodes: vec![Node { search: 0, g: 0.0, pos: DVec3::ZERO, parent: NONE, via: Via::Start, closed: false }; n],
            open: BinaryHeap::new(),
            search: 0,
            start: DVec3::ZERO,
            goal: DVec3::ZERO,
            start_poly: PolyRef { island: 0, poly: 0 },
            goal_poly: PolyRef { island: 0, poly: 0 },
            status: Status::Failed,
            best: (f64::MAX, NONE),
            found: NONE,
            refs,
            allowed: Vec::new(),
            expanded: 0,
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// Starts a search between two points already located on polygons.
    pub fn begin(&mut self, world: &NavWorld, start_poly: PolyRef, start: DVec3, goal_poly: PolyRef, goal: DVec3) {
        if self.nodes.len() != world.poly_count() {
            *self = PathQuery::new(world);
        }
        self.search = self.search.wrapping_add(1).max(1);
        self.open.clear();
        self.start = start;
        self.goal = goal;
        self.start_poly = start_poly;
        self.goal_poly = goal_poly;
        self.found = NONE;
        self.expanded = 0;
        self.allowed.clear();
        let i = world.poly_index(start_poly) as u32;
        self.nodes[i as usize] = Node { search: self.search, g: 0.0, pos: start, parent: NONE, via: Via::Start, closed: false };
        self.best = ((goal - world.closest_point_on_poly(start_poly, goal).0).length(), i);
        self.open.push(Open { f: (goal - start).length(), node: i });
        self.status = Status::InProgress;
    }

    /// Locates both points (within `half_extents`) and starts a search.
    pub fn begin_at(&mut self, world: &NavWorld, start: DVec3, goal: DVec3, half_extents: DVec3) -> Status {
        if let (Some(s), Some(g)) = (world.nearest_poly(start, half_extents), world.nearest_poly(goal, half_extents)) {
            self.begin(world, s.poly, s.point, g.poly, g.point);
            Status::InProgress
        } else {
            self.status = Status::Failed;
            Status::Failed
        }
    }

    /// Keeps the current search to these islands (call after `begin`).
    pub fn restrict_to(&mut self, world: &NavWorld, islands: &[u32]) {
        self.allowed = vec![false; world.islands.len()];
        for &i in islands {
            self.allowed[i as usize] = true;
        }
    }

    /// Expands up to `max_expansions` polygons. Returns the status after.
    pub fn step(&mut self, world: &NavWorld, max_expansions: usize) -> Status {
        if self.status != Status::InProgress {
            return self.status;
        }
        let goal_index = world.poly_index(self.goal_poly) as u32;
        for _ in 0..max_expansions {
            let Some(top) = self.open.pop() else {
                self.status = Status::Done;
                return self.status;
            };
            let cur = top.node;
            let node = self.nodes[cur as usize];
            if node.closed || node.search != self.search {
                continue;
            }
            if top.f > node.g + (self.goal - node.pos).length() + 1e-9 {
                continue;
            }
            self.nodes[cur as usize].closed = true;
            self.expanded += 1;
            if cur == goal_index {
                self.found = cur;
                self.status = Status::Done;
                return self.status;
            }
            let pref = self.refs[cur as usize];
            for (next, portal, via, cost) in portals(world, pref) {
                if !self.allowed.is_empty() && !self.allowed[next.island as usize] {
                    continue;
                }
                let ni = world.poly_index(next) as u32;
                let n = self.nodes[ni as usize];
                let (_, t) = dist_pt_seg_sqr_2d(node.pos, portal.0, portal.1);
                let entry = portal.0.lerp(portal.1, t);
                let g = node.g + (entry - node.pos).length() + cost;
                if n.search == self.search && g >= n.g - 1e-6 {
                    continue;
                }
                if n.search != self.search {
                    let near = (self.goal - world.closest_point_on_poly(next, self.goal).0).length();
                    if near < self.best.0 {
                        self.best = (near, ni);
                    }
                }
                self.nodes[ni as usize] = Node { search: self.search, g, pos: entry, parent: cur, via, closed: false };
                let h = (self.goal - entry).length();
                self.open.push(Open { f: g + h, node: ni });
            }
        }
        self.status
    }

    /// The path found: the corridor and the straight path through it. When the goal was not
    /// reachable, the path to the reachable polygon nearest it, marked incomplete.
    pub fn finish(&self, world: &NavWorld) -> Option<Path> {
        if self.status != Status::Done {
            return None;
        }
        let (end, complete) = if self.found == NONE { (self.best.1, false) } else { (self.found, true) };
        let mut corridor = Vec::new();
        let mut at = end;
        while at != NONE {
            let n = self.nodes[at as usize];
            corridor.push((self.refs[at as usize], n.via));
            at = n.parent;
        }
        corridor.reverse();
        let goal = if complete { self.goal } else { world.closest_point_on_poly(corridor.last().unwrap().0, self.goal).0 };
        let waypoints = straighten(world, &corridor, self.start, goal);
        Some(Path { corridor, waypoints, complete, version: world.version() })
    }
}

/// Every way out of `p` as (next polygon, its doorway as a segment, how, extra cost).
fn portals(world: &NavWorld, p: PolyRef) -> Vec<(PolyRef, (DVec3, DVec3), Via, f64)> {
    let isl = &world.islands[p.island as usize];
    let poly = &isl.asset.polys[p.poly as usize];
    let verts = world.poly_dverts(p);
    let n = verts.len();
    let mut out = Vec::new();
    for (e, link) in poly.edges.iter().enumerate() {
        if let EdgeLink::Internal(o) = *link {
            out.push((PolyRef { island: p.island, poly: o }, (verts[e], verts[(e + 1) % n]), Via::Edge, 0.0));
        }
    }
    for &l in &isl.links_from[p.poly as usize] {
        let link = &world.links[l as usize];
        if link.enabled {
            let s = link.from_segment.map(|v| v.as_dvec3());
            out.push((link.to, (s[0], s[1]), Via::Link(l), f64::from(link.cost_mm)));
        }
    }
    out
}

/// The doorway between consecutive corridor polygons as (left, right), seen walking from `a`.
fn portal_between(world: &NavWorld, a: PolyRef, b: PolyRef, via: Via) -> Vec<(DVec3, DVec3)> {
    if let Via::Link(l) = via {
        let link = &world.links[l as usize];
        let f = link.from_segment.map(|v| v.as_dvec3());
        let t = link.to_segment.map(|v| v.as_dvec3());
        if f == t { vec![(f[0], f[1])] } else { vec![(f[0], f[1]), (t[0], t[1])] }
    } else {
        let isl = &world.islands[a.island as usize];
        let poly = &isl.asset.polys[a.poly as usize];
        let verts = world.poly_dverts(a);
        let n = verts.len();
        let e = poly.edges.iter().position(|&x| x == EdgeLink::Internal(b.poly)).expect("corridor steps are neighbours");
        vec![(verts[(e + 1) % n], verts[e])]
    }
}

/// Which side of the line a -> b p is on, from above: positive left, negative right.
fn side(a: DVec3, b: DVec3, p: DVec3) -> f64 {
    (b.z - a.z) * (p.x - a.x) - (b.x - a.x) * (p.z - a.z)
}

fn same(a: DVec3, b: DVec3) -> bool {
    (a.x - b.x).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6
}

/// The funnel algorithm over a corridor, split at typed links (each a jump the agent makes).
pub fn straighten(world: &NavWorld, corridor: &[(PolyRef, Via)], start: DVec3, goal: DVec3) -> Vec<Waypoint> {
    let mut out = vec![Waypoint::Walk(start)];
    let mut portals: Vec<(DVec3, DVec3)> = Vec::new();
    let mut from = start;
    for w in corridor.windows(2) {
        let (a, (b, via)) = (w[0].0, w[1]);
        if let Via::Link(l) = via {
            let link = &world.links[l as usize];
            if let LinkKind::Typed(_) = link.kind {
                let s = link.from_segment.map(|v| v.as_dvec3());
                let (_, t) = dist_pt_seg_sqr_2d(from, s[0], s[1]);
                let at = s[0].lerp(s[1], t);
                funnel(from, at, &portals, &mut out);
                out.push(Waypoint::LinkStart(at, l));
                let e = link.to_segment.map(|v| v.as_dvec3());
                let land = e[0].lerp(e[1], t);
                out.push(Waypoint::LinkEnd(land, l));
                portals.clear();
                from = land;
                continue;
            }
        }
        portals.extend(portal_between(world, a, b, via));
    }
    funnel(from, goal, &portals, &mut out);
    out.dedup_by(|a, b| same(a.pos(), b.pos()) && matches!(a, Waypoint::Walk(_)));
    out
}

/// Simple stupid funnel (Mononen): the shortest line from `start` to `goal` through `portals`,
/// each (left, right). Appends the corners after `start`, then `goal`.
fn funnel(start: DVec3, goal: DVec3, portals: &[(DVec3, DVec3)], out: &mut Vec<Waypoint>) {
    let mut pts: Vec<(DVec3, DVec3)> = Vec::with_capacity(portals.len() + 1);
    pts.extend_from_slice(portals);
    pts.push((goal, goal));
    let (mut apex, mut left, mut right) = (start, start, start);
    let (mut li, mut ri) = (0usize, 0usize);
    let mut i = 0usize;
    while i < pts.len() {
        let (nl, nr) = pts[i];
        if side(apex, right, nr) >= 0.0 {
            if same(apex, right) || side(apex, left, nr) < 0.0 {
                right = nr;
                ri = i;
            } else {
                out.push(Waypoint::Walk(left));
                apex = left;
                right = apex;
                ri = li;
                i = li + 1;
                continue;
            }
        }
        if side(apex, left, nl) <= 0.0 {
            if same(apex, left) || side(apex, right, nl) > 0.0 {
                left = nl;
                li = i;
            } else {
                out.push(Waypoint::Walk(right));
                apex = right;
                left = apex;
                li = ri;
                i = ri + 1;
                continue;
            }
        }
        i += 1;
    }
    if !out.last().is_some_and(|w| same(w.pos(), goal)) {
        out.push(Waypoint::Walk(goal));
    }
}

/// One-call search: locate, search to completion, straighten.
pub fn find_path(world: &NavWorld, start: DVec3, goal: DVec3, half_extents: DVec3) -> Option<Path> {
    let mut q = PathQuery::new(world);
    if q.begin_at(world, start, goal, half_extents) == Status::Failed {
        return None;
    }
    while q.step(world, usize::MAX) == Status::InProgress {}
    q.finish(world)
}

/// Path search guided by the room graph: the rooms to pass through are chosen on exact
/// doorway-to-doorway walking distances, then the polygon search runs inside those rooms only.
///
/// Plain A* scores a route by the zig-zag through its polygons' entry points; across many large
/// rooms of thin polygons that overstates some routes more than others, and it was measured
/// picking a route 3% longer than one through the doorways' middles. The room graph's distances
/// are already straightened, so the choice of rooms is right, and inside a room the search is
/// within 0.01% of the shortest (`tests/path.rs`).
pub fn find_path_guided(world: &NavWorld, graph: &crate::world::RoomGraph, start: DVec3, goal: DVec3, half_extents: DVec3) -> Option<Path> {
    let s = world.nearest_poly(start, half_extents)?;
    let g = world.nearest_poly(goal, half_extents)?;
    let mut q = PathQuery::new(world);
    let rooms = route_rooms(world, graph, &mut q, (s.poly, s.point), (g.poly, g.point));
    q.begin(world, s.poly, s.point, g.poly, g.point);
    if let Some(rooms) = rooms {
        q.restrict_to(world, &rooms);
    }
    while q.step(world, usize::MAX) == Status::InProgress {}
    q.finish(world)
}

/// The rooms of the best route from `start` to `goal`, or `None` to search everywhere (same room,
/// or no route by the graph). First and last legs are measured by searches inside those rooms.
fn route_rooms(world: &NavWorld, graph: &crate::world::RoomGraph, q: &mut PathQuery, start: (PolyRef, DVec3), goal: (PolyRef, DVec3)) -> Option<Vec<u32>> {
    let (rs, rg) = (start.0.island, goal.0.island);
    if rs == rg {
        return None;
    }
    let mut legs = |room: u32, at: (PolyRef, DVec3)| -> Vec<(u32, u64)> {
        let mut out = Vec::new();
        for &c in &graph.doorways[room as usize] {
            let m = world.connections[c as usize].midpoint.as_dvec3();
            let Some(mp) = world.nearest_poly_in(room, m, DVec3::new(200.0, 500.0, 200.0)) else { continue };
            q.begin(world, at.0, at.1, mp.poly, mp.point);
            q.restrict_to(world, &[room]);
            while q.step(world, usize::MAX) == Status::InProgress {}
            if let Some(p) = q.finish(world).filter(|p| p.complete) {
                out.push((c, p.length().round() as u64));
            }
        }
        out
    };
    let from = legs(rs, start);
    let to = legs(rg, goal);
    let (chain, _) = graph.route_between(world, &from, &to)?;
    let mut rooms = vec![rs, rg];
    for c in chain {
        let conn = &world.connections[c as usize];
        rooms.extend([conn.a.island, conn.b.island]);
    }
    rooms.sort_unstable();
    rooms.dedup();
    Some(rooms)
}

/// A request for [`PathScheduler`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request {
    pub id: u64,
    pub start: DVec3,
    pub goal: DVec3,
}

/// Runs path searches one after another, first come first served, within a per-frame budget of
/// polygon expansions, so many agents asking at once cannot spike a frame.
pub struct PathScheduler {
    query: PathQuery,
    queue: VecDeque<Request>,
    current: Option<Request>,
    pub half_extents: DVec3,
}

impl PathScheduler {
    pub fn new(world: &NavWorld) -> PathScheduler {
        PathScheduler { query: PathQuery::new(world), queue: VecDeque::new(), current: None, half_extents: DVec3::new(1000.0, 2000.0, 1000.0) }
    }

    pub fn request(&mut self, r: Request) {
        self.queue.push_back(r);
    }

    pub fn pending(&self) -> usize {
        self.queue.len() + usize::from(self.current.is_some())
    }

    /// Spends up to `budget` polygon expansions; returns the requests finished, each with its
    /// path (`None` when the start or goal is off the navmesh).
    pub fn update(&mut self, world: &NavWorld, mut budget: usize) -> Vec<(u64, Option<Path>)> {
        let mut done = Vec::new();
        while budget > 0 {
            if self.current.is_none() {
                let Some(r) = self.queue.pop_front() else { break };
                if self.query.begin_at(world, r.start, r.goal, self.half_extents) == Status::Failed {
                    done.push((r.id, None));
                    continue;
                }
                self.current = Some(r);
            }
            let before = self.query.expanded;
            let st = self.query.step(world, budget);
            budget = budget.saturating_sub((self.query.expanded - before).max(1));
            if st != Status::InProgress {
                let r = self.current.take().unwrap();
                done.push((r.id, self.query.finish(world)));
            }
        }
        done
    }
}
