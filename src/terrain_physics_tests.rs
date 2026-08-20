//! Does sculpted ground actually hold something up?
//!
//! A collider that compiles and a collider that catches a falling body are
//! different claims, and the gap between them is where "the level has no floor"
//! lives. This drops a ball onto a heightfield with a known hill and asserts
//! where it comes to rest.

use std::collections::HashMap;
use std::io::Write;

use glam::Vec3;
use space_soup_protocol::PlayerId;

use crate::events::InputFrame;
use crate::rig::PlayerRig;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

/// A 33x33 field over 32m, flat at 0 except a plateau 10m high in the middle.
fn write_test_terrain(path: &std::path::Path, plateau_height_frac: f32) {
    let n = 33usize;
    let mut bytes = Vec::with_capacity(n * n * 2);
    for iz in 0..n {
        for ix in 0..n {
            let in_plateau = (12..=20).contains(&ix) && (12..=20).contains(&iz);
            let v = if in_plateau {
                (plateau_height_frac * u16::MAX as f32) as u16
            } else {
                0
            };
            bytes.write_all(&v.to_le_bytes()).unwrap();
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn scene_json() -> String {
    // Terrain spans -16..16 on x/z, heights 0..20m. The plateau is the middle
    // ninth, so a ball dropped at the origin lands on it and a ball dropped near
    // the corner lands on the flat.
    r#"{
        "name": "test",
        "terrain": {
            "kind": "heightfield",
            "path": "terrain/test.r16",
            "resolution": [33, 33],
            "size": [32.0, 32.0],
            "height_range": [0.0, 20.0],
            "origin": [-16.0, 0.0, -16.0]
        },
        "objects": [
            {
                "id": "ball",
                "cuboid": { "position": [0.0, 18.0, 0.0], "half_size": [0.25, 0.25, 0.25] },
                "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 }
            },
            {
                "id": "outer_ball",
                "cuboid": { "position": [-13.0, 18.0, -13.0], "half_size": [0.25, 0.25, 0.25] },
                "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 }
            }
        ]
    }"#
    .to_string()
}

fn build_runtime(dir: &std::path::Path, plateau_frac: f32) -> GameRuntime {
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
    )
    .unwrap();
    std::fs::write(scenes.join("test.json"), scene_json()).unwrap();
    write_test_terrain(&dir.join("terrain/test.r16"), plateau_frac);
    GameRuntime::load(dir).unwrap()
}

fn settle(runtime: &mut GameRuntime) {
    let inputs: HashMap<PlayerId, _> =
        one_player(PlayerId::default(), frame(PlayerRig::default(), InputFrame::default()));
    for _ in 0..240 {
        runtime.update(1.0 / 60.0, &inputs);
    }
}

fn y_of(runtime: &GameRuntime, id: &str) -> f32 {
    runtime.scene().find_object(id).unwrap().cuboid.position.y
}

#[test]
fn a_ball_lands_on_the_sculpted_plateau_instead_of_falling_through() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join("ss_terrain_phys_plateau");
    let _ = std::fs::remove_dir_all(&dir);
    let mut runtime = build_runtime(&dir, 0.5); // plateau at 10m

    let start = y_of(&runtime, "ball");
    settle(&mut runtime);
    let resting = y_of(&runtime, "ball");

    assert!(resting < start, "the ball should have fallen at all");
    assert!(
        resting > 9.0,
        "the ball fell through the 10m plateau and came to rest at {resting}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_scene_holds_a_ball_up_at_the_low_ground_too() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join("ss_terrain_phys_flat");
    let _ = std::fs::remove_dir_all(&dir);
    let mut runtime = build_runtime(&dir, 0.5);

    settle(&mut runtime);
    let resting = y_of(&runtime, "outer_ball");

    // Away from the plateau the ground is at 0, so it should rest just above it
    // -- and crucially NOT keep falling, which is what a missing collider looks
    // like and what a collider cooked in the wrong place also looks like.
    assert!(
        (-0.5..1.5).contains(&resting),
        "off-plateau ball rested at {resting}, expected near ground level"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raising_the_plateau_raises_where_the_ball_stops() {
    // Proves the collider follows the SAMPLES rather than being any flat plane
    // that happens to be in the right place.
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let low_dir = std::env::temp_dir().join("ss_terrain_phys_low");
    let _ = std::fs::remove_dir_all(&low_dir);
    let mut low = build_runtime(&low_dir, 0.25); // 5m
    settle(&mut low);
    let low_y = y_of(&low, "ball");
    // physx-sys's Foundation is a process-wide singleton, so the first runtime
    // has to be gone before the second can create one -- holding both alive
    // fails with "Create Foundation returned a null pointer".
    drop(low);
    let _ = std::fs::remove_dir_all(&low_dir);

    let high_dir = std::env::temp_dir().join("ss_terrain_phys_high");
    let _ = std::fs::remove_dir_all(&high_dir);
    let mut high = build_runtime(&high_dir, 0.75); // 15m
    settle(&mut high);
    let high_y = y_of(&high, "ball");
    drop(high);
    let _ = std::fs::remove_dir_all(&high_dir);

    assert!(
        high_y > low_y + 5.0,
        "a taller plateau should stop the ball higher: low={low_y}, high={high_y}"
    );
}

#[test]
fn a_missing_terrain_file_warns_rather_than_failing_the_load() {
    // A level that opens with nothing to stand on is diagnosable; a runtime that
    // refuses to start is not.
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join("ss_terrain_phys_missing");
    let _ = std::fs::remove_dir_all(&dir);
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
    )
    .unwrap();
    std::fs::write(scenes.join("test.json"), scene_json()).unwrap();
    // deliberately no terrain/test.r16

    assert!(GameRuntime::load(&dir).is_ok(), "a missing heightfield must not abort the load");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cooking a terrain collider and then tearing the world down must not crash.
///
/// This is the test that found the drop-order fault: `terrain_meshes` was
/// declared after `foundation`, so the cooked meshes were released against a
/// destroyed foundation and the process died with SIGSEGV *after* the test body
/// had finished. Cooking itself was fine, which is why it reads as a mystery
/// crash rather than a bug in the collider.
///
/// Worth keeping separate from the ball tests: those would also fail, but they
/// would fail long after the interesting moment and blame the wrong thing.
#[test]
fn cooking_terrain_and_dropping_the_world_does_not_crash() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join("ss_terrain_teardown");
    let _ = std::fs::remove_dir_all(&dir);
    {
        let _runtime = build_runtime(&dir, 0.5);
    } // dropped here: scene, then terrain_meshes, then foundation
    let _ = std::fs::remove_dir_all(&dir);
}
