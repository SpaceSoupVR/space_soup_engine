//! Prints each real doorway's walkable span for an agent config, and the clear opening that
//! implies (span + 2 x radius).
//!
//!   `cargo run --release --example doorway_widths [config]`

use space_soup_nav::asset::Dir;
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, SocketSpec};
use space_soup_nav::bake::{Partition, Settings};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;

fn main() {
    let want = std::env::args().nth(1).unwrap_or_else(|| "human_cs10_monotone".into());
    let data = golden::test_data_dir();
    let c = golden::load_configs(&data).unwrap().configs.into_iter().find(|c| c.name == want).expect("no such config");
    let s = Settings {
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
    let dir = data.join("meshes/blacksite");
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("pieces.json")).unwrap()).unwrap();
    for p in index["pieces"].as_array().unwrap() {
        let name = p["piece"].as_str().unwrap();
        let mesh = TriMesh::load(&dir.join(format!("{name}.obj"))).unwrap();
        let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(glam::Vec3::to_array).collect();
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
        let (asset, _) = bake_piece(&input, &s, [1.0; 3], &ExportOptions::default()).unwrap();
        for sk in &asset.sockets {
            let span = (sk.b - sk.a).abs().max_element() as f32 / 1000.0;
            if sk.edges.is_empty() {
                println!("{name:<48} {:<24} blocked", sk.name);
            } else {
                println!("{name:<48} {:<24} walkable {span:.2} m  -> clear opening about {:.2} m", sk.name, span + 2.0 * c.agent_radius);
            }
        }
    }
}
