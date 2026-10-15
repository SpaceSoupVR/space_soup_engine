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
