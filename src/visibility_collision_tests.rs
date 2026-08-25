//! Visible and solid are two switches, and each must move without the other.
//!
//! An invisible wall and a walk-through hologram are ordinary things to want.
//! Before this they were representable only by accident -- `hidden` happened to
//! be render-only and nothing said so -- and neither could be changed once the
//! level was running.
//!
//! Both halves are asserted physically rather than by reading the flag back: a
//! collider is checked with a RAY, because "the flag says false" and "the wall
//! stopped blocking" are different claims and it is the second one that matters.
//! That distinction is exactly what let a breached wall ship visibly broken and
//! still solid.

use glam::Vec3;

use std::collections::HashMap;

use crate::runtime::GameRuntime;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::script::EngineCommand;

/// A solid wall, a wall authored with its collider switched OFF, and a wall
/// with no rigid_body at all -- the three states a level actually contains.
fn scene_json() -> String {
    r#"{
      "name": "vis_test",
      "objects": [
        {
          "id": "wall",
          "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [2.0, 1.0, 0.25] },
          "rigid_body": { "mode": "Static", "shape": "Box" }
        },
        {
          "id": "open_gate",
          "cuboid": { "position": [0.0, 1.0, 10.0], "half_size": [2.0, 1.0, 0.25] },
          "rigid_body": { "mode": "Static", "shape": "Box", "enabled": false }
        },
        {
          "id": "hologram",
          "cuboid": { "position": [0.0, 1.0, 20.0], "half_size": [2.0, 1.0, 0.25] }
        }
      ]
    }"#
    .to_string()
}

fn runtime(tag: &str) -> GameRuntime {
    let dir = std::env::temp_dir().join(format!("ss_vis_{tag}_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(scenes.join("vis_test.json"), scene_json()).expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"vis","version":"0.1.0","entry_scene":"vis_test","scenes":["vis_test"]}"#,
    )
    .expect("write manifest");
    GameRuntime::load(&dir).expect("runtime loads")
}

/// Fires along +z at whatever sits at `z`, from 5m short of it.
fn ray_hits(rt: &GameRuntime, z: f32) -> bool {
    ray_hits_at(rt, 0.0, z)
}

/// The same shot, offset along the wall so its WIDTH decides the answer.
fn ray_hits_at(rt: &GameRuntime, x: f32, z: f32) -> bool {
    rt.rigid_physics
        .raycast(Vec3::new(x, 1.0, z - 5.0), Vec3::Z, 10.0)
        .is_some()
}

fn drawn(rt: &GameRuntime, id: &str) -> bool {
    rt.collect_render_cuboids().iter().any(|c| c.id == id)
}

fn run(rt: &mut GameRuntime, cmd: EngineCommand) {
    rt.script_host.push_command(cmd);
    rt.apply_script_commands();
}

#[test]
fn a_collider_authored_off_is_absent_but_recoverable() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("authored_off");

    assert!(!ray_hits(&rt, 10.0), "an authored-off gate must not block");
    assert!(drawn(&rt, "open_gate"), "and it is still drawn");

    // The whole reason `enabled` is a flag rather than the component's absence:
    // there is still a definition to spawn a collider from.
    run(&mut rt, EngineCommand::SetObjectSolid { id: "open_gate".into(), solid: true });
    assert!(ray_hits(&rt, 10.0), "closing the gate must block the ray");
}

#[test]
fn hiding_an_object_leaves_it_solid() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("hide_solid");

    run(&mut rt, EngineCommand::SetObjectVisible { id: "wall".into(), visible: false });

    assert!(!drawn(&rt, "wall"), "hidden means not drawn");
    assert!(
        ray_hits(&rt, 0.0),
        "and STILL solid -- an invisible wall is the point, not a bug",
    );
}

#[test]
fn making_an_object_passable_leaves_it_visible() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("passable_visible");

    run(&mut rt, EngineCommand::SetObjectSolid { id: "wall".into(), solid: false });

    assert!(!ray_hits(&rt, 0.0), "the collider is gone, not merely flagged");
    assert!(drawn(&rt, "wall"), "and the wall is still on screen");
}

#[test]
fn the_two_switches_do_not_disturb_each_other() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("independent");

    // All four states, reached in sequence from one object.
    run(&mut rt, EngineCommand::SetObjectVisible { id: "wall".into(), visible: false });
    run(&mut rt, EngineCommand::SetObjectSolid { id: "wall".into(), solid: false });
    assert!(!drawn(&rt, "wall") && !ray_hits(&rt, 0.0), "invisible and passable");

    run(&mut rt, EngineCommand::SetObjectVisible { id: "wall".into(), visible: true });
    assert!(drawn(&rt, "wall"), "visible again");
    assert!(!ray_hits(&rt, 0.0), "and turning the picture back on did not restore the collider");

    run(&mut rt, EngineCommand::SetObjectSolid { id: "wall".into(), solid: true });
    assert!(drawn(&rt, "wall") && ray_hits(&rt, 0.0), "visible and solid");
}

#[test]
fn turning_collision_on_twice_does_not_leave_a_second_collider() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("no_duplicate");

    // Driven through spawn_actor DIRECTLY, not through SetObjectSolid.
    //
    // The command refuses a no-op change (`enabled == solid`), so going through
    // it would short-circuit before the guard and this test could never fail --
    // it would assert on the early return and prove nothing about duplicates.
    // The spawn path is reachable from several places (scene load, SpawnObject,
    // this command), and the guard has to hold for all of them.
    //
    // The failure it prevents is silent and unrecoverable: a second actor
    // overwrites the map entry while its predecessor stays in the PhysX scene,
    // so the object is solid twice and only one of them can ever be removed. It
    // shows up as a wall that stays solid after being opened.
    let obj = rt.scene.find_object("wall").expect("wall").clone();
    let def = obj.rigid_body.clone().expect("wall has a rigid_body");
    rt.rigid_physics.spawn_actor(&obj, &def);
    rt.rigid_physics.spawn_actor(&obj, &def);

    run(&mut rt, EngineCommand::SetObjectSolid { id: "wall".into(), solid: false });
    assert!(
        !ray_hits(&rt, 0.0),
        "one 'off' must be enough -- if it is not, a collider was leaked",
    );
}

#[test]
fn an_object_with_no_rigid_body_cannot_be_made_solid() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("no_body");

    // Refused rather than invented. Guessing a collider shape and mass for a
    // decorative object would produce physics nobody authored, and the warning
    // is what tells the author their scene is missing something.
    run(&mut rt, EngineCommand::SetObjectSolid { id: "hologram".into(), solid: true });
    assert!(!ray_hits(&rt, 20.0), "no rigid_body means no collider to switch on");
}

#[test]
fn collision_state_survives_a_scene_reload_as_authored() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("reload");

    run(&mut rt, EngineCommand::SetObjectSolid { id: "wall".into(), solid: false });
    assert!(!ray_hits(&rt, 0.0));

    rt.load_scene("vis_test").expect("reload");
    // Back to what the FILE says, both ways -- the wall solid again and the
    // gate still open. A runtime toggle is match state, not an edit.
    assert!(ray_hits(&rt, 0.0), "the wall is authored solid and comes back solid");
    assert!(!ray_hits(&rt, 10.0), "the gate is authored open and comes back open");
}

#[test]
fn a_scene_written_before_enabled_existed_keeps_its_collision() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rt = runtime("legacy");

    // `wall` names no `enabled` at all, exactly like every scene on disk today.
    // Defaulting it to false would silently unsolidify every level ever made.
    assert!(ray_hits(&rt, 0.0));
}

/// The scene editor writes these actions; the engine reads them. Nothing else
/// checks that the two agree on the shape, and the failure mode is a trigger
/// that silently does nothing in the headset while looking right in the editor.
///
/// These strings are the editor's output verbatim -- externally tagged, `id`
/// omitted for "this object", which is what TriggerEditor.jsx emits when the
/// object field is left empty.
mod editor_shapes {
    use crate::scene_animation::PartTriggerAction;

    fn parse(json: &str) -> PartTriggerAction {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} should parse: {e}"))
    }

    #[test]
    fn an_omitted_id_means_this_object() {
        match parse(r#"{"SetObjectSolid":{"solid":false}}"#) {
            PartTriggerAction::SetObjectSolid { id, solid } => {
                assert_eq!(id, None, "empty in the editor must arrive as None, not \"\"");
                assert!(!solid);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn a_named_object_arrives_named() {
        match parse(r#"{"SetObjectVisible":{"id":"blast_door","visible":true}}"#) {
            PartTriggerAction::SetObjectVisible { id, visible } => {
                assert_eq!(id.as_deref(), Some("blast_door"));
                assert!(visible);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// And back the other way: what the engine writes is what the editor reads.
    /// A `None` id must not serialise as `"id": null`, which the editor's
    /// Select would show as a selected-but-blank object.
    #[test]
    fn a_self_targeted_action_writes_no_id_at_all() {
        let json = serde_json::to_string(&PartTriggerAction::SetObjectSolid {
            id: None,
            solid: true,
        })
        .expect("serialises");
        assert_eq!(json, r#"{"SetObjectSolid":{"solid":true}}"#);
    }
}

/// The script functions are the half a level designer actually reaches for, and
/// a registered function that nothing ever calls is invisible to every other
/// test here -- the commands above would keep passing while `set_solid` was
/// misspelled, took the wrong argument types, or was never registered at all.
#[test]
fn a_script_can_open_and_close_a_gate() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join(format!("ss_vis_script_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(
        scenes.join("vis_test.json"),
        r#"{
          "name": "vis_test",
          "objects": [
            {
              "id": "wall",
              "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [2.0, 1.0, 0.25] },
              "rigid_body": { "mode": "Static", "shape": "Box" }
            },
            {
              "id": "controller",
              "cuboid": { "position": [20.0, 0.0, 20.0], "half_size": [0.1, 0.1, 0.1] },
              "script": "fn on_update(dt) { if get_var(\"open\") == \"yes\" { set_solid(\"wall\", false); set_visible(\"wall\", false); } }"
            }
          ]
        }"#,
    )
    .expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"vis","version":"0.1.0","entry_scene":"vis_test","scenes":["vis_test"]}"#,
    )
    .expect("write manifest");

    let mut rt = GameRuntime::load(&dir).expect("runtime loads");
    std::fs::remove_dir_all(&dir).ok();
    let inputs = HashMap::new();

    rt.update(1.0 / 60.0, &inputs);
    assert!(ray_hits(&rt, 0.0), "nothing has opened the gate yet");
    assert!(drawn(&rt, "wall"));

    rt.script_host.set_var("open", "yes");
    rt.update(1.0 / 60.0, &inputs);

    assert!(!ray_hits(&rt, 0.0), "set_solid must actually remove the collider");
    assert!(!drawn(&rt, "wall"), "and set_visible must actually hide it");
}

/// The wall is as wide as the scene says it is.
///
/// Every other ray in this file is fired at x = 0, straight down the wall's
/// centre line -- where a 4m wall and a 0.5m cube are the same object. That is
/// not a hypothetical: these scenes DID declare their walls with a key serde
/// ignores, so every wall here was a 0.5m cube, and the whole suite stayed
/// green through it. Shrinking a wall a hundredfold still passed everything
/// else.
///
/// Shots either side of that cube's edge are what makes the declared extent
/// load-bearing, so the same mistake cannot be silent twice.
#[test]
fn a_walls_declared_width_is_the_width_that_blocks() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rt = runtime("declared_width");

    // Half-width is 2m. Well inside it, and well outside a 0.5m cube.
    assert!(ray_hits_at(&rt, 1.5, 0.0), "1.5m off centre is still wall");
    assert!(!ray_hits_at(&rt, 2.5, 0.0), "and 2.5m off centre is past its end");
}
