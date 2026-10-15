use glam::Vec3;

use std::collections::HashMap;

use crate::runtime::GameRuntime;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::script::EngineCommand;

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

fn ray_hits(rt: &GameRuntime, z: f32) -> bool {
    ray_hits_at(rt, 0.0, z)
}

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

    assert!(ray_hits(&rt, 0.0), "the wall is authored solid and comes back solid");
    assert!(!ray_hits(&rt, 10.0), "the gate is authored open and comes back open");
}

#[test]
fn a_scene_written_before_enabled_existed_keeps_its_collision() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rt = runtime("legacy");

    assert!(ray_hits(&rt, 0.0));
}

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

#[test]
fn a_walls_declared_width_is_the_width_that_blocks() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rt = runtime("declared_width");

    assert!(ray_hits_at(&rt, 1.5, 0.0), "1.5m off centre is still wall");
    assert!(!ray_hits_at(&rt, 2.5, 0.0), "and 2.5m off centre is past its end");
}
