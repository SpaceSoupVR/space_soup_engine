use std::path::Path;

use glam::Vec3;

use crate::locomotion::{Locomotion, LocomotionMode};
use crate::rigid_physics::PhysicsWorld;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scatter::{resolve, FlatGround, ScatterInstance};
use crate::scene::Scene;

fn scene_with(kind: &str) -> Scene {
    let json = format!(
        r#"{{"name":"scatter_test","objects":[],"scatter":[{{
            "id":"scatter_{kind}","seed":7,
            "prototypes":[{{"mesh":"models/train/{kind}/thing.glb","scale_range":[0.5,0.5],"max_slope_deg":45.0}}],
            "strokes":[{{"id":0,"center":[0.0,0.0],"radius":8.0,"density":0.05}}]
        }}]}}"#
    );
    serde_json::from_str(&json).unwrap()
}

fn first_placement(scene: &Scene) -> ScatterInstance {
    resolve(&scene.scatter[0], &FlatGround(0.0))
        .into_iter()
        .next()
        .expect("the stroke places at least one thing")
}

fn walk(physics: &PhysicsWorld, from: Vec3, to: Vec3) -> Vec3 {
    let mut loco = Locomotion::new(LocomotionMode::Smooth);
    loco.player_offset = from;
    let steps = ((to - from).length() / 0.05).ceil() as usize;
    for i in 1..=steps {
        let start = loco.collision_start(Some(loco.player_offset + Vec3::Y * 1.7));
        let target = from + (to - from) * (i as f32 / steps as f32);
        loco.player_offset.x = target.x;
        loco.player_offset.z = target.z;
        loco.apply_collision(physics, start);
    }
    loco.player_offset
}

#[test]
fn a_tree_trunk_stops_a_walking_player() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let scene = scene_with("trees");
    let tree = first_placement(&scene);
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));

    let from = tree.position + Vec3::new(-3.0, 0.0, 0.0);
    let to = tree.position + Vec3::new(3.0, 0.0, 0.0);
    let end = walk(&physics, from, to);
    assert!(
        end.x < tree.position.x - 0.2,
        "walked through the trunk: ended at {end:?}, tree at {:?}",
        tree.position
    );
    assert!(end.x > from.x + 1.0, "never got moving: ended at {end:?} from {from:?}");
}

#[test]
fn a_bush_is_walked_through() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let scene = scene_with("bushes");
    let bush = first_placement(&scene);
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));

    let from = bush.position + Vec3::new(-3.0, 0.0, 0.0);
    let to = bush.position + Vec3::new(3.0, 0.0, 0.0);
    let end = walk(&physics, from, to);
    assert!((end - to).length() < 0.05, "a bush blocked the walk: ended at {end:?}, wanted {to:?}");
}
