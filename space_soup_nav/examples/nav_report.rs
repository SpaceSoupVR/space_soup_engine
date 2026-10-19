//! A navigation report for a kit and its levels: every piece checked, the kit checked, every level
//! assembled and reported on, per agent class, with a picture of the first level per class.
//!
//!   cargo run --release --example nav_report -- <kit dir> <layouts dir> <out dir> [config ...]
//!
//! Writes `<out dir>/report.md` and `<out dir>/<class>_seed<N>.svg` (floor coloured by connected
//! area, problems marked in red).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use glam::IVec3;
use space_soup_nav::asset::{Dir, NavPieceAsset, Turn};
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, SocketSpec};
use space_soup_nav::bake::{Fixes, Partition, Settings};
use space_soup_nav::debug::{DebugDraw, FloorColour};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;
use space_soup_nav::validate::{check_kit, check_level, check_piece, Severity};
use space_soup_nav::world::{NavWorld, PolyRef, SocketRef};

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: nav_report <kit dir> <layouts dir> <out dir> [config ...]";
    let kit_dir = PathBuf::from(args.next().expect(usage));
    let layouts_dir = PathBuf::from(args.next().expect(usage));
    let out = PathBuf::from(args.next().expect(usage));
    let wanted: Vec<String> = args.collect();
    std::fs::create_dir_all(&out).unwrap();

    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(kit_dir.join("pieces.json")).unwrap()).unwrap();
    let mut layouts: Vec<PathBuf> = std::fs::read_dir(&layouts_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    layouts.sort();
    let configs: Vec<_> = golden::load_configs(&golden::test_data_dir())
        .unwrap()
        .configs
        .into_iter()
        .filter(|c| wanted.is_empty() || wanted.contains(&c.name))
        .collect();

    let mut md = String::from("# Navigation report\n\n");
    let _ = writeln!(md, "Kit: `{}` ({} pieces). Levels: {} from `{}`.\n", kit_dir.display(), index["pieces"].as_array().unwrap().len(), layouts.len(), layouts_dir.display());
    for c in &configs {
        let settings = Settings {
            cell_size: c.cell_size,
            cell_height: c.cell_height,
            agent_radius: c.agent_radius,
            agent_height: c.agent_height,
            agent_max_climb: c.agent_max_climb,
            agent_max_slope_deg: c.agent_max_slope_deg,
            partition: if c.partition == "monotone" { Partition::Monotone } else { Partition::Layers },
            region_min_size: c.region_min_size,
            region_merge_size: c.region_merge_size,
            edge_max_len: c.edge_max_len,
            edge_max_error: c.edge_max_error,
            verts_per_poly: c.verts_per_poly,
            detail_sample_dist: c.detail_sample_dist,
            detail_sample_max_error: c.detail_sample_max_error,
            fixes: Fixes::default(),
        };
        let kit: Vec<Arc<NavPieceAsset>> = index["pieces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let name = p["piece"].as_str().unwrap();
                let mesh = TriMesh::load(&kit_dir.join(format!("{name}.obj"))).unwrap();
                let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
                let sockets: Vec<SocketSpec> = p["doorways"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| SocketSpec {
                        name: d["name"].as_str().unwrap().into(),
                        position: serde_json::from_value(d["position"].clone()).unwrap(),
                        outward: serde_json::from_value::<Dir>(d["outward"].clone()).unwrap(),
                        size_class: d["size"].as_str().unwrap().into(),
                    })
                    .collect();
                let input = PieceInput { name, vertices: &verts, triangles: &mesh.triangles, sockets: &sockets, links: Vec::new() };
                Arc::new(bake_piece(&input, &settings, [1.0; 3], &ExportOptions::default()).unwrap().0)
            })
            .collect();

        let _ = writeln!(md, "## {} (radius {:.2} m, height {:.2} m)\n", c.name, c.agent_radius, c.agent_height);
        let _ = writeln!(md, "### Pieces\n");
        let mut issues: Vec<_> = kit.iter().flat_map(check_piece).chain(check_kit(&kit)).collect();
        issues.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.subject.cmp(&b.subject)));
        if issues.is_empty() {
            md.push_str("No issues.\n");
        }
        for i in &issues {
            let mark = match i.severity {
                Severity::Error => "**Error**",
                Severity::Warning => "Warning",
                Severity::Info => "Info",
            };
            let _ = writeln!(md, "- {mark}, {}: {}", i.subject, i.message);
        }
        md.push('\n');

        let mut summary = Vec::new();
        for (k, path) in layouts.iter().enumerate() {
            let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
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
                    if let Err(e) = w.connect(SocketRef { island: i as u32, socket: s as u16 }, SocketRef { island: j as u32, socket: t as u16 }, (!open).then_some("door")) {
                        let _ = writeln!(md, "- **Error**, seed {}: join {i}.{s} - {j}.{t} refused: {e:?}", layout["seed"]);
                    }
                }
            }
            let start = (0..w.islands.len()).find(|&i| !w.islands[i].asset.polys.is_empty()).map(|i| PolyRef { island: i as u32, poly: 0 }).unwrap();
            let report = check_level(&mut w, start);
            summary.push((layout["seed"].as_u64().unwrap(), report.clone()));
            if k == 0 {
                let mut d = DebugDraw::world(&mut w, FloorColour::ByArea);
                for (_, at) in &report.impassable_joins {
                    d.add_problem(*at, "impassable");
                }
                for (i, at) in &report.unreachable {
                    d.add_problem(*at, format!("#{i} cut off"));
                }
                let file = format!("{}_seed{}.svg", c.name, layout["seed"]);
                std::fs::write(out.join(&file), d.to_svg(10.0, &format!("{} seed {}: floor shaded by connected area; orange doors, blue open doorways; red = problems", c.name, layout["seed"]))).unwrap();
                let _ = writeln!(md, "Picture of the first level: [{file}]({file})\n");
            }
        }
        summary.sort_by_key(|s| s.0);
        let _ = writeln!(md, "### Levels\n\n| Seed | Rooms reached | Impassable joins | Doors (cutting) | Welded joins (widest) |\n|---|---|---|---|---|");
        for (seed, r) in &summary {
            let _ = writeln!(md, "| {seed} | {}/{} | {} | {} ({}) | {} ({} mm) |", r.reachable_rooms, r.rooms, r.impassable_joins.len(), r.doors, r.cutting_doors.len(), r.welded, r.widest_weld_mm);
        }
        md.push('\n');
    }
    std::fs::write(out.join("report.md"), &md).unwrap();
    println!("wrote {}", out.join("report.md").display());
}
