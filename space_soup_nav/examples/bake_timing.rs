//! Times the baker on every golden mesh and config, stage by stage.
//!
//!   `cargo run --release --example bake_timing [filter]`

use std::time::Instant;

use space_soup_nav::bake::{self, Partition, Settings};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let data = golden::test_data_dir();
    let configs = golden::load_configs(&data).unwrap().configs;
    for (file, g) in golden::load_all(&data).unwrap() {
        if !file.contains(&filter) {
            continue;
        }
        let c = configs.iter().find(|c| c.name == g.config_name).unwrap();
        let mesh = TriMesh::load(&data.join("meshes").join(&g.input.mesh)).unwrap();
        let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(glam::Vec3::to_array).collect();
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
        fixes: space_soup_nav::bake::Fixes::recast(),
        };
        let t = Instant::now();
        let b = bake::build(&verts, &mesh.triangles, &s);
        println!("{:>8.1} ms  {:<70} {:?}", t.elapsed().as_secs_f64() * 1000.0, file, b.timings);
    }
}
