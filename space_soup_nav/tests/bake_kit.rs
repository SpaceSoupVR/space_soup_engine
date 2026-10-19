//! Bakes every real room piece for every agent config and checks what the game needs from it:
//! a valid asset that round-trips, and doorways that reach their socket planes.

use space_soup_nav::asset::{Dir, EdgeLink, NavPieceAsset};
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, SocketSpec};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;

mod common;
use common::settings;

#[derive(serde::Deserialize)]
struct Pieces {
    pieces: Vec<Piece>,
}
#[derive(serde::Deserialize)]
struct Piece {
    piece: String,
    doorways: Vec<Doorway>,
}
#[derive(serde::Deserialize)]
struct Doorway {
    name: String,
    position: [f32; 3],
    outward: Dir,
    size: String,
}

#[test]
fn every_piece_bakes_to_a_valid_asset_whose_doorways_reach_their_sockets() {
    let data = golden::test_data_dir();
    let dir = data.join("meshes/blacksite");
    let index: Pieces = serde_json::from_str(&std::fs::read_to_string(dir.join("pieces.json")).unwrap()).unwrap();
    let configs = golden::load_configs(&data).unwrap().configs;
    let mut problems = Vec::new();
    for piece in &index.pieces {
        let mesh = TriMesh::load(&dir.join(format!("{}.obj", piece.piece))).unwrap();
        let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
        let sockets: Vec<SocketSpec> = piece
            .doorways
            .iter()
            .map(|d| SocketSpec { name: d.name.clone(), position: d.position, outward: d.outward, size_class: d.size.clone() })
            .collect();
        for c in &configs {
            let input = PieceInput { name: &piece.piece, vertices: &verts, triangles: &mesh.triangles, sockets: &sockets, links: Vec::new() };
            let (asset, report) = match bake_piece(&input, &settings(c), [1.0; 3], &ExportOptions::default()) {
                Ok(r) => r,
                Err(e) => {
                    problems.push(format!("{} / {}: {e}", piece.piece, c.name));
                    continue;
                }
            };
            println!(
                "{:<48} {:<20} recast {:>3} -> cut {:>3} -> final {:>3} polys, {:>2} with detail, socket edges {:?}, {:.1} + {:.1} ms",
                piece.piece, c.name, report.recast_polys, report.after_cut, report.final_polys, report.detail_polys, report.socket_edges, report.build_ms, report.export_ms
            );
            let bytes = asset.to_bytes();
            let back = NavPieceAsset::from_bytes(&bytes).unwrap();
            assert_eq!(back, asset);
            // Baking is deterministic: the same input gives the same bytes, so a content hash
            // identifies a bake.
            let (again, _) = bake_piece(&input, &settings(c), [1.0; 3], &ExportOptions::default()).unwrap();
            assert_eq!(again.to_bytes(), bytes, "{} / {}: two bakes differ", piece.piece, c.name);

            // Socket edges lie exactly on their plane, at the marker's floor height.
            for (k, s) in asset.sockets.iter().enumerate() {
                let axis = if matches!(s.outward, Dir::PosX | Dir::NegX) { 0 } else { 2 };
                let plane = (sockets[k].position[axis] * 1000.0).round() as i32;
                for &(p, e) in &s.edges {
                    let poly = &asset.polys[p as usize];
                    assert_eq!(poly.edges[e as usize], EdgeLink::Socket(k as u16));
                    for v in [poly.verts[e as usize], poly.verts[(e as usize + 1) % poly.verts.len()]] {
                        let v = asset.vertices[v as usize];
                        assert_eq!(v[axis], plane, "{}: socket {} edge off its plane", piece.piece, s.name);
                        assert_eq!(v.y, (sockets[k].position[1] * 1000.0).round() as i32);
                    }
                }
            }
            // A human-sized agent must get through every doorway of every real room.
            if c.agent_radius <= 0.4 && !piece.piece.contains("NO_PATH") {
                for (k, n) in report.socket_edges.iter().enumerate() {
                    if *n == 0 {
                        problems.push(format!("{} / {}: doorway {} is not reached", piece.piece, c.name, piece.doorways[k].name));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
