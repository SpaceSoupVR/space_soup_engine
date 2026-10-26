use std::collections::HashMap;
use std::io::Write;

use space_soup_protocol::PlayerId;

use crate::events::InputFrame;
use crate::rig::PlayerRig;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

fn write_test_terrain(path: &std::path::Path, plateau_height_frac: f32) {
    let n = 33usize;
    let mut bytes = Vec::with_capacity(n * n * 2);
    for iz in 0..n {
        for ix in 0..n {
            let in_plateau = (12..=20).contains(&ix) && (12..=20).contains(&iz);
            let v = if in_plateau {
                (plateau_height_frac * f32::from(u16::MAX)) as u16
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
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("ss_terrain_phys_plateau");
    let _ = std::fs::remove_dir_all(&dir);
    let mut runtime = build_runtime(&dir, 0.5);

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
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("ss_terrain_phys_flat");
    let _ = std::fs::remove_dir_all(&dir);
    let mut runtime = build_runtime(&dir, 0.5);

    settle(&mut runtime);
    let resting = y_of(&runtime, "outer_ball");

    assert!(
        (-0.5..1.5).contains(&resting),
        "off-plateau ball rested at {resting}, expected near ground level"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raising_the_plateau_raises_where_the_ball_stops() {

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    let low_dir = std::env::temp_dir().join("ss_terrain_phys_low");
    let _ = std::fs::remove_dir_all(&low_dir);
    let mut low = build_runtime(&low_dir, 0.25);
    settle(&mut low);
    let low_y = y_of(&low, "ball");

    drop(low);
    let _ = std::fs::remove_dir_all(&low_dir);

    let high_dir = std::env::temp_dir().join("ss_terrain_phys_high");
    let _ = std::fs::remove_dir_all(&high_dir);
    let mut high = build_runtime(&high_dir, 0.75);
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

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

    assert!(GameRuntime::load(&dir).is_ok(), "a missing heightfield must not abort the load");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cooking_terrain_and_dropping_the_world_does_not_crash() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("ss_terrain_teardown");
    let _ = std::fs::remove_dir_all(&dir);
    {
        let _runtime = build_runtime(&dir, 0.5);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
