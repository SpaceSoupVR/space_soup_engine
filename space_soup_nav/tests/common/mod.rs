//! Shared by the integration tests: the agent configs as baker settings, the real kit baked,
//! and generated levels.
#![allow(dead_code)]

use std::sync::Arc;

use glam::IVec3;
use space_soup_nav::asset::{Dir, NavPieceAsset, Turn};
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, SocketSpec};
use space_soup_nav::bake::{Partition, Settings};
use space_soup_nav::golden::{self, GoldenConfig};
use space_soup_nav::obj::TriMesh;
use space_soup_nav::world::{NavWorld, SocketRef};

pub fn settings(c: &GoldenConfig) -> Settings {
    Settings {
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
    }
}

pub fn config(name: &str) -> GoldenConfig {
    golden::load_configs(&golden::test_data_dir()).unwrap().configs.into_iter().find(|c| c.name == name).unwrap()
}

#[derive(serde::Deserialize)]
pub struct KitIndex {
    pub pieces: Vec<KitPiece>,
}
#[derive(serde::Deserialize)]
pub struct KitPiece {
    pub piece: String,
    pub doorways: Vec<KitDoorway>,
}
#[derive(serde::Deserialize)]
pub struct KitDoorway {
    pub name: String,
    pub position: [f32; 3],
    pub outward: Dir,
    pub size: String,
}

pub fn kit_index() -> KitIndex {
    let dir = golden::test_data_dir().join("meshes/blacksite");
    serde_json::from_str(&std::fs::read_to_string(dir.join("pieces.json")).unwrap()).unwrap()
}

/// Every kit piece baked for one agent config, in kit order (a layout's `kind`).
pub fn bake_kit(config_name: &str) -> Vec<Arc<NavPieceAsset>> {
    let dir = golden::test_data_dir().join("meshes/blacksite");
    let s = settings(&config(config_name));
    kit_index()
        .pieces
        .iter()
        .map(|p| {
            let mesh = TriMesh::load(&dir.join(format!("{}.obj", p.piece))).unwrap();
            let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
            let sockets: Vec<SocketSpec> = p
                .doorways
                .iter()
                .map(|d| SocketSpec { name: d.name.clone(), position: d.position, outward: d.outward, size_class: d.size.clone() })
                .collect();
            let input = PieceInput { name: &p.piece, vertices: &verts, triangles: &mesh.triangles, sockets: &sockets, links: Vec::new() };
            Arc::new(bake_piece(&input, &s, [1.0; 3], &ExportOptions::default()).unwrap().0)
        })
        .collect()
}

#[derive(serde::Deserialize)]
pub struct Layout {
    pub seed: u32,
    pub pieces: Vec<LayoutPiece>,
    pub unsealed: usize,
}
#[derive(serde::Deserialize)]
pub struct LayoutPiece {
    pub kind: usize,
    pub position: [f32; 3],
    pub quarter_turns: u8,
    pub links: Vec<Option<[usize; 2]>>,
}

pub fn layouts() -> Vec<Layout> {
    let dir = golden::test_data_dir().join("layouts");
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    names.iter().map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()).collect()
}

pub fn mm(v: [f32; 3]) -> IVec3 {
    IVec3::new((v[0] * 1000.0).round() as i32, (v[1] * 1000.0).round() as i32, (v[2] * 1000.0).round() as i32)
}

/// Builds the world for a layout, turning the whole level by `extra_turns` about the origin.
/// DoorType doorways get doors; OPEN ones do not.
pub fn assemble(kit: &[Arc<NavPieceAsset>], layout: &Layout, extra_turns: u8) -> NavWorld {
    let mut w = NavWorld::new();
    let t = Turn(extra_turns);
    for p in &layout.pieces {
        w.add_island(kit[p.kind].clone(), Turn((p.quarter_turns + extra_turns) % 4), t.apply(mm(p.position)));
    }
    for (i, p) in layout.pieces.iter().enumerate() {
        for (s, l) in p.links.iter().enumerate() {
            let Some([j, t2]) = *l else { continue };
            if (j, t2) <= (i, s) {
                continue;
            }
            let a = SocketRef { island: i as u32, socket: s as u16 };
            let b = SocketRef { island: j as u32, socket: t2 as u16 };
            let names = (&kit[p.kind].sockets[s].name, &kit[layout.pieces[j].kind].sockets[t2].name);
            let floors = !kit[p.kind].polys.is_empty() && !kit[layout.pieces[j].kind].polys.is_empty();
            let door = (floors && !names.0.contains("OPEN") && !names.1.contains("OPEN")).then_some("door");
            w.connect(a, b, door).unwrap_or_else(|e| panic!("seed {} join {i}.{s} - {j}.{t2}: {e:?}", layout.seed));
        }
    }
    w
}
