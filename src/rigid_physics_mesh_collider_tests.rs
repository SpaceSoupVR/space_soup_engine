//! A model used as its own collider (`terrain_collider`) has to collide where it is drawn:
//! stretched along the MODEL's axes, then turned. Models exported Z-up carry a -90 degree X
//! rotation on their mesh node, so stretching the object's Z and turning it is the common case,
//! not an edge case -- and it is what a modular room kit does to every piece.
//!
//! The model here is built in the test: one wall, authored Z-up behind a -90 degree X node
//! rotation exactly as Blender/Sketchfab exports are. It stands 3.9 m ahead along +Z.

use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};

use crate::rigid_physics::PhysicsWorld;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::Scene;

/// A closed box, wound outward, as glb bytes: positions (f32x3) then u32 indices.
fn box_glb(min: Vec3, max: Vec3, node_rotation: Quat) -> Vec<u8> {
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    };
    let positions: Vec<Vec3> = (0..8).map(corner).collect();
    let centre = (min + max) * 0.5;
    let faces = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
    let mut indices: Vec<u32> = Vec::new();
    for f in faces {
        for tri in [[f[0], f[1], f[2]], [f[0], f[2], f[3]]] {
            let (a, b, c) = (positions[tri[0]], positions[tri[1]], positions[tri[2]]);
            let normal = (b - a).cross(c - a);
            let outward = (a + b + c) / 3.0 - centre;
            let tri = if normal.dot(outward) >= 0.0 { tri } else { [tri[0], tri[2], tri[1]] };
            indices.extend(tri.iter().map(|&i| i as u32));
        }
    }

    let mut bin: Vec<u8> = Vec::new();
    for p in &positions {
        for v in [p.x, p.y, p.z] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let index_offset = bin.len();
    for i in &indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let json = serde_json::json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "rotation": [node_rotation.x, node_rotation.y, node_rotation.z, node_rotation.w] }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1 }] }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": index_offset },
            { "buffer": 0, "byteOffset": index_offset, "byteLength": bin.len() - index_offset }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3", "min": [min.x, min.y, min.z], "max": [max.x, max.y, max.z] },
            { "bufferView": 1, "componentType": 5125, "count": indices.len(), "type": "SCALAR" }
        ]
    });
    let mut json = serde_json::to_vec(&json).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    glb
}

/// A folder holding `wall.glb`: a wall 3.9 m ahead along +Z once its node's -90 degree X
/// rotation is applied (Z-up: the mesh's Y is the room's depth, its Z is height).
pub(crate) fn wall_model_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("space_soup_mesh_collider_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let z_up_to_y_up = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let glb = box_glb(Vec3::new(-2.0, -4.1, 0.0), Vec3::new(2.0, -3.9, 3.0), z_up_to_y_up);
    std::fs::write(dir.join("wall.glb"), glb).unwrap();
    dir
}

/// Distance to the wall straight ahead along the object's own +Z, for the object stretched
/// `stretch` and turned `quarter_turns` x 90 degrees about up.
fn wall_ahead(dir: &Path, stretch: Vec3, quarter_turns: u32) -> Option<f32> {
    let turn = Quat::from_rotation_y(quarter_turns as f32 * std::f32::consts::FRAC_PI_2);
    let scene: Scene = serde_json::from_value(serde_json::json!({
        "name": "mesh_collider_test",
        "objects": [{
            "id": "wall",
            "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [1.0, 1.0, 1.0], "rotation": [turn.x, turn.y, turn.z, turn.w] },
            "mesh": { "path": "wall.glb", "scale": [stretch.x, stretch.y, stretch.z] },
            "terrain_collider": {}
        }]
    }))
    .unwrap();
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, dir);
    let origin = Vec3::new(0.0, 1.5 * stretch.y, 0.0);
    physics.raycast(origin, turn * Vec3::Z, 100.0).map(|(hit, _)| hit.distance(origin))
}

#[test]
fn a_model_collider_is_where_the_model_is_drawn() {
    let _physx = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = wall_model_dir();
    let near = |got: Option<f32>, want: f32, what: &str| {
        let got = got.unwrap_or_else(|| panic!("{what}: the wall was not hit"));
        assert!((got - want).abs() < 0.01, "{what}: wall at {got:.3} m, drawn at {want:.3} m");
    };
    near(wall_ahead(&dir, Vec3::ONE, 0), 3.9, "as authored");
    near(wall_ahead(&dir, Vec3::new(1.0, 1.0, 2.0), 0), 7.8, "stretched 2x along its own Z");
    near(wall_ahead(&dir, Vec3::new(1.0, 1.0, 2.0), 1), 7.8, "stretched, turned 90 degrees");
    near(wall_ahead(&dir, Vec3::new(1.0, 1.0, 2.0), 2), 7.8, "stretched, turned 180 degrees");
    near(wall_ahead(&dir, Vec3::new(1.0, 1.0, 2.0), 3), 7.8, "stretched, turned 270 degrees");
    near(wall_ahead(&dir, Vec3::new(1.0, 0.8, 1.0), 1), 3.9, "squashed to 0.8 tall, turned 90 degrees");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The test wall (a mesh collider, the way room models collide) plus a static box at +X and a
/// dynamic box at -X, for the sweep and contact queries a pushing hand needs.
fn query_world(dir: &Path) -> PhysicsWorld {
    let scene: Scene = serde_json::from_value(serde_json::json!({
        "name": "query_test",
        "objects": [
            {
                "id": "wall",
                "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [1.0, 1.0, 1.0], "rotation": [0.0, 0.0, 0.0, 1.0] },
                "mesh": { "path": "wall.glb", "scale": [1.0, 1.0, 1.0] },
                "terrain_collider": {}
            },
            {
                "id": "pillar",
                "cuboid": { "position": [6.0, 1.0, 0.0], "half_size": [0.5, 0.5, 0.5], "rotation": [0.0, 0.0, 0.0, 1.0] },
                "rigid_body": { "mode": "Static" }
            },
            {
                "id": "crate",
                "cuboid": { "position": [-6.0, 1.0, 0.0], "half_size": [0.5, 0.5, 0.5], "rotation": [0.0, 0.0, 0.0, 1.0] },
                "rigid_body": { "mode": "Dynamic" }
            }
        ]
    }))
    .unwrap();
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, dir);
    physics
}

#[test]
fn a_swept_sphere_stops_at_the_first_surface_and_says_whether_it_is_fixed() {
    let _physx = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = wall_model_dir();
    let physics = query_world(&dir);

    let wall = physics.sphere_sweep(Vec3::new(0.0, 1.5, 0.0), 0.1, Vec3::Z, 20.0).expect("the wall");
    assert!((wall.distance - 3.8).abs() < 0.01, "wall at {}", wall.distance);
    assert!(wall.normal.distance(Vec3::NEG_Z) < 0.01, "normal {}", wall.normal);
    assert!(wall.fixed, "a level's mesh collider is world geometry");

    let pillar = physics.sphere_sweep(Vec3::new(0.0, 1.0, 0.0), 0.1, Vec3::X, 20.0).expect("the pillar");
    assert!((pillar.distance - 5.4).abs() < 0.01, "pillar at {}", pillar.distance);
    assert!(pillar.fixed);

    let crate_hit = physics.sphere_sweep(Vec3::new(0.0, 1.0, 0.0), 0.1, Vec3::NEG_X, 20.0).expect("the crate");
    assert!((crate_hit.distance - 5.4).abs() < 0.01, "crate at {}", crate_hit.distance);
    assert!(!crate_hit.fixed, "a dynamic body is not something to push off");

    assert!(physics.sphere_sweep(Vec3::new(0.0, 1.0, 0.0), 0.1, Vec3::NEG_Z, 20.0).is_none(), "nothing behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hand_near_a_room_mesh_finds_the_wall_and_which_way_is_out() {
    let _physx = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = wall_model_dir();
    let physics = query_world(&dir);

    let hand = Vec3::new(0.3, 1.5, 3.8);
    let contacts = physics.sphere_contacts(hand, 0.2);
    let nearest = contacts.first().expect("the wall is within reach");
    assert!((nearest.distance - 0.1).abs() < 0.005, "distance {}", nearest.distance);
    assert!(nearest.point.distance(Vec3::new(0.3, 1.5, 3.9)) < 0.005, "closest point {}", nearest.point);
    assert!(nearest.normal.distance(Vec3::NEG_Z) < 0.01, "out of the wall is -Z, got {}", nearest.normal);
    assert!(nearest.fixed);

    let inside = physics.sphere_contacts(Vec3::new(6.0, 1.0, 0.0), 0.1);
    assert!(inside.first().is_some_and(|c| c.distance == 0.0 && c.normal == Vec3::ZERO), "{inside:?}");

    let near_crate = physics.sphere_contacts(Vec3::new(-5.4, 1.0, 0.0), 0.2);
    assert!(near_crate.first().is_some_and(|c| !c.fixed && (c.distance - 0.1).abs() < 0.005), "{near_crate:?}");

    assert!(physics.sphere_contacts(Vec3::new(0.0, 1.5, 0.0), 0.2).is_empty(), "nothing within reach in the open");
    let _ = std::fs::remove_dir_all(&dir);
}
