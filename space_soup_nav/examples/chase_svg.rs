//! Draws a generated level's navmesh from above with one chase on it, as SVG: the floor (grey),
//! doorways (blue: open passage, orange: door), the monster's track (red) and the player's (green).
//!
//!   cargo run --release --example `chase_svg` -- <kit dir> <layout.json> <out.svg> [seed]

use std::fmt::Write as _;
use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::{Dir, NavPieceAsset, Turn};
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, SocketSpec};
use space_soup_nav::bake::{Partition, Settings};
use space_soup_nav::follow::{FollowSettings, PathFollower, Steer};
use space_soup_nav::obj::TriMesh;
use space_soup_nav::path::find_path_guided;
use space_soup_nav::world::{NavWorld, PolyRef, SocketRef};

const EXT: DVec3 = DVec3::new(500.0, 2000.0, 500.0);

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (kit_dir, layout_path, out) = (&a[0], &a[1], &a[2]);
    let mut seed: u32 = a.get(3).map_or(1, |s| s.parse().unwrap());
    let settings = Settings {
        cell_size: 0.1, cell_height: 0.1, agent_radius: 0.4, agent_height: 1.8, agent_max_climb: 0.3, agent_max_slope_deg: 45.0,
        partition: Partition::Monotone, region_min_size: 8, region_merge_size: 20, edge_max_len: 12.0, edge_max_error: 1.3,
        verts_per_poly: 6, detail_sample_dist: 6.0, detail_sample_max_error: 1.0, fixes: space_soup_nav::bake::Fixes::default(),
    };
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{kit_dir}/pieces.json")).unwrap()).unwrap();
    let kit: Vec<Arc<NavPieceAsset>> = index["pieces"].as_array().unwrap().iter().map(|p| {
        let name = p["piece"].as_str().unwrap();
        let mesh = TriMesh::load(std::path::Path::new(&format!("{kit_dir}/{name}.obj"))).unwrap();
        let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(glam::Vec3::to_array).collect();
        let sockets: Vec<SocketSpec> = p["doorways"].as_array().unwrap().iter().map(|d| SocketSpec {
            name: d["name"].as_str().unwrap().into(),
            position: serde_json::from_value(d["position"].clone()).unwrap(),
            outward: serde_json::from_value::<Dir>(d["outward"].clone()).unwrap(),
            size_class: d["size"].as_str().unwrap().into(),
        }).collect();
        let input = PieceInput { name, vertices: &verts, triangles: &mesh.triangles, sockets: &sockets, links: Vec::new() };
        Arc::new(bake_piece(&input, &settings, [1.0; 3], &ExportOptions::default()).unwrap().0)
    }).collect();
    let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(layout_path).unwrap()).unwrap();
    let pieces = layout["pieces"].as_array().unwrap();
    let mut w = NavWorld::new();
    for p in pieces {
        let pos: [f32; 3] = serde_json::from_value(p["position"].clone()).unwrap();
        let off = IVec3::new((pos[0] * 1000.0).round() as i32, (pos[1] * 1000.0).round() as i32, (pos[2] * 1000.0).round() as i32);
        w.add_island(kit[p["kind"].as_u64().unwrap() as usize].clone(), Turn(p["quarter_turns"].as_u64().unwrap() as u8), off);
    }
    for (i, p) in pieces.iter().enumerate() {
        for (s, l) in p["links"].as_array().unwrap().iter().enumerate() {
            let Some(l) = l.as_array() else { continue };
            let (j, t) = (l[0].as_u64().unwrap() as usize, l[1].as_u64().unwrap() as usize);
            if (j, t) <= (i, s) {
                continue;
            }
            let (ka, kb) = (&kit[p["kind"].as_u64().unwrap() as usize], &kit[pieces[j]["kind"].as_u64().unwrap() as usize]);
            let open = ka.sockets[s].name.contains("OPEN") || kb.sockets[t].name.contains("OPEN") || ka.polys.is_empty() || kb.polys.is_empty();
            w.connect(SocketRef { island: i as u32, socket: s as u16 }, SocketRef { island: j as u32, socket: t as u16 }, (!open).then_some("door")).unwrap();
        }
    }
    let graph = w.room_graph();

    let mut rnd = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); f64::from(seed >> 8) / 16777216.0 };
    let mut pick = |w: &NavWorld| loop {
        let i = (rnd() * w.islands.len() as f64) as usize % w.islands.len();
        let n = w.islands[i].asset.polys.len();
        if n > 0 {
            let v = w.poly_dverts(PolyRef { island: i as u32, poly: ((rnd() * n as f64) as usize % n) as u16 });
            break v.iter().copied().sum::<DVec3>() / v.len() as f64;
        }
    };
    let (route, monster0) = loop {
        let (a, b, m) = (pick(&w), pick(&w), pick(&w));
        if let Some(r) = find_path_guided(&w, &graph, a, b, EXT).filter(|r| r.complete && r.length() > 60_000.0) {
            if (m - a).length() > 40_000.0 && find_path_guided(&w, &graph, m, a, EXT).is_some_and(|p| p.complete) {
                break (r, m);
            }
        }
    };
    let pts: Vec<DVec3> = route.waypoints.iter().map(space_soup_nav::path::Waypoint::pos).collect();
    let along = |mut d: f64| { for s in pts.windows(2) { let l = (s[1] - s[0]).length(); if d <= l { return s[0].lerp(s[1], d / l.max(1e-9)); } d -= l; } *pts.last().unwrap() };
    let mut f = PathFollower::new(FollowSettings::default());
    let mut pos = w.nearest_poly(monster0, EXT).unwrap().point;
    let (mut mtrack, mut ptrack) = (vec![pos], vec![along(0.0)]);
    let dt = 1.0 / 36.0;
    let mut t = 0.0;
    let caught = loop {
        let player = along(t * 1400.0);
        ptrack.push(player);
        match f.update(&w, &graph, pos, player, dt) {
            Steer::Arrived => break Some(t),
            Steer::NoPath => break None,
            Steer::TakeLink { to, .. } => pos = to,
            Steer::Walk(target) => {
                let d = DVec3::new(target.x - pos.x, 0.0, target.z - pos.z);
                let step = (3500.0 * dt).min(d.length());
                if step > 1e-6 {
                    let here = w.nearest_poly(pos, EXT).unwrap();
                    let (got, _) = w.move_along_surface(here.poly, pos, pos + d.normalize() * step);
                    pos = w.nearest_poly(got, EXT).unwrap().point;
                }
            }
        }
        mtrack.push(pos);
        t += dt;
        if t > 600.0 {
            break None;
        }
    };

    let (mut lo, mut hi) = (IVec3::MAX, IVec3::MIN);
    for isl in &w.islands {
        lo = lo.min(isl.min);
        hi = hi.max(isl.max);
    }
    let s = 0.01;
    let (wpx, hpx) = ((f64::from(hi.x - lo.x) * s) + 40.0, (f64::from(hi.z - lo.z) * s) + 40.0);
    let px = |v: DVec3| ((v.x - f64::from(lo.x)) * s + 20.0, (v.z - f64::from(lo.z)) * s + 20.0);
    let mut svg = String::new();
    let _ = writeln!(svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="{wpx:.0}" height="{hpx:.0}" viewBox="0 0 {wpx:.0} {hpx:.0}"><rect width="100%" height="100%" fill="#14161a"/>"##);
    for (ii, isl) in w.islands.iter().enumerate() {
        for pi in 0..isl.asset.polys.len() {
            let v = w.poly_dverts(PolyRef { island: ii as u32, poly: pi as u16 });
            let pts: Vec<String> = v.iter().map(|&q| { let (x, y) = px(q); format!("{x:.1},{y:.1}") }).collect();
            let shade = if v.iter().all(|q| q.y > 1500.0) { "#5a6170" } else { "#3a3f48" };
            let _ = writeln!(svg, r##"<polygon points="{}" fill="{shade}" stroke="#4a505a" stroke-width="0.3"/>"##, pts.join(" "));
        }
    }
    for c in &w.connections {
        let sa = w.socket(c.a).unwrap();
        let (a, b) = (px(sa.a.as_dvec3()), px(sa.b.as_dvec3()));
        let colour = if c.links.is_empty() { continue } else if c.gate.is_some() { "#e08a2c" } else { "#4aa3ff" };
        let _ = writeln!(svg, r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{colour}" stroke-width="3"/>"#, a.0, a.1, b.0, b.1);
    }
    let line = |t: &[DVec3], colour: &str, svg: &mut String| {
        let pts: Vec<String> = t.iter().step_by(4).map(|&q| { let (x, y) = px(q); format!("{x:.1},{y:.1}") }).collect();
        let _ = writeln!(svg, r#"<polyline points="{}" fill="none" stroke="{colour}" stroke-width="2.2" stroke-linejoin="round"/>"#, pts.join(" "));
    };
    line(&ptrack, "#52d273", &mut svg);
    line(&mtrack, "#ff4d5e", &mut svg);
    for (q, colour) in [(ptrack[0], "#52d273"), (mtrack[0], "#ff4d5e")] {
        let (x, y) = px(q);
        let _ = writeln!(svg, r#"<circle cx="{x:.1}" cy="{y:.1}" r="6" fill="{colour}"/>"#);
    }
    let (x, y) = px(*mtrack.last().unwrap());
    let _ = writeln!(svg, r##"<circle cx="{x:.1}" cy="{y:.1}" r="8" fill="none" stroke="#ffffff" stroke-width="2"/>"##);
    let caption = match caught { Some(t) => format!("caught after {t:.0} s, {} plans", f.plans), None => "not caught".into() };
    let _ = writeln!(svg, r##"<text x="20" y="{:.0}" fill="#cfd3da" font-family="sans-serif" font-size="16">Seed {} navmesh: grey floor (lighter = catwalk/upper floor), blue = open doorway, orange = door. Green = player, red = monster, white ring = {caption}</text>"##, hpx - 8.0, layout["seed"]);
    svg.push_str("</svg>\n");
    std::fs::write(out, svg).unwrap();
    println!("{} islands, {} polygons; chase {caption}; wrote {out}", w.islands.len(), w.poly_count());
}
