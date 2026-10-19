//! The baker against C++ Recast, stage by stage, on every golden file (`tests/data/golden`).
//! Integer stages must match exactly; a mismatch names the file and the stage.

use space_soup_nav::bake::{self, Partition, Settings};
use space_soup_nav::golden::{self, GoldenConfig};
use space_soup_nav::obj::TriMesh;

fn settings(c: &GoldenConfig) -> Settings {
    Settings {
        cell_size: c.cell_size,
        cell_height: c.cell_height,
        agent_radius: c.agent_radius,
        agent_height: c.agent_height,
        agent_max_climb: c.agent_max_climb,
        agent_max_slope_deg: c.agent_max_slope_deg,
        partition: match c.partition.as_str() {
            "monotone" => Partition::Monotone,
            "layers" => Partition::Layers,
            p => panic!("unknown partition {p}"),
        },
        region_min_size: c.region_min_size,
        region_merge_size: c.region_merge_size,
        edge_max_len: c.edge_max_len,
        edge_max_error: c.edge_max_error,
        verts_per_poly: c.verts_per_poly,
        detail_sample_dist: c.detail_sample_dist,
        detail_sample_max_error: c.detail_sample_max_error,
    fixes: space_soup_nav::bake::Fixes::recast(),
    }
}

#[test]
fn every_stage_matches_recast() {
    let data = golden::test_data_dir();
    let configs = golden::load_configs(&data).unwrap().configs;
    let mut failures = Vec::new();
    let mut compared = 0;
    for (file, g) in golden::load_all(&data).unwrap() {
        let c = configs.iter().find(|c| c.name == g.config_name).unwrap();
        let mesh = TriMesh::load(&data.join("meshes").join(&g.input.mesh)).unwrap();
        let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
        let b = bake::build(&verts, &mesh.triangles, &settings(c));

        let r = &g.recast_config;
        let cfg = &b.config;
        let ours = (cfg.width, cfg.height, cfg.bmin, cfg.bmax, cfg.walkable_height, cfg.walkable_climb, cfg.walkable_radius, cfg.max_edge_len);
        let theirs = (r.width, r.height, r.bmin, r.bmax, r.walkable_height, r.walkable_climb, r.walkable_radius, r.max_edge_len);
        if ours != theirs {
            failures.push(format!("{file}: config {ours:?} != {theirs:?}"));
            continue;
        }
        let Some(st) = &g.stages else { continue };
        compared += 1;
        let pairs = [
            ("heightfield spans", b.counts.heightfield_spans as i64, st.heightfield_spans),
            ("walkable after filters", b.counts.walkable_spans_after_filters as i64, st.walkable_spans_after_filters),
            ("compact spans", b.counts.compact_spans as i64, st.compact_spans),
            ("walkable after erode", b.counts.walkable_after_erode as i64, st.walkable_after_erode),
            ("regions", b.counts.max_regions as i64, st.max_regions as i64),
        ];
        if let Some((stage, ours, theirs)) = pairs.iter().find(|p| p.1 != p.2) {
            failures.push(format!("{file}: {stage}: ours {ours}, Recast {theirs}"));
            continue;
        }
        let conts = &b.contours.conts;
        if conts.len() != st.contours.len() {
            failures.push(format!("{file}: contours: ours {}, Recast {}", conts.len(), st.contours.len()));
            continue;
        }
        if let Some((i, (o, t))) = conts.iter().zip(&st.contours).enumerate().find(|(_, (o, t))| {
            o.reg != t.region || o.area != t.area || o.verts != t.verts || o.rverts.len() as i32 != t.raw_vertex_count
        }) {
            failures.push(format!(
                "{file}: contour {i}: ours reg {} area {} {} verts ({} raw), Recast reg {} area {} {} verts ({} raw)",
                o.reg, o.area, o.verts.len(), o.rverts.len(), t.region, t.area, t.verts.len(), t.raw_vertex_count
            ));
            continue;
        }
        let pm = &b.polymesh;
        let gp = &st.polymesh;
        if pm.verts != gp.verts {
            failures.push(format!("{file}: polymesh vertices: ours {}, Recast {}", pm.verts.len(), gp.verts.len()));
            continue;
        }
        if pm.npolys() != gp.polys.len() {
            failures.push(format!("{file}: polygons: ours {}, Recast {}", pm.npolys(), gp.polys.len()));
            continue;
        }
        if let Some(i) = (0..pm.npolys()).find(|&i| {
            let t = &gp.polys[i];
            pm.poly_verts(i) != t.verts.as_slice() || pm.poly_neis(i) != t.neis.as_slice() || pm.regs[i] != t.region || pm.areas[i] != t.area
        }) {
            let t = &gp.polys[i];
            failures.push(format!(
                "{file}: polygon {i}: ours {:?} neis {:?} reg {}, Recast {:?} neis {:?} reg {}",
                pm.poly_verts(i), pm.poly_neis(i), pm.regs[i], t.verts, t.neis, t.region
            ));
            continue;
        }
        let d = &b.detail;
        let gd = &st.detail;
        if d.meshes != gd.meshes || d.tris != gd.tris || d.verts.len() != gd.verts.len() {
            let first = (0..d.meshes.len().min(gd.meshes.len())).find(|&i| d.meshes[i] != gd.meshes[i]);
            failures.push(format!(
                "{file}: detail: ours {} meshes {} verts {} tris, Recast {} {} {}; first differing polygon {first:?}",
                d.meshes.len(), d.verts.len(), d.tris.len(), gd.meshes.len(), gd.verts.len(), gd.tris.len()
            ));
            continue;
        }
        let worst = d.verts.iter().zip(&gd.verts).map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
        if worst > 0.0 {
            failures.push(format!("{file}: detail vertices differ by up to {worst} m"));
            continue;
        }
    }
    assert_eq!(compared, 66, "every golden file has stages and must be compared");
    assert!(failures.is_empty(), "{} mismatch(es):\n{}", failures.len(), failures.join("\n"));
}
