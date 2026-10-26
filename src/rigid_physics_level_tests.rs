//! What a physics world takes from the LEVEL it is built for, rather than from
//! a constant or from the level before it: the height under which a body has
//! fallen out of the world, and the colliders a scene change leaves behind.

use std::collections::HashMap;
use std::path::Path;

use glam::Vec3;

use crate::rigid_physics::{PhysicsWorld, KILL_MARGIN, KILL_Y_HIGHEST};
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::Scene;

fn scene(objects: &str) -> Scene {
    serde_json::from_str(&format!(r#"{{"name":"level_test","objects":[{objects}]}}"#)).unwrap()
}

fn world_of(scene: &Scene, game_dir: &Path) -> PhysicsWorld {
    let mut physics = PhysicsWorld::new();
    physics.rebuild(scene, game_dir);
    physics
}

fn y_of(scene: &Scene, id: &str) -> f32 {
    scene.find_object(id).unwrap().cuboid.position.y
}

/// A slab whose top is at y = -40, a crate that starts two metres over it,
/// and a second crate out past the slab's edge with nothing under it.
const DEEP_LEVEL: &str = r#"
    { "id": "floor", "cuboid": { "position": [0.0, -40.5, 0.0], "half_size": [4.0, 0.5, 4.0] },
      "rigid_body": { "mode": "Static", "shape": "Box" } },
    { "id": "crate", "cuboid": { "position": [0.0, -38.0, 0.0], "half_size": [0.25, 0.25, 0.25] },
      "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 } },
    { "id": "lost", "cuboid": { "position": [20.0, -38.0, 0.0], "half_size": [0.25, 0.25, 0.25] },
      "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 } }
"#;

/// THE LEVEL'S FLOOR IS ITS OWN. Under the constant -15 every level had, this
/// crate was put back at its start with no speed on every tick -- it began
/// under the kill height -- and hung two metres over the floor for ever.
#[test]
fn a_body_resting_on_ground_forty_metres_down_is_not_respawned() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut scene = scene(DEEP_LEVEL);
    let mut physics = world_of(&scene, Path::new("."));

    let rigs = HashMap::new();
    for _ in 0..180 {
        physics.step(1.0 / 60.0, &mut scene, &rigs);
    }
    let resting = y_of(&scene, "crate");
    assert!(
        (resting - -39.75).abs() < 0.05,
        "the crate should lie on the floor at y = -39.75, and is at {resting}"
    );
    assert_eq!(physics.kill_height(), -41.0 - KILL_MARGIN);
    for _ in 0..60 {
        physics.step(1.0 / 60.0, &mut scene, &rigs);
    }
    assert!((y_of(&scene, "crate") - resting).abs() < 0.01, "the crate moved after it had landed");
}

/// ...and a body that leaves that level still comes back: past the floor's
/// edge it falls under the level, and is put back where it started.
#[test]
fn a_body_that_falls_out_under_a_deep_levels_floor_is_still_respawned() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut scene = scene(DEEP_LEVEL);
    let mut physics = world_of(&scene, Path::new("."));
    let kill = physics.kill_height();

    let rigs = HashMap::new();
    let (mut lowest, mut fell_past_the_floor, mut came_back) = (f32::MAX, false, false);
    for _ in 0..360 {
        physics.step(1.0 / 60.0, &mut scene, &rigs);
        let y = y_of(&scene, "lost");
        lowest = lowest.min(y);
        if y < -42.0 {
            fell_past_the_floor = true;
        }
        if fell_past_the_floor && y > -38.5 {
            came_back = true;
        }
    }
    assert!(fell_past_the_floor, "nothing was under it: it should have fallen past the floor (lowest {lowest})");
    assert!(came_back, "it fell out of the level and was never put back (lowest {lowest})");
    assert!(lowest > kill - 0.5, "it fell to {lowest}, well under the kill height {kill}");
}

/// A level at the origin keeps the height it always had.
#[test]
fn a_level_whose_floor_is_at_the_origin_keeps_the_old_kill_height() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let scene = scene(
        r#"{ "id": "floor", "cuboid": { "position": [0.0, -0.5, 0.0], "half_size": [4.0, 0.5, 4.0] },
             "rigid_body": { "mode": "Static", "shape": "Box" } }"#,
    );
    let mut physics = world_of(&scene, Path::new("."));
    assert_eq!(physics.kill_height(), KILL_Y_HIGHEST);
    assert_eq!(KILL_Y_HIGHEST, -15.0);

    physics.rebuild(&self::scene(DEEP_LEVEL), Path::new("."));
    assert!(physics.kill_height() < -40.0);
    physics.rebuild(&scene, Path::new("."));
    assert_eq!(physics.kill_height(), KILL_Y_HIGHEST);
}

/// SCULPTED GROUND forty metres down holds a body too: the floor is taken from
/// the heightfield's own lowest sample, not from where its box says it could
/// reach.
#[test]
fn the_kill_height_follows_the_lowest_ground_of_a_heightfield() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("ss_level_floor_terrain");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("terrain")).unwrap();
    let n = 9usize;
    let mut bytes = Vec::new();
    for i in 0..n * n {
        let raw: u16 = if i % 7 == 0 { 20000 } else { 19661 };
        bytes.extend_from_slice(&raw.to_le_bytes());
    }
    std::fs::write(dir.join("terrain/deep.r16"), bytes).unwrap();
    let mut scene: Scene = serde_json::from_str(
        r#"{
            "name": "deep",
            "terrain": {
                "kind": "heightfield", "path": "terrain/deep.r16", "resolution": [9, 9],
                "size": [16.0, 16.0], "height_range": [-100.0, 100.0], "origin": [-8.0, 0.0, -8.0]
            },
            "objects": [
                { "id": "crate", "cuboid": { "position": [0.3, -38.0, 0.2], "half_size": [0.25, 0.25, 0.25] },
                  "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 } }
            ]
        }"#,
    )
    .unwrap();
    let mut physics = world_of(&scene, &dir);
    let lowest_ground = -100.0 + 19661.0 / 65535.0 * 200.0;
    assert!((physics.kill_height() - (lowest_ground - KILL_MARGIN)).abs() < 1e-3, "kill height {}", physics.kill_height());

    let rigs = HashMap::new();
    for _ in 0..240 {
        physics.step(1.0 / 60.0, &mut scene, &rigs);
    }
    let resting = y_of(&scene, "crate");
    assert!((-40.0..-38.5).contains(&resting), "the crate should lie on the ground near y = -39.7, and is at {resting}");
    drop(physics);
    let _ = std::fs::remove_dir_all(&dir);
}

fn ray_down_at(physics: &PhysicsWorld, x: f32, z: f32) -> Option<f32> {
    physics.raycast_down(Vec3::new(x, 50.0, z), 100.0).map(|(hit, _)| hit.y)
}

/// A SCENE CHANGE LEAVES NOTHING OF THE LAST SCENE'S STATICS. `rebuild`
/// cleared every list but `statics`, and the spawners skip an id that is
/// still in it: so a wall whose id the next scene also uses was never built
/// there at all -- the old scene's actor went with the old scene, and the
/// stale entry stopped the new one being made -- and `despawn_static` on it
/// released an actor that was already gone.
#[test]
fn a_static_whose_id_the_next_scene_reuses_is_built_where_the_next_scene_puts_it() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let wall_at = |x: f32, half_y: f32| {
        scene(&format!(
            r#"{{ "id": "wall", "cuboid": {{ "position": [{x}, 0.0, 0.0], "half_size": [1.0, {half_y}, 1.0] }},
                 "rigid_body": {{ "mode": "Static", "shape": "Box" }} }}"#
        ))
    };
    let mut physics = world_of(&wall_at(0.0, 1.0), Path::new("."));
    assert_eq!(ray_down_at(&physics, 0.0, 0.0), Some(1.0));
    assert_eq!(ray_down_at(&physics, 10.0, 0.0), None);

    physics.rebuild(&wall_at(10.0, 2.0), Path::new("."));
    assert_eq!(ray_down_at(&physics, 0.0, 0.0), None, "the last scene's wall is still standing");
    assert_eq!(ray_down_at(&physics, 10.0, 0.0), Some(2.0), "the new scene's wall was not built");
    assert!(physics.has_static("wall"));

    assert!(physics.despawn_static("wall"));
    assert_eq!(ray_down_at(&physics, 10.0, 0.0), None);
    assert!(!physics.despawn_static("wall"));

    physics.rebuild(&wall_at(10.0, 2.0), Path::new("."));
    assert_eq!(ray_down_at(&physics, 10.0, 0.0), Some(2.0), "a reloaded scene lost its wall");
}

/// The same for a solid brush, which shares the list.
#[test]
fn a_brush_whose_id_the_next_scene_reuses_is_built_where_the_next_scene_puts_it() {
    use crate::brush::{block_solid, BrushDef};
    use crate::scene::GameObject;

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let block_at = |x: f64| {
        let mut s = scene("");
        s.objects = vec![GameObject {
            id: "block".into(),
            brush: Some(BrushDef { solids: vec![block_solid([x - 1.0, 0.0, -1.0], [x + 1.0, 3.0, 1.0], "m")], subtract: vec![] }),
            ..Default::default()
        }];
        s
    };
    let mut physics = world_of(&block_at(0.0), Path::new("."));
    assert!(ray_down_at(&physics, 0.0, 0.0).is_some_and(|y| (y - 3.0).abs() < 1e-4));

    physics.rebuild(&block_at(10.0), Path::new("."));
    assert_eq!(ray_down_at(&physics, 0.0, 0.0), None, "the last scene's block is still standing");
    assert!(ray_down_at(&physics, 10.0, 0.0).is_some_and(|y| (y - 3.0).abs() < 1e-4), "the new scene's block was not built");
}
