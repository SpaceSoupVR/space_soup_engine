//! Shooting a fractured wall: the impact picks the chunk, not the author.
//!
//! `apply_damage` answers "this object took 40" and is exactly right when the
//! shooter already knows what it hit. A wall that is fifty convex chunks makes
//! "what was hit" a geometry question, and answering it inside each weapon
//! would mean every weapon carrying its own copy of it.
//!
//! Asserted through the two surfaces that actually matter -- what the renderer
//! is handed, and whether a RAY still stops -- rather than by reading flags
//! back. A chunk whose stage says `removed` and still blocks fire is an
//! invisible wall, and that is the failure this file exists to catch.

use glam::Vec3;

use crate::runtime::GameRuntime;
use crate::runtime_test_support::PHYSX_TEST_LOCK;

/// Five 1m chunks in a row along x at z = 0, each destroyed outright by 100
/// damage -- the shape the editor's Voronoi fracture produces, simplified to a
/// row so a test can name which piece should have gone.
///
/// `half_size` and not `size`. `size` is not a field on CuboidDef and serde
/// ignores it, so a scene written that way silently loads 0.5m cubes wherever
/// it says something else -- which several older test scenes in this crate do.
fn scene_json() -> String {
    let chunks: Vec<String> = (0..5)
        .map(|i| {
            format!(
                r#"{{
                  "id": "wall_chunk_{i}",
                  "cuboid": {{ "position": [{x}.0, 1.0, 0.0], "half_size": [0.5, 1.0, 0.25] }},
                  "rigid_body": {{ "mode": "Static", "shape": "Box" }},
                  "breakable": {{
                    "health": 100.0,
                    "stages": [{{ "at": 1.0, "removed": true }}]
                  }}
                }}"#,
                i = i,
                x = i
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

/// Fires along +z at the chunk standing at `x`, from 5m short of it.
fn ray_hits(rt: &GameRuntime, x: f32) -> bool {
    rt.rigid_physics
        .raycast(Vec3::new(x, 1.0, -5.0), Vec3::Z, 10.0)
        .is_some()
}

#[test]
fn a_round_destroys_the_chunk_it_hit_and_leaves_its_neighbours() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("one_chunk");

    for x in 0..5 {
        assert!(drawn(&rt, &format!("wall_chunk_{x}")), "wall starts whole");
    }

    // radius 0: a bullet is a point. Only the chunk it is inside takes it.
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
    // The whole point of breaching. A chunk that stopped being drawn and still
    // stops a bullet is worse than one that never broke.
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("hole_open");

    assert!(ray_hits(&rt, 2.0), "the intact wall stops a ray");
    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 100.0);

    assert!(!ray_hits(&rt, 2.0), "and the hole lets one through");
    assert!(ray_hits(&rt, 1.0), "while the wall beside it still stops one");
}

#[test]
fn a_round_that_does_not_finish_a_chunk_changes_nothing_visible() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("chipped");

    let changes = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    assert!(changes.is_empty(), "no threshold crossed, nothing to replicate");
    assert!(drawn(&rt, "wall_chunk_2"), "still standing");
    assert_eq!(rt.damage_taken("wall_chunk_2"), 30.0, "but it remembers");

    // Three more rounds finish it -- accumulation, not a single-hit rule.
    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    let last = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.0, 30.0);
    assert_eq!(last.len(), 1, "the fourth round takes it out");
    assert!(!drawn(&rt, "wall_chunk_2"));
}

#[test]
fn a_blast_opens_a_hole_and_only_chips_the_edges() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("blast");

    // Centred on chunk 2 with a 2m radius. Falloff is what makes this a hole
    // with a ragged edge rather than a rectangle of missing wall.
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
    // Measured to each chunk's SURFACE, so the pair either side of the centre
    // take the same beating -- the blast has no preferred direction.
    assert_eq!(
        rt.damage_taken("wall_chunk_0"),
        rt.damage_taken("wall_chunk_4"),
        "falloff is symmetric about the impact"
    );

    // A second grenade in the same place finishes what the first started.
    let second = rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 2.0, 100.0);
    let widened: Vec<&str> = second.iter().map(|c| c.object_id.as_str()).collect();
    assert_eq!(widened, ["wall_chunk_1", "wall_chunk_3"], "the hole widens");
}

#[test]
fn damage_at_a_point_nothing_stands_at_is_harmless() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("miss");

    assert!(rt.apply_damage_at(Vec3::new(2.0, 1.0, 40.0), 1.0, 500.0).is_empty());
    for x in 0..5 {
        assert!(drawn(&rt, &format!("wall_chunk_{x}")), "a miss breaks nothing");
    }
}

/// One runtime per test on purpose: PhysX allows a single foundation per
/// process, so two live runtimes in one test body abort inside the C++ layer
/// rather than failing an assertion.
#[test]
fn a_tighter_blast_leaves_the_far_chunks_alone() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("blast_tight");

    rt.apply_damage_at(Vec3::new(2.0, 1.0, 0.0), 0.8, 100.0);
    assert!(rt.damage_taken("wall_chunk_1") > 0.0, "the neighbour is caught");
    assert_eq!(rt.damage_taken("wall_chunk_4"), 0.0, "the far end is not");
}

/// An object exactly as the editor's fracture writes it, parsed by the engine.
///
/// Copied out of a running editor rather than written by hand, because the two
/// sides agreeing is the entire claim and a hand-written literal only tests
/// this file's idea of the format. `half_size`, quaternion rotation, and a
/// `removed` stage all have to survive the crossing -- a field serde does not
/// recognise is dropped in SILENCE and loads as a default, which is exactly how
/// a chunk would end up routed against a box that is not the one on screen.
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

    // And the routing finds it: a round inside its box picks it up.
    let hit = crate::damage::impact_targets(
        std::slice::from_ref(&obj),
        obj.cuboid.position,
        0.0,
    );
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].share, 1.0);
}

/* ------------------------------------------------ brushes are not boxes -- */

/// A wall built as a BRUSH rather than a bare cuboid, which is what the editor
/// actually writes -- two solids, so it is visibly not a box.
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
    // It was, and a fractured wall arrived on the headset as a pile of
    // overlapping crates: every chunk drawn at the size of the box around it.
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    // The client built this wall at scene load and cannot know it has since
    // been shot. This list is the whole channel that tells it.
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = brush_runtime("skip_list");

    rt.apply_damage_at(Vec3::new(0.0, 1.0, 0.0), 0.0, 100.0);
    assert_eq!(rt.hidden_brushes(), ["brush_wall"]);
    assert!(!rt.is_solid("brush_wall"), "and it stopped blocking");
}

#[test]
fn a_brush_a_script_hid_is_skipped_too() {
    // Damage is not the only way a brush stops being drawn, and a client that
    // only honoured damage would leave a hidden door standing.
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = brush_runtime("script_hidden");

    rt.script_host.push_command(crate::script::EngineCommand::SetObjectVisible {
        id: "brush_wall".into(),
        visible: false,
    });
    rt.apply_script_commands();

    assert_eq!(rt.hidden_brushes(), ["brush_wall"]);
}
