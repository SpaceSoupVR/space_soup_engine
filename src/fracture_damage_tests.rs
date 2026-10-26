use glam::Vec3;

use crate::runtime::GameRuntime;
use crate::runtime_test_support::PHYSX_TEST_LOCK;

fn scene_json() -> String {
    let chunks: Vec<String> = (0..5)
        .map(|i| {
            format!(
                r#"{{
                  "id": "wall_chunk_{i}",
                  "cuboid": {{ "position": [{i}.0, 1.0, 0.0], "half_size": [0.5, 1.0, 0.25] }},
                  "rigid_body": {{ "mode": "Static", "shape": "Box" }},
                  "breakable": {{
                    "health": 100.0,
                    "stages": [{{ "at": 1.0, "removed": true }}]
                  }}
                }}"#
            )
        })
        .collect();
    format!(
        r#"{{ "name": "frac_test", "objects": [{}] }}"#,
        chunks.join(",")
    )
}

fn runtime(tag: &str) -> GameRuntime {
    let dir = std::env::temp_dir().join(format!("ss_frac_{tag}_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(scenes.join("frac_test.json"), scene_json()).expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"frac","version":"0.1.0","entry_scene":"frac_test","scenes":["frac_test"]}"#,
    )
    .expect("write manifest");
    GameRuntime::load(&dir).expect("runtime loads")
}

fn drawn(rt: &GameRuntime, id: &str) -> bool {
    rt.collect_render_cuboids().iter().any(|c| c.id == id)
}

fn ray_hits(rt: &GameRuntime, x: f32) -> bool {
    rt.rigid_physics
        .raycast(Vec3::new(x, 1.0, -5.0), Vec3::Z, 10.0)
        .is_some()
}

#[test]
fn a_round_destroys_the_chunk_it_hit_and_leaves_its_neighbours() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = runtime("one_chunk");

    for x in 0..5 {
        assert!(drawn(&rt, &format!("wall_chunk_{x}")), "wall starts whole");
    }

    let changes = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 100.0);
    assert_eq!(changes.len(), 1, "one round, one chunk: {changes:?}");
    assert_eq!(changes[0].object_id, "wall_chunk_2");

    assert!(!drawn(&rt, "wall_chunk_2"), "the chunk that was shot is gone");
    for x in [0, 1, 3, 4] {
        assert!(
            drawn(&rt, &format!("wall_chunk_{x}")),
            "chunk {x} was not hit and must still be there"
        );
    }
}

#[test]
fn the_hole_is_actually_open() {

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = runtime("hole_open");

    assert!(ray_hits(&rt, 2.0), "the intact wall stops a ray");
    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 100.0);

    assert!(!ray_hits(&rt, 2.0), "and the hole lets one through");
    assert!(ray_hits(&rt, 1.0), "while the wall beside it still stops one");
}

#[test]
fn a_round_that_does_not_finish_a_chunk_changes_nothing_visible() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = runtime("chipped");

    let changes = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    assert!(changes.is_empty(), "no threshold crossed, nothing to replicate");
    assert!(drawn(&rt, "wall_chunk_2"), "still standing");
    assert_eq!(rt.damage_taken("wall_chunk_2"), 30.0, "but it remembers");

    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    let last = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    assert_eq!(last.len(), 1, "the fourth round takes it out");
    assert!(!drawn(&rt, "wall_chunk_2"));
}

#[test]
fn a_blast_opens_a_hole_and_only_chips_the_edges() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = runtime("blast");

    let changes = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 2.0, 100.0);
    let gone: Vec<&str> = changes.iter().map(|c| c.object_id.as_str()).collect();
    assert_eq!(gone, ["wall_chunk_2"], "only the centre is destroyed outright");

    assert!(
        rt.damage_taken("wall_chunk_1") > 0.0 && rt.damage_taken("wall_chunk_1") < 100.0,
        "its neighbour is damaged but standing: {}",
        rt.damage_taken("wall_chunk_1")
    );
    assert!(
        rt.damage_taken("wall_chunk_1") > rt.damage_taken("wall_chunk_0"),
        "and nearer chunks come off worse"
    );

    assert_eq!(
        rt.damage_taken("wall_chunk_0"),
        rt.damage_taken("wall_chunk_4"),
        "falloff is symmetric about the impact"
    );

    let second = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 2.0, 100.0);
    let widened: Vec<&str> = second.iter().map(|c| c.object_id.as_str()).collect();
    assert_eq!(widened, ["wall_chunk_1", "wall_chunk_3"], "the hole widens");
}

#[test]
fn damage_at_a_point_nothing_stands_at_is_harmless() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = runtime("miss");

    assert!(rt.apply_damage_at(Vec3::new(2.0, 1.0, 40.0), 1.0, 500.0).is_empty());
    for x in 0..5 {
        assert!(drawn(&rt, &format!("wall_chunk_{x}")), "a miss breaks nothing");
    }
}

#[test]
fn a_tighter_blast_leaves_the_far_chunks_alone() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = runtime("blast_tight");

    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.8, 100.0);
    assert!(rt.damage_taken("wall_chunk_1") > 0.0, "the neighbour is caught");
    assert_eq!(rt.damage_taken("wall_chunk_4"), 0.0, "the far end is not");
}

#[test]
fn a_chunk_written_by_the_editor_loads_with_the_shape_it_was_given() {
    let authored = r#"{
      "id": "block_chunk_3",
      "cuboid": {
        "position": [1.5347300000000001, 1.25408, -2.75],
        "half_size": [0.46526999999999996, 0.89059, 0.15000000000000013],
        "rotation": [0, 0, 0, 1],
        "color": [180, 180, 190, 255],
        "wire_color": [120, 200, 255, 255],
        "style": "Solid"
      },
      "breakable": {
        "health": 100,
        "stages": [{ "at": 1, "hidden_parts": [], "solid": false, "removed": true }]
      }
    }"#;

    let obj: crate::scene::GameObject =
        serde_json::from_str(authored).expect("the editor's output parses");

    assert!(
        (obj.cuboid.half_size.x - 0.46527).abs() < 1e-5,
        "half_size survived: {:?}",
        obj.cuboid.half_size
    );

    let b = obj.breakable.as_ref().expect("breakable survived");
    assert_eq!(b.health, 100.0);
    assert!(b.stages[0].removed, "the stage that takes the chunk away");
    assert!(!b.is_solid_at(100.0), "and it stops blocking when it goes");

    let hit = crate::damage::impact_targets(
        std::slice::from_ref(&obj),
        obj.cuboid.position,
        0.0,
    );
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].share, 1.0);
}

fn brush_scene_json() -> String {
    r#"{
      "name": "brush_test",
      "objects": [
        {
          "id": "brush_wall",
          "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [2.0, 1.0, 0.15] },
          "brush": {
            "solids": [
              { "faces": [
                { "plane": [1, 0, 0, 2.0], "material": "concrete" },
                { "plane": [-1, 0, 0, 2.0], "material": "concrete" },
                { "plane": [0, 1, 0, 2.0], "material": "concrete" },
                { "plane": [0, -1, 0, 0.0], "material": "concrete" },
                { "plane": [0, 0, 1, 0.15], "material": "concrete" },
                { "plane": [0, 0, -1, 0.15], "material": "concrete" }
              ] }
            ]
          },
          "breakable": {
            "health": 100.0,
            "stages": [{ "at": 1.0, "removed": true }]
          }
        }
      ]
    }"#
    .to_string()
}

fn brush_runtime(tag: &str) -> GameRuntime {
    let dir = std::env::temp_dir().join(format!("ss_brush_{tag}_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(scenes.join("brush_test.json"), brush_scene_json()).expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"brush","version":"0.1.0","entry_scene":"brush_test","scenes":["brush_test"]}"#,
    )
    .expect("write manifest");
    GameRuntime::load(&dir).expect("runtime loads")
}

#[test]
fn a_brush_is_never_sent_as_a_bounding_box() {

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let rt = brush_runtime("not_a_box");

    assert!(
        !drawn(&rt, "brush_wall"),
        "a brush must not ride the cuboid list -- the client meshes it"
    );
    assert!(
        rt.hidden_brushes().is_empty(),
        "and an intact wall is not in the skip list either"
    );
}

#[test]
fn a_destroyed_brush_is_named_in_the_list_the_client_skips() {

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = brush_runtime("skip_list");

    rt.apply_damage_at(Vec3::new(0.0, 1.0, 0.0), 0.0, 100.0);
    assert_eq!(rt.hidden_brushes(), ["brush_wall"]);
    assert!(!rt.is_solid("brush_wall"), "and it stopped blocking");
}

#[test]
fn a_brush_a_script_hid_is_skipped_too() {

    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rt = brush_runtime("script_hidden");

    rt.script_host.push_command(crate::script::EngineCommand::SetObjectVisible {
        id: "brush_wall".into(),
        visible: false,
    });
    rt.apply_script_commands();

    assert_eq!(rt.hidden_brushes(), ["brush_wall"]);
}
