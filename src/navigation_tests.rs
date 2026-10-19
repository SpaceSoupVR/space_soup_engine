//! A scene's navmesh: baked from exactly what players collide with (`rigid_physics_nav`), walked
//! round its obstacles, saved and loaded, and known to be stale when its scene has moved on.

use std::path::Path;

use glam::{Quat, Vec3};

use crate::navigation::{files, SceneNav};
use crate::rigid_physics::PhysicsWorld;
use crate::rigid_physics_mesh_collider_tests::wall_model_dir;
use crate::rigid_physics_nav::CollisionTriangles;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::Scene;

/// A 33 x 33 terrain over 32 m with a 10 m plateau in its middle ninth.
fn write_terrain(dir: &Path) {
    let n = 33usize;
    let mut bytes = Vec::with_capacity(n * n * 2);
    for iz in 0..n {
        for ix in 0..n {
            let plateau = (12..=20).contains(&ix) && (12..=20).contains(&iz);
            bytes.extend_from_slice(&(if plateau { u16::MAX / 2 } else { 0 }).to_le_bytes());
        }
    }
    std::fs::create_dir_all(dir.join("terrain")).unwrap();
    std::fs::write(dir.join("terrain/nav.r16"), bytes).unwrap();
}

/// Every kind of collider a level has: sculpted ground, a static box, a model's own mesh collider
/// (Z-up behind its node's turn, stretched 2x along its own Z and turned 90 degrees: the case the
/// mesh collider fix is about), and a dynamic crate that must not be walked on.
fn every_kind(dir: &Path) -> Scene {
    write_terrain(dir);
    let turn = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    serde_json::from_value(serde_json::json!({
        "name": "nav_kinds",
        "terrain": {
            "kind": "heightfield", "path": "terrain/nav.r16", "resolution": [33, 33],
            "size": [32.0, 32.0], "height_range": [0.0, 20.0], "origin": [-16.0, 0.0, -16.0]
        },
        "objects": [
            { "id": "pillar", "cuboid": { "position": [-9.0, 1.0, 9.0], "half_size": [0.5, 1.0, 0.5], "rotation": [0.0, 0.3826834, 0.0, 0.9238795] },
              "rigid_body": { "mode": "Static" } },
            { "id": "wall", "cuboid": { "position": [-9.0, 0.0, -9.0], "half_size": [1.0, 1.0, 1.0], "rotation": [turn.x, turn.y, turn.z, turn.w] },
              "mesh": { "path": "wall.glb", "scale": [1.0, 1.0, 2.0] }, "terrain_collider": {} },
            { "id": "crate", "cuboid": { "position": [9.0, 0.5, -9.0], "half_size": [0.5, 0.5, 0.5] },
              "rigid_body": { "mode": "Dynamic" } }
        ]
    }))
    .unwrap()
}

/// Where a ray first meets any of `geometry`'s triangles, either side (Moller-Trumbore).
fn first_hit(geometry: &CollisionTriangles, origin: Vec3, dir: Vec3, max: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in &geometry.triangles {
        let [a, b, c] = t.map(|i| Vec3::from(geometry.vertices[i as usize]));
        let (e1, e2) = (b - a, c - a);
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            continue;
        }
        let s = origin - a;
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = dir.dot(q) / det;
        let d = e2.dot(q) / det;
        if (-1e-5..=1.0 + 1e-5).contains(&u) && v >= -1e-5 && u + v <= 1.0 + 1e-5 && d >= 0.0 && d <= max && best.is_none_or(|b| d < b) {
            best = Some(d);
        }
    }
    best
}

#[test]
fn the_navmesh_is_baked_from_exactly_what_players_collide_with() {
    let _physx = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = wall_model_dir();
    let scene = every_kind(&dir);
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, &dir);
    let geometry = physics.static_collision_triangles();
    eprintln!("read back: {:?}, {} triangles, skipped {:?}", geometry.shapes, geometry.triangles.len(), geometry.skipped);
    for kind in ["Box", "Trianglemesh", "Heightfield"] {
        assert!(geometry.shapes.contains_key(kind), "no {kind} read back: {:?}", geometry.shapes);
    }
    let inside_crate = |v: &[f32; 3]| (Vec3::from(*v) - Vec3::new(9.0, 0.5, -9.0)).abs().max_element() < 0.49;
    assert!(!geometry.vertices.iter().any(inside_crate), "the dynamic crate was read back as level geometry");

    let mut rng = 0x9e37_79b9_u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };
    let (mut checked, mut hits, mut wall_hits, mut pillar_hits) = (0, 0, 0, 0);
    while checked < 400 {
        let origin = Vec3::new(next() * 30.0 - 15.0, 0.5 + next() * 14.0, next() * 30.0 - 15.0);
        let aim = match checked % 3 {
            0 => Some(Vec3::new(-9.0 + 7.9, 1.5, -9.0)),
            1 => Some(Vec3::new(-9.0, 1.0, 9.0)),
            _ => None,
        };
        let jitter = Vec3::new(next() - 0.5, next() - 0.5, next() - 0.5) * 3.0;
        let dir = match aim {
            Some(t) => (t + jitter - origin).normalize_or_zero(),
            None => Vec3::new(next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0).normalize_or_zero(),
        };
        if dir == Vec3::ZERO || (origin.x > 5.0 && origin.z < -5.0) {
            continue;
        }
        let under_ground = physics.raycast(Vec3::new(origin.x, 40.0, origin.z), Vec3::NEG_Y, 80.0).is_some_and(|(hit, _)| hit.y > origin.y);
        let inside = physics.sphere_contacts(origin, 0.05).iter().any(|c| c.distance == 0.0);
        if under_ground || inside {
            continue;
        }
        let ours = first_hit(&geometry, origin, dir, 40.0);
        let physx = physics.raycast(origin, dir, 40.0).map(|(hit, _)| hit.distance(origin));
        if physx.is_some_and(|d| (origin + dir * d - Vec3::new(9.0, 0.5, -9.0)).abs().max_element() < 0.6) {
            continue;
        }
        checked += 1;
        match (ours, physx) {
            (Some(a), Some(b)) => {
                hits += 1;
                let at = origin + dir * b;
                if at.y > 0.2 && (at - Vec3::new(-9.0 + 7.9, 1.5, -9.0)).length() < 3.0 {
                    wall_hits += 1;
                }
                if at.y > 0.2 && (at - Vec3::new(-9.0, 1.0, 9.0)).length() < 2.0 {
                    pillar_hits += 1;
                }
                assert!((a - b).abs() < 0.01, "ray from {origin} along {dir}: triangles at {a:.3} m, PhysX at {b:.3} m");
            }
            (None, None) => {}
            (a, b) => panic!("ray from {origin} along {dir}: triangles {a:?}, PhysX {b:?}"),
        }
    }
    eprintln!("{checked} rays checked, {hits} hit something ({wall_hits} the mesh-collider wall, {pillar_hits} the pillar), all where PhysX says");
    assert!(hits > 150 && wall_hits > 30 && pillar_hits > 30, "too few rays hit to mean much: {hits}, wall {wall_hits}, pillar {pillar_hits}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_baked_scene_walks_round_its_walls() {
    let _physx = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let scene: Scene = serde_json::from_value(serde_json::json!({
        "name": "nav_room",
        "objects": [
            { "id": "floor", "cuboid": { "position": [0.0, -0.5, 0.0], "half_size": [10.0, 0.5, 10.0] }, "rigid_body": { "mode": "Static" } },
            { "id": "wall", "cuboid": { "position": [-3.0, 1.5, 0.0], "half_size": [7.0, 1.5, 0.2] }, "rigid_body": { "mode": "Static" } }
        ]
    }))
    .unwrap();
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    let nav = SceneNav::bake(&physics, "nav_room").unwrap();
    eprintln!("nav_room: {:?}", nav.info);
    assert!(nav.info.polygons > 0);

    let (from, to) = (Vec3::new(-6.0, 0.0, -5.0), Vec3::new(-6.0, 0.0, 5.0));
    let path = nav.find_path(from, to).expect("a way round the wall");
    let length: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
    eprintln!("round the wall: {} corners, {length:.1} m (10 m straight through it)", path.len());
    let crossing = path.windows(2).find(|w| (w[0].z < 0.0) != (w[1].z < 0.0)).expect("it crosses z = 0");
    let x = crossing[0].x + (crossing[1].x - crossing[0].x) * (0.0 - crossing[0].z) / (crossing[1].z - crossing[0].z);
    assert!(x > 4.3, "it crossed the wall's line at x = {x:.2}, through the wall");
    assert!(length > 22.0, "{length:.1} m: through the wall, not round it");
    for p in &path {
        assert!(p.y.abs() < 0.15, "a corner off the floor: {p}");
        assert!(!(p.x < 4.0 + 0.35 && p.z.abs() < 0.2 + 0.35), "a corner inside the wall's clearance: {p}");
    }
    assert!(nav.nearest(Vec3::new(2.0, 1.0, 5.0)).is_some_and(|p| p.y.abs() < 0.15));
    assert!(nav.nearest(Vec3::new(2.0, 30.0, 5.0)).is_none(), "a point 30 m up is not on the floor");
    assert!(nav.find_path(from, Vec3::new(40.0, 0.0, 40.0)).is_none(), "a goal off the floor has no path");
}

#[test]
fn a_saved_navmesh_loads_with_its_scene_and_knows_when_the_scene_has_moved_on() {
    let _physx = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("ss_nav_saved_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("scenes")).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"name":"t","version":"0.1.0","entry_scene":"room","scenes":["room"]}"#).unwrap();
    let room = |wall_x: f32| {
        serde_json::json!({
            "name": "room",
            "objects": [
                { "id": "floor", "cuboid": { "position": [0.0, -0.5, 0.0], "half_size": [10.0, 0.5, 10.0] }, "rigid_body": { "mode": "Static" } },
                { "id": "wall", "cuboid": { "position": [wall_x, 1.5, 0.0], "half_size": [7.0, 1.5, 0.2] }, "rigid_body": { "mode": "Static" } }
            ]
        })
    };
    std::fs::write(dir.join("scenes/room.json"), room(-3.0).to_string()).unwrap();

    let runtime = GameRuntime::load(&dir).unwrap();
    assert!(runtime.nav().is_none(), "nothing baked yet");
    let baked = SceneNav::bake(&runtime.rigid_physics, "room").unwrap();
    baked.save(&dir).unwrap();
    let (nav_file, info_file) = files(&dir, "room");
    assert!(nav_file.exists() && info_file.exists());
    drop(runtime);

    let runtime = GameRuntime::load(&dir).unwrap();
    let nav = runtime.nav().expect("the runtime loads the scene's navmesh");
    assert!(!nav.stale);
    assert_eq!(crate::navigation::SceneNavInfo { bake_ms: 0.0, ..nav.info.clone() }, crate::navigation::SceneNavInfo { bake_ms: 0.0, ..baked.info.clone() });
    assert_eq!(nav.world.nearest_poly(glam::DVec3::new(2000.0, 0.0, 5000.0), glam::DVec3::splat(600.0)).map(|n| n.point), baked.world.nearest_poly(glam::DVec3::new(2000.0, 0.0, 5000.0), glam::DVec3::splat(600.0)).map(|n| n.point));
    drop(runtime);

    std::fs::write(dir.join("scenes/room.json"), room(3.0).to_string()).unwrap();
    let runtime = GameRuntime::load(&dir).unwrap();
    assert!(runtime.nav().is_some_and(|n| n.stale), "a navmesh baked for another wall was not noticed");
    drop(runtime);
    let _ = std::fs::remove_dir_all(&dir);
}

