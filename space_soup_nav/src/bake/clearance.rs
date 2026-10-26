//! Erosion by the agent's radius, measured against the walls themselves instead of in voxels.
//!
//! Recast keeps a span when its chamfer distance (in cells) from the edge of the walkable area is
//! at least the radius rounded up to whole cells. Walls are rasterised onto every cell they touch,
//! so that edge already sits up to a cell inside the true wall, and the rounding takes up to
//! another: a gap the agent fits through with 10-20 cm to spare can close
//! (`tests/data/golden/README.md`).
//!
//! Here the voxels still decide *what kind* of edge each missing neighbour is:
//! - a **wall** when the neighbouring column has solid in the agent's body (above its step
//!   height, below its head): the span is kept only if its cell centre is at least the radius
//!   from the actual wall geometry inside that neighbouring cell;
//! - a **drop** (floor ends, ledge, out of the grid): eroded exactly as Recast does.
//!
//! Stair steps never count as walls: a step the agent can climb is a connected neighbour, not a
//! missing one. A voxel grid still cannot hold a corridor narrower than one cell, so what is
//! promised depends on the margin asked for ([`super::Fixes::wall_margin_cells`]).

use super::compact::{chamfer, CompactHeightfield, NOT_CONNECTED};
use super::heightfield::{Heightfield, DIR_X, DIR_Z};
use super::{Config, NULL_AREA};

type P2 = [f64; 2];

/// Triangles bucketed on a 1 m grid over the floor plane, for finding the ones near a cell.
struct TriangleGrid {
    origin: [f64; 2],
    size: f64,
    nx: i32,
    nz: i32,
    buckets: Vec<Vec<u32>>,
    /// Per triangle: its vertices and its y range.
    tris: Vec<([[f64; 3]; 3], f64, f64)>,
}

impl TriangleGrid {
    fn new(verts: &[[f32; 3]], tris: &[[u32; 3]], bmin: [f32; 3], bmax: [f32; 3]) -> TriangleGrid {
        let size = 1.0;
        let origin = [bmin[0] as f64, bmin[2] as f64];
        let nx = (((bmax[0] - bmin[0]) as f64 / size).ceil() as i32).max(1);
        let nz = (((bmax[2] - bmin[2]) as f64 / size).ceil() as i32).max(1);
        let mut g = TriangleGrid { origin, size, nx, nz, buckets: vec![Vec::new(); (nx * nz) as usize], tris: Vec::with_capacity(tris.len()) };
        for (ti, t) in tris.iter().enumerate() {
            let v = t.map(|k| verts[k as usize].map(|c| c as f64));
            let (ylo, yhi) = (v.iter().map(|p| p[1]).fold(f64::MAX, f64::min), v.iter().map(|p| p[1]).fold(f64::MIN, f64::max));
            g.tris.push((v, ylo, yhi));
            let (xlo, xhi) = (v.iter().map(|p| p[0]).fold(f64::MAX, f64::min), v.iter().map(|p| p[0]).fold(f64::MIN, f64::max));
            let (zlo, zhi) = (v.iter().map(|p| p[2]).fold(f64::MAX, f64::min), v.iter().map(|p| p[2]).fold(f64::MIN, f64::max));
            let (a, b) = (g.cell(xlo, zlo), g.cell(xhi, zhi));
            for z in a.1..=b.1 {
                for x in a.0..=b.0 {
                    g.buckets[(x + z * nx) as usize].push(ti as u32);
                }
            }
        }
        g
    }

    fn cell(&self, x: f64, z: f64) -> (i32, i32) {
        (
            (((x - self.origin[0]) / self.size).floor() as i32).clamp(0, self.nx - 1),
            (((z - self.origin[1]) / self.size).floor() as i32).clamp(0, self.nz - 1),
        )
    }

    /// Triangles whose bucket overlaps the box, each once, in index order.
    fn near(&self, xlo: f64, zlo: f64, xhi: f64, zhi: f64) -> Vec<u32> {
        let (a, b) = (self.cell(xlo, zlo), self.cell(xhi, zhi));
        let mut out = Vec::new();
        for z in a.1..=b.1 {
            for x in a.0..=b.0 {
                out.extend_from_slice(&self.buckets[(x + z * self.nx) as usize]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Sutherland-Hodgman against `keep(p) >= 0`, with `cross` giving where an edge meets it.
fn clip<T: Copy>(poly: &[T], side: impl Fn(&T) -> f64, lerp: impl Fn(&T, &T, f64) -> T) -> Vec<T> {
    let n = poly.len();
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (a, b) = (&poly[i], &poly[(i + 1) % n]);
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            out.push(*a);
        }
        if (sa >= 0.0) != (sb >= 0.0) {
            out.push(lerp(a, b, sa / (sa - sb)));
        }
    }
    out
}

fn dist_point_seg(p: P2, a: P2, b: P2) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l = dx * dx + dz * dz;
    let t = if l > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qz) = (a[0] + dx * t - p[0], a[1] + dz * t - p[1]);
    (qx * qx + qz * qz).sqrt()
}

/// Distance from `p` to a convex polygon on the floor plane (0 inside). Degenerate
/// polygons, which a vertical wall projects to, are measured too.
fn dist_point_poly(p: P2, poly: &[P2]) -> f64 {
    match poly.len() {
        0 => f64::MAX,
        1 => ((p[0] - poly[0][0]).powi(2) + (p[1] - poly[0][1]).powi(2)).sqrt(),
        n => {
            let cross = |a: P2, b: P2| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            let signs: Vec<f64> = (0..n).map(|i| cross(poly[i], poly[(i + 1) % n])).collect();
            let inside = n >= 3 && (signs.iter().all(|&s| s >= 0.0) || signs.iter().all(|&s| s <= 0.0)) && signs.iter().any(|&s| s != 0.0);
            if inside {
                return 0.0;
            }
            (0..n).map(|i| dist_point_seg(p, poly[i], poly[(i + 1) % n])).fold(f64::MAX, f64::min)
        }
    }
}

/// A wall cell: a grid cell beside some walkable span, with solid in that span's body band.
struct Site {
    x: i32,
    z: i32,
    floor: i32,
    /// The wall's geometry inside the cell, clipped to the band, seen from above.
    footprints: Vec<Vec<P2>>,
}

/// Erodes `chf`: a cell centre must be `clearance_m` from any wall (exact distance), and Recast's
/// chamfer erosion by the agent's radius applies from drops.
pub fn erode_exact(chf: &mut CompactHeightfield, hf: &Heightfield, verts: &[[f32; 3]], tris: &[[u32; 3]], cfg: &Config, clearance_m: f32) {
    let (w, h) = (chf.width, chf.height);
    let (cs, ch) = (cfg.cs as f64, cfg.ch as f64);
    let (bx, by, bz) = (cfg.bmin[0] as f64, cfg.bmin[1] as f64, cfg.bmin[2] as f64);
    let (climb, height) = (cfg.walkable_climb, cfg.walkable_height);

    let mut dist = vec![0xffu8; chf.spans.len()];
    let mut sites: Vec<Site> = Vec::new();
    let mut site_at: Vec<Vec<u32>> = vec![Vec::new(); (w * h) as usize];
    for z in 0..h {
        for x in 0..w {
            let c = chf.cells[(x + z * w) as usize];
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                if chf.areas[i] == NULL_AREA {
                    dist[i] = 0;
                    continue;
                }
                let s = chf.spans[i];
                let floor = s.y as i32;
                for dir in 0..4 {
                    if s.con[dir] != NOT_CONNECTED && chf.areas[chf.neighbour(x, z, dir, s.con[dir])] != NULL_AREA {
                        continue;
                    }
                    let (nx, nz) = (x + DIR_X[dir], z + DIR_Z[dir]);
                    if nx < 0 || nz < 0 || nx >= w || nz >= h {
                        dist[i] = 0;
                        continue;
                    }
                    let column = &hf.columns[(nx + nz * w) as usize];
                    let wall = column.iter().any(|sp| (sp.smin as i32) < floor + height && (sp.smax as i32) > floor + climb);
                    if !wall {
                        dist[i] = 0;
                        continue;
                    }
                    let key = (nx + nz * w) as usize;
                    if !site_at[key].iter().any(|&k| sites[k as usize].floor == floor) {
                        site_at[key].push(sites.len() as u32);
                        sites.push(Site { x: nx, z: nz, floor, footprints: Vec::new() });
                    }
                }
            }
        }
    }
    chamfer(chf, &mut dist);

    let grid = TriangleGrid::new(verts, tris, cfg.bmin, cfg.bmax);
    for site in sites.iter_mut() {
        const EDGE: f64 = 0.001;
        let (x0, z0) = (bx + site.x as f64 * cs - EDGE, bz + site.z as f64 * cs - EDGE);
        let (x1, z1) = (x0 + cs + 2.0 * EDGE, z0 + cs + 2.0 * EDGE);
        let (ylo, yhi) = (by + (site.floor + climb) as f64 * ch, by + (site.floor + height) as f64 * ch);
        for ti in grid.near(x0, z0, x1, z1) {
            let (v, tlo, thi) = &grid.tris[ti as usize];
            if *thi < ylo || *tlo > yhi {
                continue;
            }
            let lerp3 = |a: &[f64; 3], b: &[f64; 3], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
            let mut p: Vec<[f64; 3]> = v.to_vec();
            p = clip(&p, |q| q[1] - ylo, lerp3);
            p = clip(&p, |q| yhi - q[1], lerp3);
            let lerp2 = |a: &P2, b: &P2, t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let mut f: Vec<P2> = p.iter().map(|q| [q[0], q[2]]).collect();
            f = clip(&f, |q| q[0] - x0, lerp2);
            f = clip(&f, |q| x1 - q[0], lerp2);
            f = clip(&f, |q| q[1] - z0, lerp2);
            f = clip(&f, |q| z1 - q[1], lerp2);
            if !f.is_empty() {
                site.footprints.push(f);
            }
        }
        if site.footprints.is_empty() {
            site.footprints.push(vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]]);
        }
    }

    let r = clearance_m as f64;
    let reach = (r / cs).ceil() as i32 + 1;
    let min_drop = (cfg.walkable_radius * 2).min(255) as u8;
    for z in 0..h {
        for x in 0..w {
            let c = chf.cells[(x + z * w) as usize];
            let centre = [bx + (x as f64 + 0.5) * cs, bz + (z as f64 + 0.5) * cs];
            for i in c.index as usize..(c.index + c.count as u32) as usize {
                if chf.areas[i] == NULL_AREA {
                    continue;
                }
                if dist[i] < min_drop {
                    chf.areas[i] = NULL_AREA;
                    continue;
                }
                let floor = chf.spans[i].y as i32;
                let mut near = f64::MAX;
                'cells: for sz in (z - reach).max(0)..=(z + reach).min(h - 1) {
                    for sx in (x - reach).max(0)..=(x + reach).min(w - 1) {
                        for &k in &site_at[(sx + sz * w) as usize] {
                            let site = &sites[k as usize];
                            if (site.floor - floor).abs() > climb {
                                continue;
                            }
                            for f in &site.footprints {
                                near = near.min(dist_point_poly(centre, f));
                                if near < r - 1e-4 {
                                    break 'cells;
                                }
                            }
                        }
                    }
                }
                if near < r - 1e-4 {
                    chf.areas[i] = NULL_AREA;
                }
            }
        }
    }
}
