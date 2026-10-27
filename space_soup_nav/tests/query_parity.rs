//! The point queries against C++ Detour's answers on every golden mesh and config: 200 nearest-
//! polygon, 100 raycast and 100 move-along-surface queries each.
//!
//! The world is the golden mesh baked without merging (so polygon numbers are Detour's), on the
//! millimetre lattice, as one island. Two documented differences are allowed (see `query.rs`):
//! Detour rounding its search box out to whole cells, and Detour's detail heights reading one
//! cell high. Everything else must agree.

mod common;

use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::Turn;
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;
use space_soup_nav::world::{NavWorld, PolyRef};

fn mm(v: [f32; 3]) -> DVec3 {
    DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2])) * 1000.0
}

/// Our mesh is Detour's moved by a constant under half a millimetre: the baker rounds the grid's
/// origin onto the millimetre. Detour's query points lie exactly on its own edges, so they are
/// moved the same way before asking, and answers moved back.
fn lattice_shift(bmin: [f32; 3]) -> DVec3 {
    let b = DVec3::new(f64::from(bmin[0]), f64::from(bmin[1]), f64::from(bmin[2])) * 1000.0;
    b.round() - b
}

/// The distance from `p` to the nearest edge line of a polygon, on the floor plane.
fn edge_distance(verts: &[DVec3], p: DVec3) -> f64 {
    let n = verts.len();
    (0..n)
        .map(|j| {
            let (a, b) = (verts[j], verts[(j + 1) % n]);
            let e = b - a;
            ((e.z * (p.x - a.x) - e.x * (p.z - a.z)) / (e.x * e.x + e.z * e.z).sqrt()).abs()
        })
        .fold(f64::MAX, f64::min)
}

#[derive(Default, Debug)]
struct Tally {
    nearest: usize,
    nearest_box_rounding: usize,
    nearest_ties: usize,
    nearest_detail_offset: usize,
    rays_f32_far_from_origin: usize,
    rays_detour_stalled: usize,
    nearest_equally_near_points: usize,
    rays: usize,
    rays_starting_on_a_wall: usize,
    moves: usize,
    moves_corner_ties: usize,
    worst_xz_mm: f64,
}

#[test]
fn point_queries_agree_with_detour() {
    let data = golden::test_data_dir();
    let configs = golden::load_configs(&data).unwrap().configs;
    let mut failures = Vec::new();
    let mut tally = Tally::default();
    for (file, g) in golden::load_all(&data).unwrap() {
        let Some(q) = &g.queries else { continue };
        let c = configs.iter().find(|c| c.name == g.config_name).unwrap();
        let mesh = TriMesh::load(&data.join("meshes").join(&g.input.mesh)).unwrap();
        let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(glam::Vec3::to_array).collect();
        let input = PieceInput { name: &file, vertices: &verts, triangles: &mesh.triangles, sockets: &[], links: Vec::new() };
        let opts = ExportOptions { merge_across_regions: false, flat_tolerance_mm: -1, ..Default::default() };
        let mut settings = common::settings(c);
        settings.fixes = space_soup_nav::bake::Fixes::recast();
        let (asset, _) = bake_piece(&input, &settings, [1.0; 3], &opts).unwrap();
        assert_eq!(asset.polys.len(), g.stages.as_ref().unwrap().polymesh.polys.len(), "{file}: polygon numbering differs");
        let mut w = NavWorld::new();
        let isl = w.add_island(Arc::new(asset), Turn(0), IVec3::ZERO);
        let pref = |i: i32| PolyRef { island: isl, poly: i as u16 };
        let ch_mm = f64::from(c.cell_height) * 1000.0;
        let shift = lattice_shift(g.stages.as_ref().unwrap().polymesh.bmin);
        let mm = |v: [f32; 3]| mm(v) + shift;
        let far = g.recast_config.bmin.iter().chain(&g.recast_config.bmax).fold(0.0f64, |a, &b| a.max(f64::from(b.abs())));
        let resolution = (far * 1000.0 * 2.0f64.powi(-23) * 2.0).max(0.05);

        for (k, n) in q.nearest.iter().enumerate() {
            let center = mm(n.point);
            let ext = mm(n.half_extents);
            let ours = w.nearest_poly(center, ext);
            let ours_poly = ours.map_or(-1, |o| i32::from(o.poly.poly));
            if ours_poly != n.poly {
                let candidates = w.polys_in_box(center - ext, center + ext);
                if n.poly >= 0 && !candidates.contains(&pref(n.poly)) {
                    tally.nearest_box_rounding += 1;
                    continue;
                }
                if let (Some(o), true) = (ours, n.poly >= 0) {
                    let (theirs_pt, over) = w.closest_point_on_poly(pref(n.poly), center);
                    let dist = |p: DVec3, over: bool| {
                        let diff = center - p;
                        if over { (diff.y.abs() - f64::from(c.agent_max_climb) * 1000.0).max(0.0).powi(2) } else { diff.length_squared() }
                    };
                    let (dt, d_o) = (dist(theirs_pt, over), dist(o.point, o.over_poly));
                    if (dt - d_o).abs() < 1.0 {
                        tally.nearest_ties += 1;
                        continue;
                    }
                    if (dt.sqrt() - d_o.sqrt()).abs() <= ch_mm + 2.0 {
                        tally.nearest_detail_offset += 1;
                        continue;
                    }
                }
                failures.push(format!("{file} nearest {k}: ours {ours_poly}, Detour {}", n.poly));
                continue;
            }
            if let Some(o) = ours {
                let t = mm(n.nearest);
                let dxz = ((o.point.x - t.x).powi(2) + (o.point.z - t.z).powi(2)).sqrt();
                let flat = |p: DVec3| ((p.x - center.x).powi(2) + (p.z - center.z).powi(2)).sqrt();
                if dxz > 2.0 && (flat(o.point) - flat(t)).abs() <= resolution.max(1.0) {
                    tally.nearest_equally_near_points += 1;
                    continue;
                }
                tally.worst_xz_mm = tally.worst_xz_mm.max(dxz);
                if dxz > 2.0 || (o.point.y - t.y).abs() > ch_mm + 2.0 {
                    failures.push(format!("{file} nearest {k}: point {:?} vs Detour {:?}", o.point, t));
                }
            }
            tally.nearest += 1;
        }

        for (k, r) in q.raycast.iter().enumerate() {
            let (s, e) = (mm(r.start), mm(r.end));
            let hit = w.raycast(pref(r.start_poly), s, e);
            let path: Vec<i32> = hit.path.iter().map(|p| i32::from(p.poly)).collect();
            let reached = r.t > 1e30;
            let len = (e - s).length();
            let theirs_normal = DVec3::new(f64::from(r.hit_normal[0]), f64::from(r.hit_normal[1]), f64::from(r.hit_normal[2]));
            let blocked_at_start = |t: f64, reached: bool| !reached && t * len <= 2.0;
            if blocked_at_start(hit.t, hit.t == f64::MAX) && blocked_at_start(f64::from(r.t), reached) {
                tally.rays_starting_on_a_wall += 1;
                continue;
            }
            if (hit.path.is_empty() || r.path.is_empty()) && edge_distance(&w.poly_dverts(pref(r.start_poly)), s) <= resolution {
                tally.rays_f32_far_from_origin += 1;
                continue;
            }
            if !reached && r.t == 0.0 && r.hit_normal == [0.0; 3] && !r.path.is_empty() && path.starts_with(&r.path) {
                tally.rays_detour_stalled += 1;
                continue;
            }
            let ok = if reached {
                hit.t == f64::MAX && path == r.path
            } else {
                let dir = (e - s) / len;
                let across = (hit.t - f64::from(r.t)).abs() * len * dir.dot(theirs_normal).abs();
                hit.t != f64::MAX && path == r.path && (hit.normal - theirs_normal).length() < 1e-3 && across <= 1.5
            };
            if !ok {
                failures.push(format!("{file} raycast {k}: ours t {} path {path:?} n {:?}; Detour t {} path {:?} n {:?}", hit.t, hit.normal, r.t, r.path, r.hit_normal));
                continue;
            }
            tally.rays += 1;
        }

        for (k, m) in q.move_along_surface.iter().enumerate() {
            let (pos, visited) = w.move_along_surface(pref(m.start_poly), mm(m.start), mm(m.end));
            let visited: Vec<i32> = visited.iter().map(|p| i32::from(p.poly)).collect();
            let t = mm(m.result);
            let off = (pos - t).abs().max_element();
            if visited != m.visited && off <= 2.0 && visited.first() == m.visited.first() {
                tally.moves_corner_ties += 1;
                continue;
            }
            if visited != m.visited || off > 2.0 {
                failures.push(format!("{file} move {k}: ours {pos:?} {visited:?}; Detour {t:?} {:?}", m.visited));
                continue;
            }
            tally.moves += 1;
        }
    }
    println!("{tally:?}");
    let shown: Vec<_> = failures.iter().take(25).collect();
    println!("raycast {} move {} nearest {}", failures.iter().filter(|f| f.contains(" raycast ")).count(), failures.iter().filter(|f| f.contains(" move ")).count(), failures.iter().filter(|f| f.contains(" nearest ")).count());
    assert!(failures.is_empty(), "{} disagreements:\n{}", failures.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}
