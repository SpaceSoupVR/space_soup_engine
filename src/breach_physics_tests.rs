use glam::Vec3;

use crate::runtime::GameRuntime;
use crate::runtime_test_support::PHYSX_TEST_LOCK;

fn scene_json() -> String {

    r#"{
      "name": "breach_test",
      "objects": [
        {
          "id": "wall",
          "cuboid": {
            "position": [0.0, 1.0, 0.0],
            "half_size": [2.0, 1.0, 0.25],
            "color": [128, 128, 128, 255]
          },
          "rigid_body": { "mode": "Static", "shape": "Box" },
          "breakable": {
            "health": 50.0,
            "stages": [
              { "at": 1.0, "hidden_parts": ["intact"], "solid": false }
            ]
          }
        }
      ]
    }"#
    .to_string()
}

fn runtime(tag: &str) -> GameRuntime {
    let dir = std::env::temp_dir().join(format!("ss_breach_{tag}_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(scenes.join("breach_test.json"), scene_json()).expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"breach","version":"0.1.0","entry_scene":"breach_test","scenes":["breach_test"]}"#,
    )
    .expect("write manifest");

    GameRuntime::load(&dir).expect("runtime loads")
}

fn ray_hits_wall(rt: &GameRuntime) -> bool {
    rt.rigid_physics
        .raycast(Vec3::new(0.0, 1.0, -5.0), Vec3::Z, 20.0)
        .is_some()
}

#[test]
fn a_breached_wall_stops_blocking_a_ray() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("ray");

    assert!(
        rt.rigid_physics.has_static("wall"),
        "the wall should start with a static collider",
    );
    assert!(ray_hits_wall(&rt), "an intact wall must block the ray");
    assert!(rt.is_solid("wall"));

    let change = rt.apply_damage("wall", 60.0).expect("that much damage breaches it");
    assert!(change.breached());

    assert!(!rt.is_solid("wall"), "the wall reports itself breached");
    assert!(
        !rt.rigid_physics.has_static("wall"),
        "and its collider is gone, not merely flagged",
    );
    assert!(
        !ray_hits_wall(&rt),
        "THE point: the ray must now pass through the breach",
    );
}

#[test]
fn damage_short_of_a_breach_leaves_the_collider_alone() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("chipped");

    assert!(rt.apply_damage("wall", 10.0).is_none(), "no stage crossed yet");
    assert!(rt.rigid_physics.has_static("wall"));
    assert!(ray_hits_wall(&rt), "a chipped wall is still a wall");
    assert!(rt.is_solid("wall"));
}

#[test]
fn breaching_is_idempotent() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("idempotent");

    rt.apply_damage("wall", 60.0).expect("first breach");
    assert!(!rt.rigid_physics.has_static("wall"));

    assert!(rt.apply_damage("wall", 60.0).is_none());
    assert!(!rt.rigid_physics.despawn_static("wall"), "nothing left to remove");
}

#[test]
fn reloading_the_scene_restores_the_collider() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("reload");

    rt.apply_damage("wall", 60.0).expect("breach");
    assert!(!ray_hits_wall(&rt));

    rt.load_scene("breach_test").expect("reload");
    assert_eq!(rt.damage_taken("wall"), 0.0, "damage resets with the scene");
    assert!(rt.rigid_physics.has_static("wall"), "and the collider is rebuilt");
    assert!(ray_hits_wall(&rt), "the wall blocks again");
}
