//! Bakes a room kit into navmesh assets: one `<piece>.<class>.ssnav` per piece and agent class,
//! plus `bake_report.json`.
//!
//!   cargo run --release --example bake_kit -- <kit dir> <out dir> [config name ...]
//!
//! `<kit dir>` holds `<piece>.obj` files and `pieces.json` (each piece's doorways), as
//! `blacksite_port`'s `export_nav_meshes` example writes them. Agent classes are configs from
//! `tests/data/golden_configs.json`; by default every one in it.

use std::path::PathBuf;

use space_soup_nav::asset::Dir;
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, SocketSpec};
use space_soup_nav::bake::{Partition, Settings};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;

fn main() {
    let mut args = std::env::args().skip(1);
    let kit = PathBuf::from(args.next().expect("usage: bake_kit <kit dir> <out dir> [config ...]"));
    let out = PathBuf::from(args.next().expect("usage: bake_kit <kit dir> <out dir> [config ...]"));
    let wanted: Vec<String> = args.collect();
    std::fs::create_dir_all(&out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));

    let configs: Vec<_> = golden::load_configs(&golden::test_data_dir())
        .unwrap()
        .configs
        .into_iter()
        .filter(|c| wanted.is_empty() || wanted.contains(&c.name))
        .collect();
    assert!(!configs.is_empty(), "no matching agent configs");
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(kit.join("pieces.json")).unwrap()).unwrap();

    let mut report = Vec::new();
    let mut failed = false;
    for p in index["pieces"].as_array().unwrap() {
        let name = p["piece"].as_str().unwrap();
        let mesh = TriMesh::load(&kit.join(format!("{name}.obj"))).unwrap_or_else(|e| panic!("{e}"));
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
            fixes: space_soup_nav::bake::Fixes::default(),
            };
            let input = PieceInput { name, vertices: &verts, triangles: &mesh.triangles, sockets: &sockets, links: Vec::new() };
            match bake_piece(&input, &settings, [1.0; 3], &ExportOptions::default()) {
                Ok((asset, r)) => {
                    let bytes = asset.to_bytes();
                    let file = out.join(format!("{name}.{}.ssnav", c.name));
                    std::fs::write(&file, &bytes).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
                    println!(
                        "{name:<48} {:<20} {:>3} polys {:>5} bytes  doorways reached {}/{}  {:.1} ms",
                        c.name,
                        r.final_polys,
                        bytes.len(),
                        r.socket_edges.iter().filter(|&&n| n > 0).count(),
                        r.socket_edges.len(),
                        r.build_ms + r.export_ms
                    );
                    report.push(serde_json::json!({
                        "piece": name, "class": c.name, "file": file.file_name().unwrap().to_string_lossy(),
                        "bytes": bytes.len(), "content_hash": format!("{:016x}", asset.content_hash()),
                        "recast_polys": r.recast_polys, "polys": r.final_polys, "detail_polys": r.detail_polys,
                        "socket_edges": r.socket_edges, "ms": r.build_ms + r.export_ms,
                    }));
                }
                Err(e) => {
                    eprintln!("{name} / {}: {e}", c.name);
                    failed = true;
                }
            }
        }
    }
    std::fs::write(out.join("bake_report.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    if failed {
        std::process::exit(1);
    }
}
