use std::path::Path;

use glam::Vec3;

use crate::brush::{block_solid, BrushDef};
use crate::locomotion::{Locomotion, LocomotionMode, MAX_WADE_DEPTH};
use crate::rigid_physics::PhysicsWorld;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::{GameObject, Scene};
use crate::water::WaterDef;

fn pool(surface: f32) -> PhysicsWorld {
    let mut scene: Scene = serde_json::from_str(r#"{"name":"water_test","objects":[]}"#).unwrap();
    scene.objects = vec![GameObject {
        id: "ground".into(),
        brush: Some(BrushDef {
            solids: vec![block_solid([-20.0, -1.0, -20.0], [20.0, 0.0, 20.0], "m")],
            subtract: vec![],
        }),
        ..Default::default()
    }];
    let water: WaterDef =
        serde_json::from_value(serde_json::json!({ "height": surface, "bounds": [2.0, -5.0, 10.0, 5.0] })).unwrap();
    scene.water = vec![water];
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    physics
}

fn wade(physics: &PhysicsWorld, from: Vec3, to: Vec3) -> (Vec3, f32) {
    let mut loco = Locomotion::new(LocomotionMode::Smooth);
    loco.player_offset = from;
    let mut deepest = f32::NEG_INFINITY;
    let steps = ((to - from).length() / 0.05).ceil() as usize;
    for i in 1..=steps {
        let start = loco.collision_start(Some(loco.player_offset + Vec3::Y * 1.7));
        let target = from + (to - from) * (i as f32 / steps as f32);
        loco.player_offset.x = target.x;
        loco.player_offset.z = target.z;
        loco.apply_collision(physics, start);
        let p = loco.player_offset;
        if let Some(depth) = physics.water_depth_at(p.x, p.z, p.y) {
            deepest = deepest.max(depth);
        }
    }
    (loco.player_offset, deepest)
}

#[test]
fn a_walk_stops_where_the_water_is_too_deep() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = pool(1.0);
    let (end, deepest) = wade(&physics, Vec3::ZERO, Vec3::new(8.0, 0.0, 0.0));
    assert!(end.x > 1.9 && end.x < 2.0, "a metre of water from x = 2, and the walk ended at x = {}", end.x);
    assert!(deepest <= MAX_WADE_DEPTH, "stood in {deepest} m of water");
}

#[test]
fn shallow_water_is_waded_through() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = pool(0.4);
    let (end, deepest) = wade(&physics, Vec3::ZERO, Vec3::new(8.0, 0.0, 0.0));
    assert!(end.x > 7.99, "stopped at x = {} in water {deepest} m deep", end.x);
    assert!((deepest - 0.4).abs() < 1e-3, "the water was {deepest} m deep, not 0.4");
}

#[test]
fn a_player_out_of_their_depth_can_walk_back_to_the_shore() {

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = pool(1.0);
    let (end, _) = wade(&physics, Vec3::new(6.0, 0.0, 0.0), Vec3::new(-2.0, 0.0, 0.0));
    assert!(end.x < -1.99, "held in the water, at x = {}", end.x);
}

#[test]
fn the_shipped_beach_is_waded_into_and_no_further_and_the_pond_all_the_way() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let game = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
    let scene = Scene::load(&game.join("scenes/test_room.json")).expect("test_room.json loads");
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, game);
    for (what, from, to) in [
        ("the sea", Vec3::new(0.0, 5.0, 24.0), Vec3::new(0.0, 5.0, 60.0)),
    ] {
        let (end, deepest) = wade(&physics, from, to);
        assert!(deepest > MAX_WADE_DEPTH - 0.05, "never waded into {what}: {deepest} m at most, ended at {end:?}");
        assert!(deepest <= MAX_WADE_DEPTH, "waded {deepest} m deep into {what}");
        assert!(end.distance(Vec3::new(to.x, end.y, to.z)) > 2.0, "walked out into the middle of {what}, to {end:?}");
    }
    let to = Vec3::new(-25.8, 5.0, -25.8);
    let (end, deepest) = wade(&physics, Vec3::new(-18.0, 5.0, -17.0), to);
    assert!(end.distance(Vec3::new(to.x, end.y, to.z)) < 0.1, "stopped short of the pond's middle, at {end:?}");
    assert!(deepest > MAX_WADE_DEPTH, "walked in on the pond's bed only {deepest} m deep");
}

/// A body physics drops into the water is seen going in once, as it met the
/// surface: where, how fast (what the fall gives it), how wide, how heavy --
/// what its splash is drawn from. Resting on the bed after, it is not seen
/// again.
#[test]
fn a_body_dropped_into_water_is_reported_once_as_it_met_the_surface() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut scene: Scene = serde_json::from_str(r#"{"name":"water_drop","objects":[]}"#).unwrap();
    let mut body = GameObject { id: "box".into(), ..Default::default() };
    body.cuboid.position = Vec3::new(6.0, 2.2, 0.0);
    body.cuboid.half_size = Vec3::new(0.1, 0.1, 0.15);
    body.rigid_body = Some(serde_json::from_str(r#"{"mode":"Dynamic","shape":"Box","mass":30.0}"#).unwrap());
    scene.objects = vec![
        GameObject {
            id: "ground".into(),
            brush: Some(BrushDef { solids: vec![block_solid([-20.0, -1.0, -20.0], [20.0, 0.0, 20.0], "m")], subtract: vec![] }),
            ..Default::default()
        },
        body,
    ];
    scene.water = vec![serde_json::from_value(serde_json::json!({ "height": 0.5, "bounds": [2.0, -5.0, 10.0, 5.0] })).unwrap()];
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    let rigs = std::collections::HashMap::new();
    let mut entries = Vec::new();
    for _ in 0..240 {
        physics.step(1.0 / 120.0, &mut scene, &rigs);
        entries.extend(physics.take_water_entries());
    }
    assert_eq!(entries.len(), 1, "{entries:?}");
    let e = &entries[0];
    assert_eq!(e.id, "box");
    assert!((e.position - Vec3::new(6.0, 0.5, 0.0)).length() < 0.02, "{e:?}");
    assert!((-e.velocity.y - (2.0f32 * 9.81 * 1.6).sqrt()).abs() < 0.3, "{e:?}");
    assert!((e.mass - 30.0).abs() < 1e-3 && (e.radius - (4.0f32 * 0.1 * 0.15 / std::f32::consts::PI).sqrt()).abs() < 0.02, "{e:?}");
    let rested = scene.find_object("box").unwrap().cuboid.position;
    assert!(rested.y < 0.3, "{rested:?}");
}

/// A shot's line finds the water it meets, and how deep it is there -- and
/// not water behind a wall, nor any looking up.
#[test]
fn a_shot_finds_where_it_meets_the_water() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = pool(1.0);
    let muzzle = Vec3::new(0.0, 2.0, 0.0);
    let (hit, depth) = physics.water_hit(muzzle, Vec3::new(1.0, -0.25, 0.0), 50.0).expect("the shot meets the pool");
    assert!((hit - Vec3::new(4.0, 1.0, 0.0)).length() < 1e-3, "{hit:?}");
    assert!((depth - 1.0).abs() < 0.05, "{depth}");
    assert!(physics.water_hit(muzzle, Vec3::new(1.0, 0.1, 0.0), 50.0).is_none(), "upward");
    assert!(physics.water_hit(muzzle, Vec3::new(-1.0, -0.25, 0.0), 50.0).is_none(), "onto dry ground");
    assert!(physics.water_hit(muzzle, Vec3::new(1.0, -0.25, 0.0), 3.0).is_none(), "out of range");
}

/// A body physics drops onto the level's terrain is told of once, as it
/// landed, at the speed the fall gave it (`ground_hits`): what every headset
/// raises its mud splash from where that ground is mud. Resting after, it is
/// not told again.
#[test]
fn a_body_dropped_onto_the_terrain_is_told_once_at_the_speed_it_fell() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("ss_ground_hit_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("terrain")).unwrap();
    std::fs::write(dir.join("terrain/flat.r16"), vec![0u8; 9 * 9 * 2]).unwrap();
    let mut scene: Scene = serde_json::from_str(
        r#"{"name":"ground_drop","objects":[],"terrain":{"kind":"heightfield","path":"terrain/flat.r16","resolution":[9,9],"size":[8.0,8.0],"height_range":[0.0,10.0],"origin":[-4.0,0.0,-4.0]}}"#,
    )
    .unwrap();
    let mut body = GameObject { id: "crate".into(), ..Default::default() };
    body.cuboid.position = Vec3::new(0.5, 2.0, 0.5);
    body.cuboid.half_size = Vec3::splat(0.25);
    body.rigid_body = Some(serde_json::from_str(r#"{"mode":"Dynamic","shape":"Box","mass":30.0}"#).unwrap());
    scene.objects = vec![body];
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, &dir);
    let rigs = std::collections::HashMap::new();
    let mut hits = Vec::new();
    for _ in 0..360 {
        physics.step(1.0 / 120.0, &mut scene, &rigs);
        hits.extend(physics.take_ground_hits());
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let h = &hits[0];
    assert_eq!((h.id.as_str(), h.kind), ("crate", crate::ground_hits::GroundHitKind::Landing));
    assert!(h.position.y.abs() < 0.02 && (h.position.x - 0.5).abs() < 0.05, "{h:?}");
    assert!((-h.velocity.y - (2.0f32 * 9.81 * 1.75).sqrt()).abs() < 0.5, "{h:?}");
    assert!((h.mass - 30.0).abs() < 1e-3, "{h:?}");
}
