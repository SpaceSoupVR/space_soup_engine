//! Baking one room piece into a [`NavPieceAsset`]: Recast's mesh, made to fit a level assembled
//! from pieces.
//!
//! What this adds to Recast's output:
//!
//! - **Doorways reach their socket plane.** Recast shrinks walkable space back from every edge by
//!   the agent's radius, the open doorway edge included, so two pieces' meshes would stop short of
//!   each other. Each doorway gets a temporary apron of floor beyond it for the bake; the mesh is
//!   then cut exactly at the socket plane and the edges lying on it become the socket.
//! - **Integer millimetres.** Vertices are put on an exact millimetre lattice (the voxel grid's),
//!   so quarter turns and socket joins are exact integer operations at load.
//! - **Convex merge across regions.** Recast merges triangles into polygons only within one region;
//!   neighbouring polygons of different regions are merged here too when the result stays convex,
//!   which removes the long thin slivers monotone partitioning makes.
//! - **Detail only where the floor is not flat.** A polygon keeps height detail only if the floor
//!   departs from the polygon's own surface by more than a tolerance (stairs, ramps).

use std::collections::BTreeMap;

use glam::{IVec3, Vec3};

use super::{build, Build, Settings, WALKABLE_AREA};
use crate::asset::{build_bvh, ccw_from_above, AgentClass, DetailMesh, Dir, EdgeLink, NavLink, NavPieceAsset, NavPoly, Socket, MAX_POLY_VERTS};
use crate::obj::TriMesh;

/// A doorway as authored: its marker's position (piece space, metres, on the floor and on the
/// piece's outer face) and which way it faces out.
#[derive(Debug, Clone, PartialEq)]
pub struct SocketSpec {
    pub name: String,
    pub position: [f32; 3],
    pub outward: Dir,
    pub size_class: String,
}

pub struct PieceInput<'a> {
    pub name: &'a str,
    /// Piece space, metres, +Y up, triangles counter-clockwise seen from their front.
    pub vertices: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
    pub sockets: &'a [SocketSpec],
    pub links: Vec<NavLink>,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// How far either side of a doorway's marker counts as that doorway, metres. Also the
    /// apron's half width. Wider than any doorway in the kit.
    pub socket_half_width: f32,
    pub merge_across_regions: bool,
    /// A polygon keeps its height detail only where the floor departs from the polygon's own
    /// surface by more than this.
    pub flat_tolerance_mm: i32,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { socket_half_width: 2.5, merge_across_regions: true, flat_tolerance_mm: 50 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BakeReport {
    /// Polygons as Recast made them, after cutting at the sockets, and in the asset.
    pub recast_polys: usize,
    pub after_cut: usize,
    pub final_polys: usize,
    /// Polygons that kept height detail.
    pub detail_polys: usize,
    /// Per socket: how many polygon edges lie on it (0: this agent cannot pass that doorway).
    pub socket_edges: Vec<usize>,
    /// Milliseconds: the Recast build, and everything after it.
    pub build_ms: f64,
    pub export_ms: f64,
}

/// A polygon while it is edited: explicit coordinates, and explicit detail triangles when the
/// floor under it is not flat.
#[derive(Debug, Clone)]
struct WorkPoly {
    verts: Vec<IVec3>,
    area: u8,
    detail: Option<Vec<[IVec3; 3]>>,
}

fn twice_area(v: &[IVec3]) -> i64 {
    (1..v.len().saturating_sub(1)).map(|i| ccw_from_above(v[0], v[i], v[i + 1])).sum()
}

fn is_convex(v: &[IVec3]) -> bool {
    let n = v.len();
    n >= 3 && (0..n).all(|k| ccw_from_above(v[k], v[(k + 1) % n], v[(k + 2) % n]) >= 0) && twice_area(v) > 0
}

/// Rounded `n / d`, halves away from zero.
fn round_div(n: i64, d: i64) -> i64 {
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    if n >= 0 { (2 * n + d) / (2 * d) } else { -((-2 * n + d) / (2 * d)) }
}

/// A half-space `side(v) = sign * v[axis] - plane <= 0`.
#[derive(Debug, Clone, Copy)]
struct Cut {
    axis: usize,
    sign: i32,
    plane: i32,
}

impl Cut {
    fn side(&self, v: IVec3) -> i64 {
        self.sign as i64 * v[self.axis] as i64 - self.plane as i64
    }

    /// Where segment `a`-`b` meets the plane. Computed from the endpoints in a fixed order, so the
    /// two polygons sharing an edge get the identical point.
    fn crossing(&self, a: IVec3, b: IVec3) -> IVec3 {
        let (p, q) = if (a.x, a.y, a.z) <= (b.x, b.y, b.z) { (a, b) } else { (b, a) };
        let (sp, sq) = (self.side(p), self.side(q));
        let mut out = IVec3::ZERO;
        for k in 0..3 {
            out[k] = if k == self.axis {
                self.plane * self.sign
            } else {
                (p[k] as i64 + round_div((q[k] - p[k]) as i64 * sp, sp - sq)) as i32
            };
        }
        out
    }

    /// Sutherland-Hodgman against this half-space.
    fn clip(&self, poly: &[IVec3]) -> Vec<IVec3> {
        let n = poly.len();
        let mut out = Vec::with_capacity(n + 1);
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let (sa, sb) = (self.side(a), self.side(b));
            if sa <= 0 {
                out.push(a);
            }
            if (sa < 0 && sb > 0) || (sa > 0 && sb < 0) {
                out.push(self.crossing(a, b));
            }
        }
        out.dedup();
        while out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    }
}

/// Ear clipping for a small, nearly convex polygon (a cut can dent one by half a millimetre).
fn triangulate(poly: &[IVec3]) -> Vec<[IVec3; 3]> {
    let mut idx: Vec<usize> = (0..poly.len()).collect();
    let mut tris = Vec::new();
    while idx.len() > 3 {
        let n = idx.len();
        let ear = (0..n).find(|&i| {
            let (a, b, c) = (poly[idx[(i + n - 1) % n]], poly[idx[i]], poly[idx[(i + 1) % n]]);
            ccw_from_above(a, b, c) > 0
                && idx.iter().all(|&k| {
                    let p = poly[k];
                    p == a || p == b || p == c || !(ccw_from_above(a, b, p) >= 0 && ccw_from_above(b, c, p) >= 0 && ccw_from_above(c, a, p) >= 0)
                })
        });
        let Some(i) = ear else { break };
        tris.push([poly[idx[(i + n - 1) % n]], poly[idx[i]], poly[idx[(i + 1) % n]]]);
        idx.remove(i);
    }
    if idx.len() == 3 {
        tris.push([poly[idx[0]], poly[idx[1]], poly[idx[2]]]);
    }
    tris.retain(|t| ccw_from_above(t[0], t[1], t[2]) > 0);
    tris
}

fn fan(v: &[IVec3]) -> Vec<[IVec3; 3]> {
    (1..v.len() - 1).map(|i| [v[0], v[i], v[i + 1]]).collect()
}

/// Height of the polygon's own surface (its fan from vertex 0) at `p`, if `p` is over it.
fn fan_height(v: &[IVec3], p: IVec3) -> Option<f64> {
    for t in fan(v) {
        let (a, b, c) = (t[0].as_dvec3(), t[1].as_dvec3(), t[2].as_dvec3());
        let p = p.as_dvec3();
        let det = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
        if det == 0.0 {
            continue;
        }
        let u = ((p.x - a.x) * (c.z - a.z) - (c.x - a.x) * (p.z - a.z)) / det;
        let w = ((b.x - a.x) * (p.z - a.z) - (p.x - a.x) * (b.z - a.z)) / det;
        if u >= -1e-6 && w >= -1e-6 && u + w <= 1.0 + 1e-6 {
            return Some(a.y + (b.y - a.y) * u + (c.y - a.y) * w);
        }
    }
    None
}

pub fn agent_class(s: &Settings) -> AgentClass {
    let mm = |m: f32| (m as f64 * 1000.0).round() as i32;
    AgentClass {
        radius_mm: mm(s.agent_radius),
        height_mm: mm(s.agent_height),
        climb_mm: mm(s.agent_max_climb),
        max_slope_cdeg: (s.agent_max_slope_deg as f64 * 100.0).round() as u16,
    }
}

/// Most voxel columns one piece may bake into: 8.4 million, a 290 m square at 0.1 m cells. A room
/// piece is tens of metres; anything near this is a broken export, refused before it is allocated.
pub const MAX_BAKE_COLUMNS: f64 = (1u64 << 23) as f64;

/// Refuses input the baker cannot bake sensibly: non-finite numbers, triangles naming missing
/// vertices, and a piece (or cell size) that would need an absurd grid.
fn check_input(input: &PieceInput, settings: &Settings, scale: [f32; 3]) -> Result<(), String> {
    let name = input.name;
    if scale.iter().any(|s| !s.is_finite()) {
        return Err(format!("{name}: scale {scale:?} is not finite"));
    }
    if !(settings.cell_size.is_finite() && settings.cell_size > 0.0 && settings.cell_height.is_finite() && settings.cell_height > 0.0) {
        return Err(format!("{name}: cell size {} / height {} must be positive", settings.cell_size, settings.cell_height));
    }
    if let Some(v) = input.vertices.iter().find(|v| v.iter().any(|c| !c.is_finite())) {
        return Err(format!("{name}: vertex {v:?} is not finite"));
    }
    if let Some(s) = input.sockets.iter().find(|s| s.position.iter().any(|c| !c.is_finite())) {
        return Err(format!("{name}: socket {} at {:?} is not finite", s.name, s.position));
    }
    let n = input.vertices.len() as u64;
    if let Some(t) = input.triangles.iter().find(|t| t.iter().any(|&i| i as u64 >= n)) {
        return Err(format!("{name}: triangle {t:?} names a vertex of {n}"));
    }
    // The extent the grid will cover: every vertex and socket, scaled, plus the aprons' reach.
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in input.vertices.iter().chain(input.sockets.iter().map(|s| &s.position)) {
        for k in 0..3 {
            let c = p[k] as f64 * scale[k] as f64;
            lo[k] = lo[k].min(c);
            hi[k] = hi[k].max(c);
        }
    }
    if lo[0] > hi[0] {
        return Ok(());
    }
    let reach = 2.0 * (settings.agent_radius as f64 + 4.0 * settings.cell_size as f64 + 1.0);
    let cs = settings.cell_size as f64;
    let columns = ((hi[0] - lo[0] + reach) / cs + 1.0) * ((hi[2] - lo[2] + reach) / cs + 1.0);
    let far = lo.iter().chain(&hi).any(|c| c.abs() * 1000.0 > crate::asset::MAX_COORD_MM as f64);
    if columns > MAX_BAKE_COLUMNS || far {
        return Err(format!(
            "{name}: spans ({:.1}, {:.1}, {:.1}) to ({:.1}, {:.1}, {:.1}) m, too large to bake at {cs} m cells",
            lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]
        ));
    }
    Ok(())
}

/// Bakes one piece for one agent class at one scale.
pub fn bake_piece(input: &PieceInput, settings: &Settings, scale: [f32; 3], opts: &ExportOptions) -> Result<(NavPieceAsset, BakeReport), String> {
    if scale.iter().any(|&s| s <= 0.0) {
        return Err(format!("{}: scale {scale:?}: mirrored or zero scale variants are not supported", input.name));
    }
    check_input(input, settings, scale)?;
    let t0 = std::time::Instant::now();
    let sv = Vec3::from(scale);
    let mut verts: Vec<[f32; 3]> = input.vertices.iter().map(|v| (Vec3::from(*v) * sv).to_array()).collect();
    let mut tris: Vec<[u32; 3]> = input.triangles.to_vec();
    let sockets: Vec<SocketSpec> = input
        .sockets
        .iter()
        .map(|s| SocketSpec { position: (Vec3::from(s.position) * sv).to_array(), ..s.clone() })
        .collect();

    // The apron: floor beyond each doorway, deep enough that erosion from its far edge cannot
    // reach back to the socket plane, with the doorway's own sides carried on through it. The
    // neighbour on the other side has the same doorway, so this is what the agent will meet there;
    // a plain wide apron let the opening flare out at the plane (0.60 to 1.00 m walkable for the
    // same 1.66 m door, against 0.86 m exactly).
    let apron = settings.agent_radius + 4.0 * settings.cell_size;
    let openings: Vec<(f32, f32)> = sockets.iter().map(|s| doorway_opening(&verts, &tris, s, opts.socket_half_width)).collect();
    for (s, &(left, right)) in sockets.iter().zip(&openings) {
        let p = Vec3::from(s.position);
        let out = s.outward.vec().as_vec3();
        let lat = if matches!(s.outward, Dir::PosX | Dir::NegX) { 2 } else { 0 };
        let at = |l: f32, depth: f32, y: f32| {
            let mut v = p + out * depth;
            v[lat] = l;
            v.y = p.y + y;
            v
        };
        let mut boxes = vec![(at(left - 0.2, 0.0, -0.2), at(right + 0.2, apron, 0.0))];
        if left > p[lat] - opts.socket_half_width {
            boxes.push((at(left - 0.2, 0.0, 0.0), at(left, apron, settings.agent_height + 0.5)));
        }
        if right < p[lat] + opts.socket_half_width {
            boxes.push((at(right, 0.0, 0.0), at(right + 0.2, apron, settings.agent_height + 0.5)));
        }
        for (a, b) in boxes {
            let slab = TriMesh::cuboid(a.min(b), a.max(b));
            let base = verts.len() as u32;
            verts.extend(slab.vertices.iter().map(|v| v.to_array()));
            tris.extend(slab.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        }
    }

    let b: Build = build(&verts, &tris, settings);
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let t1 = std::time::Instant::now();
    let pm = &b.polymesh;
    let mut report = BakeReport { recast_polys: pm.npolys(), build_ms, ..Default::default() };

    // Onto the millimetre lattice.
    let cs_mm = (pm.cs as f64 * 1000.0).round() as i32;
    let ch_mm = (pm.ch as f64 * 1000.0).round() as i32;
    if ((pm.cs as f64 * 1000.0) - cs_mm as f64).abs() > 1e-3 || ((pm.ch as f64 * 1000.0) - ch_mm as f64).abs() > 1e-3 {
        return Err(format!("cell size {} / height {} must be whole millimetres", pm.cs, pm.ch));
    }
    let origin = IVec3::new(
        (pm.bmin[0] as f64 * 1000.0).round() as i32,
        (pm.bmin[1] as f64 * 1000.0).round() as i32,
        (pm.bmin[2] as f64 * 1000.0).round() as i32,
    );
    let lattice = |v: [u16; 3]| origin + IVec3::new(v[0] as i32 * cs_mm, v[1] as i32 * ch_mm, v[2] as i32 * cs_mm);
    let tol = opts.flat_tolerance_mm as f64;
    let mut polys: Vec<WorkPoly> = Vec::new();
    for i in 0..pm.npolys() {
        let pv: Vec<IVec3> = pm.poly_verts(i).iter().map(|&v| lattice(pm.verts[v as usize])).collect();
        let [vb, _, tb, tc] = b.detail.meshes[i];
        // Recast places detail one cell height above its polygons; taken off so both describe
        // the same floor. Its first vertices are copies of the polygon's: those use the exact ones.
        let dv = |k: u8| -> IVec3 {
            if (k as usize) < pv.len() {
                return pv[k as usize];
            }
            let v = b.detail.verts[(vb + k as u32) as usize];
            IVec3::new(
                (v[0] as f64 * 1000.0).round() as i32,
                ((v[1] - pm.ch) as f64 * 1000.0).round() as i32,
                (v[2] as f64 * 1000.0).round() as i32,
            )
        };
        let dtris: Vec<[IVec3; 3]> = b.detail.tris[tb as usize..(tb + tc) as usize].iter().map(|t| [dv(t[0]), dv(t[1]), dv(t[2])]).collect();
        let flat = dtris.iter().flatten().all(|&v| fan_height(&pv, v).is_none_or(|h| (h - v.y as f64).abs() <= tol));
        let area = if pm.areas[i] == WALKABLE_AREA { 0 } else { pm.areas[i] };
        polys.push(WorkPoly { verts: pv, area, detail: if flat { None } else { Some(dtris) } });
    }

    // Cut at every socket plane, keeping the piece's side.
    let cuts: Vec<Cut> = sockets
        .iter()
        .map(|s| {
            let axis = if matches!(s.outward, Dir::PosX | Dir::NegX) { 0 } else { 2 };
            let sign = s.outward.vec()[axis];
            let pos = (s.position[axis] as f64 * 1000.0).round() as i32;
            Cut { axis, sign, plane: sign * pos }
        })
        .collect();
    for cut in &cuts {
        let mut next = Vec::with_capacity(polys.len());
        for p in polys {
            if p.verts.iter().all(|&v| cut.side(v) <= 0) {
                next.push(p);
                continue;
            }
            if p.verts.iter().all(|&v| cut.side(v) >= 0) {
                continue;
            }
            let verts = cut.clip(&p.verts);
            let detail = p.detail.map(|d| d.iter().flat_map(|t| fan_or_empty(&cut.clip(t))).collect());
            next.push(WorkPoly { verts, area: p.area, detail });
        }
        polys = next;
    }

    // Doorway floors are exactly the marker's height, so neighbours meet without a step.
    let doorway = |v: IVec3| -> Option<usize> {
        cuts.iter().zip(&sockets).position(|(c, s)| {
            let lat = if c.axis == 0 { 2 } else { 0 };
            let centre = (s.position[lat] as f64 * 1000.0).round() as i32;
            let half = (opts.socket_half_width as f64 * 1000.0).round() as i32;
            c.side(v) == 0 && (v[lat] - centre).abs() <= half
        })
    };
    let snap = |v: IVec3| -> IVec3 {
        match doorway(v) {
            Some(k) => IVec3::new(v.x, (sockets[k].position[1] as f64 * 1000.0).round() as i32, v.z),
            None => v,
        }
    };
    for p in polys.iter_mut() {
        for v in p.verts.iter_mut() {
            *v = snap(*v);
        }
        if let Some(d) = p.detail.as_mut() {
            for v in d.iter_mut().flatten() {
                *v = snap(*v);
            }
        }
    }

    // Anything the cuts left too big or dented becomes triangles; the merge rebuilds them.
    let mut cleaned = Vec::with_capacity(polys.len());
    for p in polys {
        if p.verts.len() < 3 || twice_area(&p.verts) <= 0 {
            continue;
        }
        if p.verts.len() <= MAX_POLY_VERTS && is_convex(&p.verts) {
            cleaned.push(p);
            continue;
        }
        let detail = p.detail.clone();
        for t in triangulate(&p.verts) {
            let d = detail.as_ref().map(|d| d.iter().filter(|dt| dt.iter().all(|&v| contains(&t, v))).copied().collect::<Vec<_>>());
            cleaned.push(WorkPoly { verts: t.to_vec(), area: p.area, detail: d.filter(|d| !d.is_empty()) });
        }
    }
    let mut polys = cleaned;
    report.after_cut = polys.len();

    if opts.merge_across_regions {
        merge_convex(&mut polys);
    }

    let asset = assemble(input, settings, scale, &sockets, &cuts, polys, &doorway, &mut report)?;
    report.export_ms = t1.elapsed().as_secs_f64() * 1000.0;
    Ok((asset, report))
}

/// The doorway's clear opening along the plane, from the geometry: the gap around the marker in
/// whatever is solid between knee and head height within a metre inside the plane. A side with
/// nothing solid within `half_width` is open to `half_width`.
pub fn doorway_opening(verts: &[[f32; 3]], tris: &[[u32; 3]], s: &SocketSpec, half_width: f32) -> (f32, f32) {
    let p = s.position;
    let axis = if matches!(s.outward, Dir::PosX | Dir::NegX) { 0 } else { 2 };
    let lat = 2 - axis;
    let sign = s.outward.vec()[axis] as f32;
    let centre = p[lat];
    let (mut left, mut right) = (centre - half_width, centre + half_width);
    for t in tris {
        let v = [verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]];
        let (ylo, yhi) = (v.iter().map(|q| q[1]).fold(f32::MAX, f32::min), v.iter().map(|q| q[1]).fold(f32::MIN, f32::max));
        if yhi < p[1] + 0.3 || ylo > p[1] + 1.6 {
            continue;
        }
        let depth = v.map(|q| sign * (p[axis] - q[axis]));
        if depth.iter().fold(f32::MIN, |a, &b| a.max(b)) < 0.0 || depth.iter().fold(f32::MAX, |a, &b| a.min(b)) > 1.0 {
            continue;
        }
        let (lo, hi) = (v.iter().map(|q| q[lat]).fold(f32::MAX, f32::min), v.iter().map(|q| q[lat]).fold(f32::MIN, f32::max));
        if hi <= centre {
            left = left.max(hi);
        } else if lo >= centre {
            right = right.min(lo);
        }
    }
    (left, right)
}

fn fan_or_empty(v: &[IVec3]) -> Vec<[IVec3; 3]> {
    if v.len() < 3 { Vec::new() } else { fan(v) }
}

/// Is `p` inside or on triangle `t` (from above)?
fn contains(t: &[IVec3; 3], p: IVec3) -> bool {
    ccw_from_above(t[0], t[1], p) >= 0 && ccw_from_above(t[1], t[2], p) >= 0 && ccw_from_above(t[2], t[0], p) >= 0
}

/// Greedy merge, longest shared edge first (Recast's rule), across all polygons of one area.
fn merge_convex(polys: &mut Vec<WorkPoly>) {
    loop {
        let mut edges: BTreeMap<(IVec3Key, IVec3Key), (usize, usize)> = BTreeMap::new();
        for (pi, p) in polys.iter().enumerate() {
            let n = p.verts.len();
            for e in 0..n {
                edges.insert((key(p.verts[e]), key(p.verts[(e + 1) % n])), (pi, e));
            }
        }
        let mut best: Option<(i64, usize, usize, usize, usize)> = None;
        for (pa, p) in polys.iter().enumerate() {
            let na = p.verts.len();
            for ea in 0..na {
                let (u, v) = (p.verts[ea], p.verts[(ea + 1) % na]);
                let Some(&(pb, eb)) = edges.get(&(key(v), key(u))) else { continue };
                if pb <= pa || polys[pb].area != p.area {
                    continue;
                }
                let nb = polys[pb].verts.len();
                if na + nb - 2 > MAX_POLY_VERTS {
                    continue;
                }
                let merged = merged_verts(&p.verts, ea, &polys[pb].verts, eb);
                if !(0..merged.len()).all(|k| ccw_from_above(merged[k], merged[(k + 1) % merged.len()], merged[(k + 2) % merged.len()]) >= 0) {
                    continue;
                }
                // Recast's rule: both corners where the polygons join must turn strictly.
                let q = &polys[pb].verts;
                if ccw_from_above(p.verts[(ea + na - 1) % na], p.verts[ea], q[(eb + 2) % nb]) <= 0
                    || ccw_from_above(q[(eb + nb - 1) % nb], q[eb], p.verts[(ea + 2) % na]) <= 0
                {
                    continue;
                }
                let d = u - v;
                let len = d.x as i64 * d.x as i64 + d.z as i64 * d.z as i64;
                if best.is_none_or(|b| len > b.0) {
                    best = Some((len, pa, ea, pb, eb));
                }
            }
        }
        let Some((_, pa, ea, pb, eb)) = best else { return };
        let b = polys.remove(pb);
        let a = &mut polys[pa];
        let detail = if a.detail.is_none() && b.detail.is_none() {
            None
        } else {
            let mut d = a.detail.take().unwrap_or_else(|| fan(&a.verts));
            d.extend(b.detail.clone().unwrap_or_else(|| fan(&b.verts)));
            Some(d)
        };
        a.verts = merged_verts(&a.verts, ea, &b.verts, eb);
        a.detail = detail;
    }
}

/// Recast's `mergePolyVerts`: A from after the shared edge round to its start, then B likewise.
fn merged_verts(a: &[IVec3], ea: usize, b: &[IVec3], eb: usize) -> Vec<IVec3> {
    let (na, nb) = (a.len(), b.len());
    let mut out = Vec::with_capacity(na + nb - 2);
    for i in 0..na - 1 {
        out.push(a[(ea + 1 + i) % na]);
    }
    for i in 0..nb - 1 {
        out.push(b[(eb + 1 + i) % nb]);
    }
    out
}

type IVec3Key = (i32, i32, i32);

fn key(v: IVec3) -> IVec3Key {
    (v.x, v.y, v.z)
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    input: &PieceInput,
    settings: &Settings,
    scale: [f32; 3],
    sockets: &[SocketSpec],
    cuts: &[Cut],
    polys: Vec<WorkPoly>,
    doorway: &dyn Fn(IVec3) -> Option<usize>,
    report: &mut BakeReport,
) -> Result<NavPieceAsset, String> {
    let mut ids: BTreeMap<IVec3Key, u16> = BTreeMap::new();
    let mut vertices: Vec<IVec3> = Vec::new();
    let mut id = |v: IVec3, vertices: &mut Vec<IVec3>| -> u16 {
        *ids.entry(key(v)).or_insert_with(|| {
            vertices.push(v);
            (vertices.len() - 1) as u16
        })
    };
    let poly_ids: Vec<Vec<u16>> = polys.iter().map(|p| p.verts.iter().map(|&v| id(v, &mut vertices)).collect()).collect();
    if vertices.len() > u16::MAX as usize || polys.len() > u16::MAX as usize {
        return Err(format!("{}: too many vertices or polygons for one piece", input.name));
    }

    let mut directed: BTreeMap<(u16, u16), u16> = BTreeMap::new();
    for (pi, p) in poly_ids.iter().enumerate() {
        for e in 0..p.len() {
            directed.insert((p[e], p[(e + 1) % p.len()]), pi as u16);
        }
    }
    let mut out_sockets: Vec<Socket> = sockets
        .iter()
        .map(|s| {
            let p = IVec3::new(
                (s.position[0] as f64 * 1000.0).round() as i32,
                (s.position[1] as f64 * 1000.0).round() as i32,
                (s.position[2] as f64 * 1000.0).round() as i32,
            );
            Socket { name: s.name.clone(), a: p, b: p, outward: s.outward, size_class: s.size_class.clone(), edges: Vec::new() }
        })
        .collect();
    let mut nav_polys = Vec::with_capacity(polys.len());
    for (pi, p) in poly_ids.iter().enumerate() {
        let n = p.len();
        let mut edges = Vec::with_capacity(n);
        for e in 0..n {
            let (u, v) = (p[e], p[(e + 1) % n]);
            let link = match directed.get(&(v, u)) {
                Some(&o) if o as usize != pi => EdgeLink::Internal(o),
                _ => {
                    let (a, b) = (vertices[u as usize], vertices[v as usize]);
                    match (doorway(a), doorway(b)) {
                        (Some(k), Some(l)) if k == l => {
                            out_sockets[k].edges.push((pi as u16, e as u8));
                            EdgeLink::Socket(k as u16)
                        }
                        _ => EdgeLink::Border,
                    }
                }
            };
            edges.push(link);
        }
        nav_polys.push(NavPoly { verts: p.clone(), edges, area: polys[pi].area, flags: 1 });
    }
    // Each socket's span: the extreme ends of its edges along the doorway.
    for (k, s) in out_sockets.iter_mut().enumerate() {
        let lat = if cuts[k].axis == 0 { 2 } else { 0 };
        let ends: Vec<IVec3> = s
            .edges
            .iter()
            .flat_map(|&(p, e)| {
                let poly = &nav_polys[p as usize];
                let n = poly.verts.len();
                [vertices[poly.verts[e as usize] as usize], vertices[poly.verts[(e as usize + 1) % n] as usize]]
            })
            .collect();
        if let (Some(lo), Some(hi)) = (ends.iter().min_by_key(|v| v[lat]), ends.iter().max_by_key(|v| v[lat])) {
            s.a = *lo;
            s.b = *hi;
        }
        report.socket_edges.push(s.edges.len());
    }

    let mut detail = DetailMesh::default();
    for (pi, p) in polys.iter().enumerate() {
        let first_vertex = detail.vertices.len() as u32;
        let first_tri = detail.triangles.len() as u32;
        let mut own: Vec<IVec3> = Vec::new();
        if let Some(d) = &p.detail {
            report.detail_polys += 1;
            for t in d {
                let mut tri = [0u8; 3];
                for (k, v) in t.iter().enumerate() {
                    let local = match p.verts.iter().position(|pv| pv == v) {
                        Some(i) => i,
                        None => {
                            let i = own.iter().position(|o| o == v).unwrap_or_else(|| {
                                own.push(*v);
                                own.len() - 1
                            });
                            p.verts.len() + i
                        }
                    };
                    if local > u8::MAX as usize {
                        return Err(format!("{}: polygon {pi} has too much height detail", input.name));
                    }
                    tri[k] = local as u8;
                }
                detail.triangles.push(tri);
            }
        }
        detail.vertices.extend_from_slice(&own);
        detail.ranges.push((first_vertex, own.len() as u16, first_tri, (detail.triangles.len() as u32 - first_tri) as u16));
    }

    let mut asset = NavPieceAsset {
        name: input.name.to_string(),
        agent: agent_class(settings),
        scale_e4: scale.map(|s| (s as f64 * 10000.0).round() as u32),
        vertices,
        polys: nav_polys,
        detail,
        bvh: Vec::new(),
        sockets: out_sockets,
        links: input.links.clone(),
    };
    asset.bvh = build_bvh(&asset);
    report.final_polys = asset.polys.len();
    asset.validate().map_err(|e| format!("{}: {e}", input.name))?;
    Ok(asset)
}
