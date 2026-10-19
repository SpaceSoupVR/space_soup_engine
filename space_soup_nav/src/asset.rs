//! The baked navigation of one room piece: the unit that is built offline, shipped as data, and
//! placed (turned and moved) at load.
//!
//! WHY INTEGERS. Vertices are millimetres as `i32` in the piece's own space. A placed piece is
//! turned in quarter turns, and a quarter turn of an integer point is a swap and a negation, so
//! the two sides of a doorway land on exactly the same integers whichever way each piece faces.
//! That is what lets sockets be paired by identity instead of by a distance tolerance, and what
//! makes an assembled level the same bytes on every headset. See DETERMINISM.md.
//!
//! WHY THE LOOKUP TREE IS IN THE ASSET. A navmesh needs a bounding-volume tree to find the
//! polygon under a point. Building it at load cost a desktop tile load ~90 ms in a reported case;
//! on the Quest that is a hitch. It is built offline by [`build_bvh`] and shipped.
//!
//! WINDING. Polygons are convex and counter-clockwise viewed from above (+Y down onto the floor).
//! [`NavPieceAsset::validate`] checks it, so a baker that emits the other winding fails loudly.

use glam::IVec3;
use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 8] = b"SSNAVPC\0";
pub const VERSION: u32 = 1;
/// Most vertices a polygon may have. Recast and Detour use 6.
pub const MAX_POLY_VERTS: usize = 6;
/// The farthest any coordinate in a piece may be from its origin: 16.7 km, so placing and turning
/// a piece anywhere in a level stays far inside an `i32`.
pub const MAX_COORD_MM: i32 = 1 << 24;

/// The agent a piece was baked for. A navmesh is shrunk by the agent's radius, so each size class
/// gets its own bake; mixing them would let a large monster plan through a gap it cannot fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentClass {
    pub radius_mm: i32,
    pub height_mm: i32,
    /// Highest step it can walk up.
    pub climb_mm: i32,
    /// Steepest walkable slope, hundredths of a degree.
    pub max_slope_cdeg: u16,
}

/// A quarter-turn about +Y, the only rotation a placed piece may have. Same sense as
/// `glam::Quat::from_rotation_y(k * 90 degrees)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Turn(pub u8);

impl Turn {
    pub fn apply(self, p: IVec3) -> IVec3 {
        match self.0 % 4 {
            0 => p,
            1 => IVec3::new(p.z, p.y, -p.x),
            2 => IVec3::new(-p.x, p.y, -p.z),
            _ => IVec3::new(-p.z, p.y, p.x),
        }
    }

    pub fn apply_dir(self, d: Dir) -> Dir {
        let v = self.apply(d.vec());
        Dir::from_vec(v).expect("a turned axis is an axis")
    }
}

/// The four horizontal axes a doorway can face out along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Dir {
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl Dir {
    pub fn vec(self) -> IVec3 {
        match self {
            Dir::PosX => IVec3::X,
            Dir::NegX => IVec3::NEG_X,
            Dir::PosZ => IVec3::Z,
            Dir::NegZ => IVec3::NEG_Z,
        }
    }

    pub fn from_vec(v: IVec3) -> Option<Dir> {
        match (v.x, v.y, v.z) {
            (1, 0, 0) => Some(Dir::PosX),
            (-1, 0, 0) => Some(Dir::NegX),
            (0, 0, 1) => Some(Dir::PosZ),
            (0, 0, -1) => Some(Dir::NegZ),
            _ => None,
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::PosX => Dir::NegX,
            Dir::NegX => Dir::PosX,
            Dir::PosZ => Dir::NegZ,
            Dir::NegZ => Dir::PosZ,
        }
    }
}

/// What lies across one edge of a polygon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeLink {
    /// A wall or a drop: nothing.
    Border,
    /// Another polygon of this piece, by index.
    Internal(u16),
    /// A doorway socket of this piece, by index into `sockets`: joined to another piece at load.
    Socket(u16),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavPoly {
    /// Indices into `vertices`, counter-clockwise from above, 3 to `MAX_POLY_VERTS`.
    pub verts: Vec<u16>,
    /// One per edge: edge `i` runs from `verts[i]` to `verts[(i + 1) % n]`.
    pub edges: Vec<EdgeLink>,
    /// Area type (floor, water, ...), for per-agent costs.
    pub area: u8,
    /// Game-defined flags, for query filters.
    pub flags: u16,
}

/// The height detail of one polygon: extra vertices and triangles so a point's height on a ramp or
/// stair is accurate, not just the polygon's flat approximation. Empty for a flat floor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailMesh {
    /// Per polygon: (first detail vertex, count, first triangle, count).
    pub ranges: Vec<(u32, u16, u32, u16)>,
    pub vertices: Vec<IVec3>,
    /// Indices: below the polygon's vertex count they name polygon vertices, at or above it detail
    /// vertices of this polygon's range. Detour's scheme.
    pub triangles: Vec<[u8; 3]>,
}

/// A node of the polygon lookup tree. `index >= 0` is a leaf naming a polygon; a negative index is
/// an inner node whose subtree ends at node `-index` (the escape index, Detour's layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BvNode {
    pub min: IVec3,
    pub max: IVec3,
    pub index: i32,
}

/// A doorway: the edge on the socket plane where this piece meets a neighbour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Socket {
    /// The marker it came from.
    pub name: String,
    /// The doorway edge's two ends on the socket plane, in piece space.
    pub a: IVec3,
    pub b: IVec3,
    /// Which way the doorway faces out of the piece.
    pub outward: Dir,
    /// Only sockets of the same size class are joined.
    pub size_class: String,
    /// The polygon edges lying along the doorway edge: (polygon, edge index).
    pub edges: Vec<(u16, u8)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LinkKind {
    Door,
    Vent,
    Ladder,
    Drop,
    Shaft,
    Custom(String),
}

/// A way across a gap the navmesh does not cover: a vent, a ladder, a drop. Joins two segments, so
/// an agent can enter anywhere along one and leave anywhere along the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavLink {
    pub kind: LinkKind,
    pub start: [IVec3; 2],
    pub end: [IVec3; 2],
    pub bidirectional: bool,
    /// Extra cost of taking it, in millimetres of equivalent walking.
    pub cost_mm: u32,
    /// Off means the link is excluded from search, not merely expensive. (A "disabled" link given
    /// the maximum cost was still taken when it was the only route; see the research report.)
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavPieceAsset {
    /// The room piece's name, e.g. its glb's.
    pub name: String,
    pub agent: AgentClass,
    /// The scale variant baked, per axis, in ten-thousandths (10000 = 1.0). A navmesh cannot be
    /// scaled after baking: clearance, step height and slope would all be wrong.
    pub scale_e4: [u32; 3],
    pub vertices: Vec<IVec3>,
    pub polys: Vec<NavPoly>,
    pub detail: DetailMesh,
    pub bvh: Vec<BvNode>,
    pub sockets: Vec<Socket>,
    pub links: Vec<NavLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    BadMagic,
    Version(u32),
    Truncated,
    BadUtf8,
    BadTag(u8),
    HashMismatch { stored: u64, computed: u64 },
    TrailingBytes(usize),
    Invalid(String),
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for AssetError {}

/// FNV-1a, 64-bit. Not cryptographic; it only has to notice two headsets holding different data.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl NavPieceAsset {
    /// The canonical bytes: magic, version, body, then the FNV-1a hash of everything before it.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(MAGIC);
        w.u32(VERSION);
        self.write_body(&mut w);
        let hash = fnv1a64(&w.out);
        w.u64(hash);
        w.out
    }

    /// The hash stored in `to_bytes`, identifying this exact data.
    pub fn content_hash(&self) -> u64 {
        let bytes = self.to_bytes();
        u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().expect("8 bytes"))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<NavPieceAsset, AssetError> {
        if bytes.len() < MAGIC.len() + 4 + 8 {
            return Err(AssetError::Truncated);
        }
        let (body, tail) = bytes.split_at(bytes.len() - 8);
        let stored = u64::from_le_bytes(tail.try_into().expect("8 bytes"));
        let computed = fnv1a64(body);
        if stored != computed {
            return Err(AssetError::HashMismatch { stored, computed });
        }
        let mut r = Reader { data: body, at: 0 };
        if r.take(MAGIC.len())? != MAGIC {
            return Err(AssetError::BadMagic);
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(AssetError::Version(version));
        }
        let asset = Self::read_body(&mut r)?;
        if r.at != body.len() {
            return Err(AssetError::TrailingBytes(body.len() - r.at));
        }
        asset.validate().map_err(AssetError::Invalid)?;
        Ok(asset)
    }

    fn write_body(&self, w: &mut Writer) {
        w.str(&self.name);
        w.i32(self.agent.radius_mm);
        w.i32(self.agent.height_mm);
        w.i32(self.agent.climb_mm);
        w.u16(self.agent.max_slope_cdeg);
        for s in self.scale_e4 {
            w.u32(s);
        }
        w.len(self.vertices.len());
        for v in &self.vertices {
            w.ivec3(*v);
        }
        w.len(self.polys.len());
        for p in &self.polys {
            w.len(p.verts.len());
            for &i in &p.verts {
                w.u16(i);
            }
            for e in &p.edges {
                match *e {
                    EdgeLink::Border => w.u8(0),
                    EdgeLink::Internal(i) => {
                        w.u8(1);
                        w.u16(i);
                    }
                    EdgeLink::Socket(i) => {
                        w.u8(2);
                        w.u16(i);
                    }
                }
            }
            w.u8(p.area);
            w.u16(p.flags);
        }
        w.len(self.detail.ranges.len());
        for &(vb, vc, tb, tc) in &self.detail.ranges {
            w.u32(vb);
            w.u16(vc);
            w.u32(tb);
            w.u16(tc);
        }
        w.len(self.detail.vertices.len());
        for v in &self.detail.vertices {
            w.ivec3(*v);
        }
        w.len(self.detail.triangles.len());
        for t in &self.detail.triangles {
            w.bytes(t);
        }
        w.len(self.bvh.len());
        for n in &self.bvh {
            w.ivec3(n.min);
            w.ivec3(n.max);
            w.i32(n.index);
        }
        w.len(self.sockets.len());
        for s in &self.sockets {
            w.str(&s.name);
            w.ivec3(s.a);
            w.ivec3(s.b);
            w.u8(dir_tag(s.outward));
            w.str(&s.size_class);
            w.len(s.edges.len());
            for &(p, e) in &s.edges {
                w.u16(p);
                w.u8(e);
            }
        }
        w.len(self.links.len());
        for l in &self.links {
            match &l.kind {
                LinkKind::Door => w.u8(0),
                LinkKind::Vent => w.u8(1),
                LinkKind::Ladder => w.u8(2),
                LinkKind::Drop => w.u8(3),
                LinkKind::Shaft => w.u8(4),
                LinkKind::Custom(name) => {
                    w.u8(5);
                    w.str(name);
                }
            }
            for p in l.start.iter().chain(l.end.iter()) {
                w.ivec3(*p);
            }
            w.u8(l.bidirectional as u8);
            w.u32(l.cost_mm);
            w.u8(l.enabled as u8);
        }
    }

    fn read_body(r: &mut Reader) -> Result<NavPieceAsset, AssetError> {
        let name = r.str()?;
        let agent = AgentClass { radius_mm: r.i32()?, height_mm: r.i32()?, climb_mm: r.i32()?, max_slope_cdeg: r.u16()? };
        let scale_e4 = [r.u32()?, r.u32()?, r.u32()?];
        let vertices = (0..r.len()?).map(|_| r.ivec3()).collect::<Result<_, _>>()?;
        let mut polys = Vec::new();
        for _ in 0..r.len()? {
            let n = r.len()?;
            let verts = (0..n).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
            let mut edges = Vec::with_capacity(n);
            for _ in 0..n {
                edges.push(match r.u8()? {
                    0 => EdgeLink::Border,
                    1 => EdgeLink::Internal(r.u16()?),
                    2 => EdgeLink::Socket(r.u16()?),
                    t => return Err(AssetError::BadTag(t)),
                });
            }
            polys.push(NavPoly { verts, edges, area: r.u8()?, flags: r.u16()? });
        }
        let ranges = (0..r.len()?).map(|_| Ok((r.u32()?, r.u16()?, r.u32()?, r.u16()?))).collect::<Result<_, AssetError>>()?;
        let dverts = (0..r.len()?).map(|_| r.ivec3()).collect::<Result<_, _>>()?;
        let tris = (0..r.len()?)
            .map(|_| {
                let t = r.take(3)?;
                Ok([t[0], t[1], t[2]])
            })
            .collect::<Result<_, AssetError>>()?;
        let bvh = (0..r.len()?).map(|_| Ok(BvNode { min: r.ivec3()?, max: r.ivec3()?, index: r.i32()? })).collect::<Result<_, AssetError>>()?;
        let mut sockets = Vec::new();
        for _ in 0..r.len()? {
            let name = r.str()?;
            let a = r.ivec3()?;
            let b = r.ivec3()?;
            let outward = dir_from_tag(r.u8()?)?;
            let size_class = r.str()?;
            let edges = (0..r.len()?).map(|_| Ok((r.u16()?, r.u8()?))).collect::<Result<_, AssetError>>()?;
            sockets.push(Socket { name, a, b, outward, size_class, edges });
        }
        let mut links = Vec::new();
        for _ in 0..r.len()? {
            let kind = match r.u8()? {
                0 => LinkKind::Door,
                1 => LinkKind::Vent,
                2 => LinkKind::Ladder,
                3 => LinkKind::Drop,
                4 => LinkKind::Shaft,
                5 => LinkKind::Custom(r.str()?),
                t => return Err(AssetError::BadTag(t)),
            };
            let start = [r.ivec3()?, r.ivec3()?];
            let end = [r.ivec3()?, r.ivec3()?];
            links.push(NavLink { kind, start, end, bidirectional: r.u8()? != 0, cost_mm: r.u32()?, enabled: r.u8()? != 0 });
        }
        Ok(NavPieceAsset {
            name,
            agent,
            scale_e4,
            vertices,
            polys,
            detail: DetailMesh { ranges, vertices: dverts, triangles: tris },
            bvh,
            sockets,
            links,
        })
    }

    /// Every structural rule a baked piece must satisfy. Checked on load, so a bad bake is caught
    /// there and not as an agent walking through a wall.
    pub fn validate(&self) -> Result<(), String> {
        let far = |v: &IVec3| v.to_array().iter().any(|c| c.unsigned_abs() > MAX_COORD_MM as u32);
        if self.vertices.iter().chain(&self.detail.vertices).any(far)
            || self.sockets.iter().any(|s| far(&s.a) || far(&s.b))
            || self.links.iter().any(|l| l.start.iter().chain(&l.end).any(far))
            || self.bvh.iter().any(|n| far(&n.min) || far(&n.max))
        {
            return Err(format!("a coordinate is more than {MAX_COORD_MM} mm from the piece's origin"));
        }
        let nv = self.vertices.len();
        let np = self.polys.len();
        if np > u16::MAX as usize || nv > u16::MAX as usize {
            return Err(format!("{np} polygons / {nv} vertices: more than a u16 index can name"));
        }
        for (pi, p) in self.polys.iter().enumerate() {
            let n = p.verts.len();
            if !(3..=MAX_POLY_VERTS).contains(&n) {
                return Err(format!("polygon {pi} has {n} vertices"));
            }
            if p.edges.len() != n {
                return Err(format!("polygon {pi} has {n} vertices but {} edges", p.edges.len()));
            }
            if let Some(&bad) = p.verts.iter().find(|&&v| v as usize >= nv) {
                return Err(format!("polygon {pi} names vertex {bad} of {nv}"));
            }
            for (k, e) in p.edges.iter().enumerate() {
                match *e {
                    EdgeLink::Internal(o) if o as usize >= np || o as usize == pi => {
                        return Err(format!("polygon {pi} edge {k} links to polygon {o}"))
                    }
                    EdgeLink::Socket(s) if s as usize >= self.sockets.len() => {
                        return Err(format!("polygon {pi} edge {k} names socket {s}"))
                    }
                    _ => {}
                }
            }
            let mut twice_area = 0i64;
            for k in 0..n {
                let a = self.vertices[p.verts[k] as usize];
                let b = self.vertices[p.verts[(k + 1) % n] as usize];
                let c = self.vertices[p.verts[(k + 2) % n] as usize];
                if ccw_from_above(a, b, c) < 0 {
                    return Err(format!("polygon {pi} is not convex and counter-clockwise from above at vertex {}", (k + 1) % n));
                }
                twice_area += ccw_from_above(self.vertices[p.verts[0] as usize], b, c);
            }
            if twice_area <= 0 {
                return Err(format!("polygon {pi} has no area"));
            }
        }
        for (pi, p) in self.polys.iter().enumerate() {
            for e in &p.edges {
                if let EdgeLink::Internal(o) = *e {
                    let back = self.polys[o as usize].edges.contains(&EdgeLink::Internal(pi as u16));
                    if !back {
                        return Err(format!("polygon {pi} links to {o}, which does not link back"));
                    }
                }
            }
        }
        for (si, s) in self.sockets.iter().enumerate() {
            for &(p, e) in &s.edges {
                let ok = self.polys.get(p as usize).and_then(|poly| poly.edges.get(e as usize)) == Some(&EdgeLink::Socket(si as u16));
                if !ok {
                    return Err(format!("socket {si} lists polygon {p} edge {e}, which is not marked as that socket"));
                }
            }
        }
        if !self.bvh.is_empty() {
            let leaves = self.bvh.iter().filter(|n| n.index >= 0).count();
            if leaves != np {
                return Err(format!("lookup tree has {leaves} leaves for {np} polygons"));
            }
            let mut seen = vec![false; np];
            for (i, n) in self.bvh.iter().enumerate() {
                if n.index >= 0 {
                    match seen.get_mut(n.index as usize) {
                        Some(s) if !*s => *s = true,
                        _ => return Err(format!("lookup tree node {i} names polygon {} twice or out of range", n.index)),
                    }
                } else if i as u64 + n.index.unsigned_abs() as u64 > self.bvh.len() as u64 {
                    return Err(format!("lookup tree node {i} escapes to {} of {}", i as i64 - n.index as i64, self.bvh.len()));
                }
            }
        }
        let d = &self.detail;
        if !d.ranges.is_empty() && d.ranges.len() != np {
            return Err(format!("{} detail ranges for {np} polygons", d.ranges.len()));
        }
        for (pi, &(vb, vc, tb, tc)) in d.ranges.iter().enumerate() {
            if vb as u64 + vc as u64 > d.vertices.len() as u64 || tb as u64 + tc as u64 > d.triangles.len() as u64 {
                return Err(format!("polygon {pi}'s detail range is outside the detail mesh"));
            }
            let limit = self.polys[pi].verts.len() + vc as usize;
            if d.triangles[tb as usize..tb as usize + tc as usize].iter().flatten().any(|&k| k as usize >= limit) {
                return Err(format!("polygon {pi} has a detail triangle naming a vertex it does not have"));
            }
        }
        Ok(())
    }

    /// Bounds of polygon `p` in millimetres.
    /// A polygon's bounds, its height detail included (a stair's floor rises above its corners).
    pub fn poly_bounds(&self, p: usize) -> (IVec3, IVec3) {
        let mut min = IVec3::MAX;
        let mut max = IVec3::MIN;
        for &v in &self.polys[p].verts {
            let q = self.vertices[v as usize];
            min = min.min(q);
            max = max.max(q);
        }
        if let Some(&(first, count, _, _)) = self.detail.ranges.get(p) {
            for q in &self.detail.vertices[first as usize..first as usize + count as usize] {
                min = min.min(*q);
                max = max.max(*q);
            }
        }
        (min, max)
    }
}

/// Twice the signed area of triangle (a, b, c) projected onto the floor, positive when a -> b -> c
/// turns counter-clockwise as seen from above.
///
/// Looking down from +Y in a right-handed frame, +X points right and +Z points DOWN the view
/// (X x Y = Z, and Y points at the viewer), so the screen's "up" is -Z. The usual 2D cross product
/// in (right, up) = (x, -z) is therefore `abx * (-acz) - (-abz) * acx`. This is the y component of
/// (ab x ac), which is positive exactly when the turn is counter-clockwise from above.
pub fn ccw_from_above(a: IVec3, b: IVec3, c: IVec3) -> i64 {
    let (abx, abz) = (b.x as i64 - a.x as i64, b.z as i64 - a.z as i64);
    let (acx, acz) = (c.x as i64 - a.x as i64, c.z as i64 - a.z as i64);
    abz * acx - abx * acz
}

/// The polygon lookup tree, built offline. Detour's construction: split the polygons' boxes at the
/// median of the longest axis, recursively, ties broken by polygon index so the tree is the same
/// on every machine.
pub fn build_bvh(asset: &NavPieceAsset) -> Vec<BvNode> {
    let mut items: Vec<(usize, IVec3, IVec3)> = (0..asset.polys.len()).map(|p| {
        let (min, max) = asset.poly_bounds(p);
        (p, min, max)
    }).collect();
    let mut nodes = Vec::with_capacity(items.len() * 2);
    if !items.is_empty() {
        subdivide(&mut items[..], &mut nodes);
    }
    nodes
}

fn subdivide(items: &mut [(usize, IVec3, IVec3)], nodes: &mut Vec<BvNode>) {
    let at = nodes.len();
    let (min, max) = items.iter().fold((IVec3::MAX, IVec3::MIN), |(lo, hi), it| (lo.min(it.1), hi.max(it.2)));
    if items.len() == 1 {
        nodes.push(BvNode { min, max, index: items[0].0 as i32 });
        return;
    }
    nodes.push(BvNode { min, max, index: 0 });
    let extent = max - min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z { 0 } else if extent.y >= extent.z { 1 } else { 2 };
    items.sort_by(|a, b| {
        let ca = a.1[axis] as i64 + a.2[axis] as i64;
        let cb = b.1[axis] as i64 + b.2[axis] as i64;
        ca.cmp(&cb).then(a.0.cmp(&b.0))
    });
    let mid = items.len() / 2;
    let (left, right) = items.split_at_mut(mid);
    subdivide(left, nodes);
    subdivide(right, nodes);
    let escape = nodes.len() - at;
    nodes[at].index = -(escape as i32);
}

fn dir_tag(d: Dir) -> u8 {
    match d {
        Dir::PosX => 0,
        Dir::NegX => 1,
        Dir::PosZ => 2,
        Dir::NegZ => 3,
    }
}

fn dir_from_tag(t: u8) -> Result<Dir, AssetError> {
    Ok(match t {
        0 => Dir::PosX,
        1 => Dir::NegX,
        2 => Dir::PosZ,
        3 => Dir::NegZ,
        t => return Err(AssetError::BadTag(t)),
    })
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).expect("a length that fits in u32"));
    }
    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.bytes(s.as_bytes());
    }
    fn ivec3(&mut self, v: IVec3) {
        self.i32(v.x);
        self.i32(v.y);
        self.i32(v.z);
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], AssetError> {
        let end = self.at.checked_add(n).ok_or(AssetError::Truncated)?;
        let s = self.data.get(self.at..end).ok_or(AssetError::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, AssetError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, AssetError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2")))
    }
    fn u32(&mut self) -> Result<u32, AssetError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn i32(&mut self) -> Result<i32, AssetError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn len(&mut self) -> Result<usize, AssetError> {
        let n = self.u32()? as usize;
        if n > self.data.len() - self.at {
            return Err(AssetError::Truncated);
        }
        Ok(n)
    }
    fn str(&mut self) -> Result<String, AssetError> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| AssetError::BadUtf8)
    }
    fn ivec3(&mut self) -> Result<IVec3, AssetError> {
        Ok(IVec3::new(self.i32()?, self.i32()?, self.i32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 m x 4 m floor split into two quads, with a doorway socket on its +Z edge and a vent link.
    pub(crate) fn sample() -> NavPieceAsset {
        let v = |x: i32, z: i32| IVec3::new(x, 0, z);
        let vertices = vec![v(-2000, -2000), v(0, -2000), v(2000, -2000), v(2000, 2000), v(0, 2000), v(-2000, 2000)];
        let polys = vec![
            NavPoly { verts: vec![0, 5, 4, 1], edges: vec![EdgeLink::Border, EdgeLink::Socket(0), EdgeLink::Internal(1), EdgeLink::Border], area: 0, flags: 1 },
            NavPoly { verts: vec![1, 4, 3, 2], edges: vec![EdgeLink::Internal(0), EdgeLink::Socket(0), EdgeLink::Border, EdgeLink::Border], area: 0, flags: 1 },
        ];
        let mut asset = NavPieceAsset {
            name: "test_room".into(),
            agent: AgentClass { radius_mm: 400, height_mm: 1800, climb_mm: 300, max_slope_cdeg: 4500 },
            scale_e4: [10000; 3],
            vertices,
            polys,
            detail: DetailMesh::default(),
            bvh: Vec::new(),
            sockets: vec![Socket {
                name: "SDoorType_A".into(),
                a: v(-950, 2000),
                b: v(950, 2000),
                outward: Dir::PosZ,
                size_class: "default".into(),
                edges: vec![(0, 1), (1, 1)],
            }],
            links: vec![NavLink {
                kind: LinkKind::Vent,
                start: [v(-500, -1000), v(500, -1000)],
                end: [IVec3::new(-500, 3000, -1000), IVec3::new(500, 3000, -1000)],
                bidirectional: true,
                cost_mm: 5000,
                enabled: true,
            }],
        };
        asset.bvh = build_bvh(&asset);
        asset
    }

    #[test]
    fn counter_clockwise_from_above_means_what_it_says() {
        let o = IVec3::ZERO;
        let east = IVec3::new(1000, 0, 0);
        let north_east = IVec3::new(1000, 0, -1000);
        assert!(ccw_from_above(o, east, north_east) > 0);
        assert!(ccw_from_above(o, north_east, east) < 0);
        let g = (east - o).as_vec3().cross((north_east - o).as_vec3()).y as i64;
        assert_eq!(g, ccw_from_above(o, east, north_east));
    }

    #[test]
    fn the_sample_is_valid() {
        sample().validate().unwrap();
    }

    #[test]
    fn bytes_round_trip_exactly() {
        let a = sample();
        let bytes = a.to_bytes();
        let b = NavPieceAsset::from_bytes(&bytes).unwrap();
        assert_eq!(a, b);
        assert_eq!(b.to_bytes(), bytes, "re-serialising gives the same bytes");
    }

    #[test]
    fn the_hash_catches_any_changed_byte() {
        let bytes = sample().to_bytes();
        for i in [0, 9, bytes.len() / 2, bytes.len() - 9] {
            let mut bad = bytes.clone();
            bad[i] ^= 0x01;
            assert!(NavPieceAsset::from_bytes(&bad).is_err(), "flipping byte {i} went unnoticed");
        }
    }

    #[test]
    fn the_hash_is_pinned() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn validation_rejects_clockwise_and_broken_links() {
        let mut cw = sample();
        cw.polys[0].verts.reverse();
        assert!(cw.validate().unwrap_err().contains("counter-clockwise"));
        let mut one_way = sample();
        one_way.polys[1].edges[0] = EdgeLink::Border;
        assert!(one_way.validate().unwrap_err().contains("does not link back"));
        let mut bad_socket = sample();
        bad_socket.sockets[0].edges.push((0, 0));
        assert!(bad_socket.validate().is_err());
    }

    #[test]
    fn a_straight_through_vertex_is_allowed_but_a_flat_polygon_is_not() {
        let mut mid = sample();
        mid.vertices.push(IVec3::new(-2000, 0, 0));
        mid.polys[0].verts = vec![0, 6, 5, 4, 1];
        mid.polys[0].edges.insert(0, EdgeLink::Border);
        mid.sockets[0].edges[0].1 = 2;
        mid.bvh = build_bvh(&mid);
        assert_eq!(mid.validate(), Ok(()));

        let mut flat = sample();
        flat.polys.push(NavPoly { verts: vec![0, 1, 2], edges: vec![EdgeLink::Border; 3], area: 0, flags: 1 });
        flat.bvh = build_bvh(&flat);
        assert!(flat.validate().unwrap_err().contains("no area"));
    }

    #[test]
    fn truncated_or_padded_data_is_rejected() {
        let bytes = sample().to_bytes();
        assert!(NavPieceAsset::from_bytes(&bytes[..bytes.len() - 20]).is_err());
        assert!(NavPieceAsset::from_bytes(&[]).is_err());
    }

    #[test]
    fn quarter_turns_are_exact_and_match_glam() {
        let p = IVec3::new(1234, 56, -789);
        for k in 0..4u8 {
            let q = glam::Quat::from_rotation_y(k as f32 * std::f32::consts::FRAC_PI_2) * p.as_vec3();
            let r = Turn(k).apply(p);
            assert!((q - r.as_vec3()).abs().max_element() < 0.01, "turn {k}: glam {q}, integer {r}");
        }
        assert_eq!(Turn(4).apply(p), p);
        assert_eq!(Turn(1).apply(Turn(3).apply(p)), p);
        for d in [Dir::PosX, Dir::NegX, Dir::PosZ, Dir::NegZ] {
            assert_eq!(Turn(2).apply_dir(d), d.opposite());
        }
    }

    #[test]
    fn the_lookup_tree_has_one_leaf_per_polygon_and_covers_it() {
        let a = sample();
        assert_eq!(a.bvh.iter().filter(|n| n.index >= 0).count(), a.polys.len());
        let root = a.bvh[0];
        for p in 0..a.polys.len() {
            let (min, max) = a.poly_bounds(p);
            assert!(min.cmpge(root.min).all() && max.cmple(root.max).all());
        }
        assert_eq!(-root.index as usize, a.bvh.len(), "the root's escape index skips the whole tree");
    }
}
