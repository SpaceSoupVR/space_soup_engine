//! The runtime navigation world: baked room pieces placed into a level and joined.
//!
//! A level is **islands joined by links**, not a tile grid: pieces are placed by quarter turns
//! wherever the level generator puts them, so they share no grid (the design research's reason for
//! not copying Detour's tiles). Each placed piece is an [`Island`]: the piece's baked asset, shared
//! between every copy, plus its vertices turned and moved into the world. Everything between
//! islands is a [`Link`]: a directed hop from one polygon to another, either through a doorway
//! ([`LinkKind::Portal`]) or a typed link (vent, ladder, drop, ...).
//!
//! Doorways are joined by **socket identity**: the generator says which doorway meets which, and
//! [`NavWorld::connect`] builds the portal from the two pieces' socket edges. Position is only used
//! to check that claim ([`NavWorld::check_joins`]), never to guess a join, because a tolerance
//! join silently links things that merely happen to be close.
//!
//! Doors are [`Gate`]s: closing one disables its links outright (a closed door is not an
//! expensive door; an "expensive" closed door is still taken when it is the only way, which the
//! research found shipped engines doing). Each change bumps [`NavWorld::version`] and queues a
//! [`NavEvent`] so agents whose paths use the door can replan.
//!
//! All geometry is integer millimetres and every container is ordered, so assembling the same level
//! gives the same world on every headset.

use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use glam::IVec3;

use crate::asset::{ccw_from_above, Dir, EdgeLink, LinkKind as TypedKind, NavPieceAsset, Turn};

/// A polygon in the world: which island, which of its piece's polygons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PolyRef {
    pub island: u32,
    pub poly: u16,
}

pub type IslandId = u32;
pub type LinkId = u32;
pub type GateId = u32;
pub type ConnectionId = u32;

/// One doorway of one island.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SocketRef {
    pub island: IslandId,
    pub socket: u16,
}

/// A doorway in world space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldSocket {
    /// The walkable span's ends on the doorway plane (equal when this agent cannot pass).
    pub a: IVec3,
    pub b: IVec3,
    pub outward: Dir,
    pub size_class: String,
    pub connection: Option<ConnectionId>,
}

pub struct Island {
    pub asset: Arc<NavPieceAsset>,
    pub turn: Turn,
    pub offset: IVec3,
    /// The asset's vertices turned and moved into the world.
    pub vertices: Vec<IVec3>,
    pub min: IVec3,
    pub max: IVec3,
    pub sockets: Vec<WorldSocket>,
    /// Per polygon: the links leaving it.
    pub links_from: Vec<Vec<LinkId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// Through a doorway between two pieces.
    Portal,
    /// A vent, ladder, drop, ... as authored in the piece.
    Typed(TypedKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    pub from: PolyRef,
    pub to: PolyRef,
    /// Where the link leaves `from` and arrives on `to`, each as [left, right] seen walking from
    /// `from` to `to`. For a portal, both lie on their own piece's doorway plane (the same plane
    /// unless the generator welded the doorways within its tolerance).
    pub from_segment: [IVec3; 2],
    pub to_segment: [IVec3; 2],
    /// Extra cost of taking it, millimetres of walking.
    pub cost_mm: u32,
    pub enabled: bool,
    pub gate: Option<GateId>,
    pub connection: Option<ConnectionId>,
    /// For a portal: the edge of `from` it leaves through.
    pub from_edge: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub a: SocketRef,
    pub b: SocketRef,
    pub links: Vec<LinkId>,
    pub gate: Option<GateId>,
    /// How far apart the two doorway planes are, mm (0 for a clean join).
    pub gap_mm: i32,
    /// The doorway's middle, for the room graph.
    pub midpoint: IVec3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub name: String,
    pub open: bool,
    pub links: Vec<LinkId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavEvent {
    /// A door opened or closed. Paths through `gate` made before `version` are stale.
    GateChanged { gate: GateId, open: bool, version: u64 },
    LinkChanged { link: LinkId, enabled: bool, version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinError {
    NoSuchSocket(SocketRef),
    AlreadyJoined(SocketRef),
    NotFacing { a: Dir, b: Dir },
    SizeClass(String, String),
    TooFar { gap_mm: i32, tolerance_mm: i32 },
    Misaligned { lateral_mm: i32 },
    Step { dy_mm: i32 },
}

/// What [`NavWorld::check_joins`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinIssue {
    /// Two unjoined doorways face each other within tolerance: the level says they do not meet,
    /// the geometry says they do.
    MissedJoin(SocketRef, SocketRef),
    /// A doorway joined to nothing: unless something blocks it, a way out of the level.
    Unjoined(SocketRef),
    /// A join this agent class cannot pass (the doorway is narrower than it).
    Impassable(ConnectionId),
}

/// How far apart joined doorway planes may be: the generator welds loops within 0.12 m.
pub const JOIN_TOLERANCE_MM: i32 = 150;

pub struct NavWorld {
    pub islands: Vec<Island>,
    pub links: Vec<Link>,
    pub gates: Vec<Gate>,
    pub connections: Vec<Connection>,
    /// Typed links that could not be attached: (island, index into its asset's links) and which
    /// end found no floor ("start", "end" or "both"). [`crate::validate`] reports them.
    pub unattached_links: Vec<(IslandId, usize, &'static str)>,
    version: u64,
    events: Vec<NavEvent>,
    components: Option<Vec<u32>>,
    island_base: Vec<u32>,
}

impl Default for NavWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// The left side of walking along `d`, from above: a quarter turn of `d`.
fn left_of(d: Dir) -> IVec3 {
    Turn(1).apply(d.vec())
}

fn axis_of(d: Dir) -> usize {
    if matches!(d, Dir::PosX | Dir::NegX) { 0 } else { 2 }
}

impl NavWorld {
    pub fn new() -> NavWorld {
        NavWorld {
            islands: Vec::new(),
            links: Vec::new(),
            gates: Vec::new(),
            connections: Vec::new(),
            unattached_links: Vec::new(),
            version: 0,
            events: Vec::new(),
            components: None,
            island_base: Vec::new(),
        }
    }

    /// Bumped by every change to what is passable.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn drain_events(&mut self) -> Vec<NavEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn poly_count(&self) -> usize {
        self.islands.iter().map(|i| i.asset.polys.len()).sum()
    }

    /// Places a piece. Its typed links (vents, ladders, ...) are attached here.
    pub fn add_island(&mut self, asset: Arc<NavPieceAsset>, turn: Turn, offset: IVec3) -> IslandId {
        let id = self.islands.len() as IslandId;
        let place = |v: IVec3| turn.apply(v) + offset;
        let vertices: Vec<IVec3> = asset.vertices.iter().map(|&v| place(v)).collect();
        let (min, max) = vertices.iter().fold((IVec3::MAX, IVec3::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let sockets = asset
            .sockets
            .iter()
            .map(|s| WorldSocket { a: place(s.a), b: place(s.b), outward: turn.apply_dir(s.outward), size_class: s.size_class.clone(), connection: None })
            .collect();
        self.island_base.push(self.island_base.last().copied().unwrap_or(0) + self.islands.last().map_or(0, |i| i.asset.polys.len() as u32));
        let links_from = vec![Vec::new(); asset.polys.len()];
        let typed = asset.links.clone();
        self.islands.push(Island { asset, turn, offset, vertices, min, max, sockets, links_from });

        for (li, l) in typed.into_iter().enumerate() {
            let start = [place(l.start[0]), place(l.start[1])];
            let end = [place(l.end[0]), place(l.end[1])];
            let (sp, ep) = (self.locate_in(id, (start[0] + start[1]) / 2), self.locate_in(id, (end[0] + end[1]) / 2));
            let (Some(sp), Some(ep)) = (sp, ep) else {
                let which = match (sp, ep) {
                    (None, None) => "both",
                    (None, _) => "start",
                    _ => "end",
                };
                self.unattached_links.push((id, li, which));
                continue;
            };
            let (from, to) = (PolyRef { island: id, poly: sp }, PolyRef { island: id, poly: ep });
            let kind = LinkKind::Typed(l.kind.clone());
            self.push_link(Link { kind: kind.clone(), from, to, from_segment: start, to_segment: end, cost_mm: l.cost_mm, enabled: l.enabled, gate: None, connection: None, from_edge: None });
            if l.bidirectional {
                self.push_link(Link { kind, from: to, to: from, from_segment: end, to_segment: start, cost_mm: l.cost_mm, enabled: l.enabled, gate: None, connection: None, from_edge: None });
            }
        }
        self.components = None;
        id
    }

    fn push_link(&mut self, link: Link) -> LinkId {
        let id = self.links.len() as LinkId;
        self.islands[link.from.island as usize].links_from[link.from.poly as usize].push(id);
        self.links.push(link);
        id
    }

    /// World positions of a polygon's vertices, counter-clockwise from above.
    pub fn poly_vertices(&self, p: PolyRef) -> impl Iterator<Item = IVec3> + '_ {
        let island = &self.islands[p.island as usize];
        island.asset.polys[p.poly as usize].verts.iter().map(move |&v| island.vertices[v as usize])
    }

    /// The polygon of `island` under or over `p` whose surface is nearest in height.
    pub fn locate_in(&self, island: IslandId, p: IVec3) -> Option<u16> {
        let isl = &self.islands[island as usize];
        let mut best: Option<(i64, u16)> = None;
        for (pi, poly) in isl.asset.polys.iter().enumerate() {
            let v: Vec<IVec3> = poly.verts.iter().map(|&k| isl.vertices[k as usize]).collect();
            let n = v.len();
            if !(0..n).all(|k| ccw_from_above(v[k], v[(k + 1) % n], p) >= 0) {
                continue;
            }
            let h = surface_height(&v, p);
            let dy = (h - p.y as i64).abs();
            if best.is_none_or(|b| dy < b.0) {
                best = Some((dy, pi as u16));
            }
        }
        best.map(|b| b.1)
    }

    /// The polygon anywhere in the world under or over `p` nearest in height, if any.
    pub fn locate(&self, p: IVec3) -> Option<PolyRef> {
        let mut best: Option<(i64, PolyRef)> = None;
        for (ii, isl) in self.islands.iter().enumerate() {
            if p.x < isl.min.x || p.x > isl.max.x || p.z < isl.min.z || p.z > isl.max.z {
                continue;
            }
            if let Some(poly) = self.locate_in(ii as IslandId, p) {
                let v: Vec<IVec3> = self.poly_vertices(PolyRef { island: ii as u32, poly }).collect();
                let dy = (surface_height(&v, p) - p.y as i64).abs();
                if best.is_none_or(|b| dy < b.0) {
                    best = Some((dy, PolyRef { island: ii as u32, poly }));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Joins two doorways the level says meet, building the portal from their socket edges.
    /// `door` puts a gate on it.
    pub fn connect(&mut self, a: SocketRef, b: SocketRef, door: Option<&str>) -> Result<ConnectionId, JoinError> {
        let sa = self.socket(a).ok_or(JoinError::NoSuchSocket(a))?.clone();
        let sb = self.socket(b).ok_or(JoinError::NoSuchSocket(b))?.clone();
        for (r, s) in [(a, &sa), (b, &sb)] {
            if s.connection.is_some() {
                return Err(JoinError::AlreadyJoined(r));
            }
        }
        if sa.outward != sb.outward.opposite() {
            return Err(JoinError::NotFacing { a: sa.outward, b: sb.outward });
        }
        if sa.size_class != sb.size_class {
            return Err(JoinError::SizeClass(sa.size_class, sb.size_class));
        }
        let axis = axis_of(sa.outward);
        let sign = sa.outward.vec()[axis];
        let gap_mm = (sb.a[axis] - sa.a[axis]) * sign;
        if gap_mm.abs() > JOIN_TOLERANCE_MM {
            return Err(JoinError::TooFar { gap_mm, tolerance_mm: JOIN_TOLERANCE_MM });
        }
        let left = left_of(sa.outward);
        let span = |s: &WorldSocket| (s.a.dot(left).min(s.b.dot(left)), s.a.dot(left).max(s.b.dot(left)));
        let ((a0, a1), (b0, b1)) = (span(&sa), span(&sb));
        let lateral_mm = a0.max(b0) - a1.min(b1);
        if lateral_mm > JOIN_TOLERANCE_MM {
            return Err(JoinError::Misaligned { lateral_mm });
        }
        let dy = sb.a.y - sa.a.y;
        let climb = self.islands[a.island as usize].asset.agent.climb_mm.min(self.islands[b.island as usize].asset.agent.climb_mm);
        if dy.abs() > climb {
            return Err(JoinError::Step { dy_mm: dy });
        }

        let id = self.connections.len() as ConnectionId;
        let gate = door.map(|name| {
            self.gates.push(Gate { name: name.to_string(), open: true, links: Vec::new() });
            (self.gates.len() - 1) as GateId
        });
        let edges_a = self.socket_edges(a);
        let edges_b = self.socket_edges(b);
        let mut links = Vec::new();
        for &(pa, ea, a0, a1) in &edges_a {
            for &(pb, eb, b0, b1) in &edges_b {
                let (la0, la1) = (a0.dot(left), a1.dot(left));
                let (lb0, lb1) = (b0.dot(left), b1.dot(left));
                let lo = la0.min(la1).max(lb0.min(lb1));
                let hi = la0.max(la1).min(lb0.max(lb1));
                if hi - lo < 1 {
                    continue;
                }
                let on = |p: IVec3, l: i32| p - left * p.dot(left) + left * l;
                let seg_a = [on(a0, hi), on(a0, lo)];
                let seg_b = [on(b0, hi), on(b0, lo)];
                let (from, to) = (PolyRef { island: a.island, poly: pa }, PolyRef { island: b.island, poly: pb });
                let ab = self.push_link(Link { kind: LinkKind::Portal, from, to, from_segment: seg_a, to_segment: seg_b, cost_mm: 0, enabled: true, gate, connection: Some(id), from_edge: Some(ea) });
                let ba = self.push_link(Link {
                    kind: LinkKind::Portal,
                    from: to,
                    to: from,
                    from_segment: [seg_b[1], seg_b[0]],
                    to_segment: [seg_a[1], seg_a[0]],
                    cost_mm: 0,
                    enabled: true,
                    gate,
                    connection: Some(id),
                    from_edge: Some(eb),
                });
                links.extend([ab, ba]);
            }
        }
        if let Some(g) = gate {
            self.gates[g as usize].links = links.clone();
        }
        let midpoint = (sa.a + sa.b + sb.a + sb.b) / 4;
        self.connections.push(Connection { a, b, links, gate, gap_mm, midpoint });
        self.islands[a.island as usize].sockets[a.socket as usize].connection = Some(id);
        self.islands[b.island as usize].sockets[b.socket as usize].connection = Some(id);
        self.components = None;
        Ok(id)
    }

    pub fn socket(&self, s: SocketRef) -> Option<&WorldSocket> {
        self.islands.get(s.island as usize)?.sockets.get(s.socket as usize)
    }

    /// A socket's polygon edges in world space: (polygon, edge, start, end).
    fn socket_edges(&self, s: SocketRef) -> Vec<(u16, u8, IVec3, IVec3)> {
        let isl = &self.islands[s.island as usize];
        isl.asset.sockets[s.socket as usize]
            .edges
            .iter()
            .map(|&(p, e)| {
                let poly = &isl.asset.polys[p as usize];
                let n = poly.verts.len();
                (p, e, isl.vertices[poly.verts[e as usize] as usize], isl.vertices[poly.verts[(e as usize + 1) % n] as usize])
            })
            .collect()
    }

    /// Opens or closes a door. Returns the event when it changed anything.
    pub fn set_gate(&mut self, gate: GateId, open: bool) -> Option<NavEvent> {
        let g = &mut self.gates[gate as usize];
        if g.open == open {
            return None;
        }
        g.open = open;
        for &l in &g.links.clone() {
            self.links[l as usize].enabled = open;
        }
        self.version += 1;
        self.components = None;
        let ev = NavEvent::GateChanged { gate, open, version: self.version };
        self.events.push(ev.clone());
        Some(ev)
    }

    /// Enables or disables one typed link (a vent grate, a lowered ladder).
    pub fn set_link_enabled(&mut self, link: LinkId, enabled: bool) -> Option<NavEvent> {
        let l = &mut self.links[link as usize];
        if l.enabled == enabled {
            return None;
        }
        l.enabled = enabled;
        self.version += 1;
        self.components = None;
        let ev = NavEvent::LinkChanged { link, enabled, version: self.version };
        self.events.push(ev.clone());
        Some(ev)
    }

    /// Every way out of `p`: into neighbouring polygons of its own piece, and over enabled links.
    pub fn neighbours(&self, p: PolyRef) -> impl Iterator<Item = (PolyRef, Option<LinkId>)> + '_ {
        let isl = &self.islands[p.island as usize];
        let internal = isl.asset.polys[p.poly as usize].edges.iter().filter_map(move |e| match *e {
            EdgeLink::Internal(o) => Some((PolyRef { island: p.island, poly: o }, None)),
            _ => None,
        });
        let links = isl.links_from[p.poly as usize].iter().filter(move |&&l| self.links[l as usize].enabled).map(move |&l| (self.links[l as usize].to, Some(l)));
        internal.chain(links)
    }

    /// A dense index for every polygon in the world, for per-polygon tables.
    pub fn poly_index(&self, p: PolyRef) -> usize {
        (self.island_base[p.island as usize] + p.poly as u32) as usize
    }

    /// Which connected piece of the world each polygon is in, counting enabled links both ways
    /// (a one-way drop joins its two ends here; path search is what respects direction).
    /// Recomputed only after something changed.
    pub fn component(&mut self, p: PolyRef) -> u32 {
        if self.components.is_none() {
            self.components = Some(self.compute_components());
        }
        let g = self.poly_index(p);
        self.components.as_ref().unwrap()[g]
    }

    pub fn same_component(&mut self, a: PolyRef, b: PolyRef) -> bool {
        self.component(a) == self.component(b)
    }

    fn compute_components(&self) -> Vec<u32> {
        let n = self.poly_count();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        let union = |parent: &mut Vec<u32>, a: usize, b: usize| {
            let (ra, rb) = (find(parent, a as u32), find(parent, b as u32));
            if ra != rb {
                let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                parent[hi as usize] = lo;
            }
        };
        for (ii, isl) in self.islands.iter().enumerate() {
            for (pi, poly) in isl.asset.polys.iter().enumerate() {
                for e in &poly.edges {
                    if let EdgeLink::Internal(o) = *e {
                        let base = self.island_base[ii] as usize;
                        union(&mut parent, base + pi, base + o as usize);
                    }
                }
            }
        }
        for l in self.links.iter().filter(|l| l.enabled) {
            union(&mut parent, self.poly_index(l.from), self.poly_index(l.to));
        }
        (0..n as u32).map(|x| find(&mut parent, x)).collect()
    }

    /// Checks the level's claims about its doorways against the geometry.
    pub fn check_joins(&self) -> Vec<JoinIssue> {
        let mut issues = Vec::new();
        let mut open: Vec<SocketRef> = Vec::new();
        for (ii, isl) in self.islands.iter().enumerate() {
            for (si, s) in isl.sockets.iter().enumerate() {
                if s.connection.is_none() {
                    open.push(SocketRef { island: ii as u32, socket: si as u16 });
                }
            }
        }
        let mut matched = vec![false; open.len()];
        for i in 0..open.len() {
            for j in i + 1..open.len() {
                let (a, b) = (self.socket(open[i]).unwrap(), self.socket(open[j]).unwrap());
                if a.outward != b.outward.opposite() || a.size_class != b.size_class {
                    continue;
                }
                let (ma, mb) = ((a.a + a.b) / 2, (b.a + b.b) / 2);
                let d = (ma - mb).abs();
                if d.x.max(d.z) <= JOIN_TOLERANCE_MM && d.y.abs() <= JOIN_TOLERANCE_MM {
                    issues.push(JoinIssue::MissedJoin(open[i], open[j]));
                    matched[i] = true;
                    matched[j] = true;
                }
            }
        }
        for (k, s) in open.iter().enumerate() {
            if !matched[k] {
                issues.push(JoinIssue::Unjoined(*s));
            }
        }
        for (ci, c) in self.connections.iter().enumerate() {
            if c.links.is_empty() {
                issues.push(JoinIssue::Impassable(ci as ConnectionId));
            }
        }
        issues
    }

    /// The room graph: pieces as nodes, joins as edges, with the real walking distance across each
    /// room between each pair of its doorways. See [`RoomGraph`].
    pub fn room_graph(&self) -> RoomGraph {
        let mut doorways: Vec<Vec<ConnectionId>> = vec![Vec::new(); self.islands.len()];
        for (ci, c) in self.connections.iter().enumerate() {
            if !c.links.is_empty() {
                doorways[c.a.island as usize].push(ci as ConnectionId);
                doorways[c.b.island as usize].push(ci as ConnectionId);
            }
        }
        let mut tables: BTreeMap<usize, Arc<CrossingTable>> = BTreeMap::new();
        let crossing = self
            .islands
            .iter()
            .map(|isl| tables.entry(Arc::as_ptr(&isl.asset) as usize).or_insert_with(|| Arc::new(crossing_table(&isl.asset))).clone())
            .collect();
        RoomGraph { doorways, crossing }
    }
}

/// Walking distance across one piece between each pair of its doorways, millimetres: `None` when
/// one cannot walk from one to the other inside the piece (a catwalk doorway, a vent room's two
/// halves).
pub type CrossingTable = Vec<Vec<Option<u32>>>;

fn crossing_table(asset: &Arc<NavPieceAsset>) -> CrossingTable {
    let mut w = NavWorld::new();
    w.add_island(asset.clone(), Turn(0), IVec3::ZERO);
    let n = asset.sockets.len();
    let mid: Vec<Option<glam::DVec3>> =
        asset.sockets.iter().map(|s| (!s.edges.is_empty()).then(|| ((s.a + s.b) / 2).as_dvec3())).collect();
    let ext = glam::DVec3::new(200.0, 500.0, 200.0);
    let mut t = vec![vec![None; n]; n];
    for i in 0..n {
        t[i][i] = mid[i].map(|_| 0);
        for j in i + 1..n {
            if let (Some(a), Some(b)) = (mid[i], mid[j]) {
                if let Some(p) = crate::path::find_path(&w, a, b, ext).filter(|p| p.complete) {
                    let d = p.length().round() as u32;
                    t[i][j] = Some(d);
                    t[j][i] = Some(d);
                }
            }
        }
    }
    t
}

/// Height of a polygon's own surface (its fan from vertex 0) under `p`, millimetres.
pub fn surface_height(v: &[IVec3], p: IVec3) -> i64 {
    for i in 1..v.len() - 1 {
        let (a, b, c) = (v[0], v[i], v[i + 1]);
        let det = ccw_from_above(a, b, c);
        if det == 0 {
            continue;
        }
        let wb = ccw_from_above(a, p, c);
        let wc = ccw_from_above(a, b, p);
        if wb >= 0 && wc >= 0 && wb + wc <= det {
            let num = a.y as i64 * det + (b.y - a.y) as i64 * wb + (c.y - a.y) as i64 * wc;
            return num / det;
        }
    }
    v.iter().map(|q| q.y as i64).sum::<i64>() / v.len() as i64
}

/// Rooms and the doorways between them: the coarse map for long routes and for "which way did the
/// player go". Built from the world at load; passability is read from the world's gates live.
pub struct RoomGraph {
    /// Per island: the joins it takes part in.
    pub doorways: Vec<Vec<ConnectionId>>,
    /// Per island: walking distances between its doorways, by socket.
    pub crossing: Vec<Arc<CrossingTable>>,
}

impl RoomGraph {
    /// Which of `room`'s sockets connection `c` uses.
    fn socket_in(world: &NavWorld, c: ConnectionId, room: IslandId) -> usize {
        let conn = &world.connections[c as usize];
        if conn.a.island == room { conn.a.socket as usize } else { conn.b.socket as usize }
    }

    fn open(world: &NavWorld, c: ConnectionId) -> bool {
        world.connections[c as usize].gate.is_none_or(|g| world.gates[g as usize].open)
    }

    /// Dijkstra over doorways from `seeds` (doorway, starting distance); returns each doorway's
    /// distance and the doorway it was reached from.
    fn search(&self, world: &NavWorld, seeds: &[(ConnectionId, u64)]) -> (Vec<u64>, Vec<Option<ConnectionId>>) {
        let n = world.connections.len();
        let mut dist = vec![u64::MAX; n];
        let mut prev = vec![None; n];
        let mut heap = BinaryHeap::new();
        for &(c, d) in seeds {
            if Self::open(world, c) && d < dist[c as usize] {
                dist[c as usize] = d;
                heap.push(std::cmp::Reverse((d, c)));
            }
        }
        while let Some(std::cmp::Reverse((d, c))) = heap.pop() {
            if d > dist[c as usize] {
                continue;
            }
            let conn = &world.connections[c as usize];
            for room in [conn.a.island, conn.b.island] {
                let from = Self::socket_in(world, c, room);
                for &next in &self.doorways[room as usize] {
                    if next == c || !Self::open(world, next) {
                        continue;
                    }
                    let Some(cross) = self.crossing[room as usize][from][Self::socket_in(world, next, room)] else { continue };
                    let nd = d + cross as u64;
                    if nd < dist[next as usize] {
                        dist[next as usize] = nd;
                        prev[next as usize] = Some(c);
                        heap.push(std::cmp::Reverse((nd, next)));
                    }
                }
            }
        }
        (dist, prev)
    }

    /// The shortest sequence of doorways, through open doors, from any doorway of `from` to
    /// `to`, with the walking distance between the first and last of them. `None` when no open
    /// route exists.
    pub fn route(&self, world: &NavWorld, from: IslandId, to: IslandId) -> Option<(Vec<ConnectionId>, u64)> {
        if from == to {
            return Some((Vec::new(), 0));
        }
        let seeds: Vec<(ConnectionId, u64)> = self.doorways[from as usize].iter().map(|&c| (c, 0)).collect();
        let (dist, prev) = self.search(world, &seeds);
        let end = self.doorways[to as usize].iter().copied().filter(|&c| dist[c as usize] < u64::MAX).min_by_key(|&c| (dist[c as usize], c))?;
        let mut path = vec![end];
        let mut at = end;
        while let Some(p) = prev[at as usize] {
            path.push(p);
            at = p;
        }
        path.reverse();
        Some((path, dist[end as usize]))
    }

    /// The best chain of doorways between two points, given each point's walking distance to the
    /// doorways of its own room: `from` and `to` as (doorway, distance). Returns the doorways in
    /// order and the total.
    pub fn route_between(&self, world: &NavWorld, from: &[(ConnectionId, u64)], to: &[(ConnectionId, u64)]) -> Option<(Vec<ConnectionId>, u64)> {
        let (dist, prev) = self.search(world, from);
        let (end, total) = to.iter().filter(|(c, _)| dist[*c as usize] < u64::MAX).map(|&(c, d)| (c, dist[c as usize] + d)).min_by_key(|&(c, t)| (t, c))?;
        let mut path = vec![end];
        let mut at = end;
        while let Some(p) = prev[at as usize] {
            path.push(p);
            at = p;
        }
        path.reverse();
        Some((path, total))
    }

    /// Walking distance from doorway `c` to every doorway, through open doors (`u64::MAX`: none).
    pub fn from_doorway(&self, world: &NavWorld, c: ConnectionId) -> Vec<u64> {
        self.search(world, &[(c, 0)]).0
    }

    /// A distance field towards room `target`: for every doorway, how far it is to walk from it
    /// into `target` through open doors (`u64::MAX`: no way). Compute once when the target (say a
    /// player) changes room, then any monster reads [`RoomGraph::toward`].
    pub fn field_to(&self, world: &NavWorld, target: IslandId) -> Vec<u64> {
        let seeds: Vec<(ConnectionId, u64)> = self.doorways[target as usize].iter().map(|&c| (c, 0)).collect();
        self.search(world, &seeds).0
    }

    /// The doorway out of `room` that leads soonest to the field's target, if any does.
    pub fn toward(&self, world: &NavWorld, field: &[u64], room: IslandId) -> Option<ConnectionId> {
        self.doorways[room as usize].iter().copied().filter(|&c| Self::open(world, c) && field[c as usize] < u64::MAX).min_by_key(|&c| (field[c as usize], c))
    }
}
