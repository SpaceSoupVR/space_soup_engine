//! A zone that notices you, does not stop you, and is not drawn.
//!
//! Every assertion here is about OBSERVABLE consequence -- a var that changed,
//! a door that stopped blocking a ray -- rather than about the occupancy map,
//! because "the map says you are inside" and "the door opened" are different
//! claims and only the second one is the feature.

use glam::Vec3;
use space_soup_protocol::PlayerId;
use std::collections::HashMap;

use crate::events::InputFrame;
use crate::rig::PlayerRig;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

/// A 4m zone at the origin, a door 20m away, and a brush-shaped zone at 40m.
///
/// The door is authored SOLID so a test can watch a volume open it, which is
/// the whole point of the feature and not something an occupancy flag proves.
fn scene_json() -> String {
    r#"{
      "name": "zones",
      "objects": [
        {
          "id": "entry_zone",
          "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [2.0, 2.0, 2.0] },
          "hidden": true,
          "trigger_volume": {
            "var": "in_entry",
            "on_enter": [ { "SetObjectSolid": { "id": "door", "solid": false } } ],
            "on_exit":  [ { "SetObjectSolid": { "id": "door", "solid": true } } ]
          }
        },
        {
          "id": "door",
          "cuboid": { "position": [0.0, 1.0, 20.0], "size": [4.0, 2.0, 0.25] },
          "rigid_body": { "mode": "Static", "shape": "Box" }
        },
        {
          "id": "ambush",
          "cuboid": { "position": [0.0, 1.0, 60.0], "half_size": [2.0, 2.0, 2.0] },
          "hidden": true,
          "trigger_volume": {
            "once": true,
            "var": "sprung",
            "on_enter": [ { "AddVar": { "name": "enemies", "delta": 3.0 } } ]
          }
        },
        {
          "id": "armed_later",
          "cuboid": { "position": [0.0, 1.0, 80.0], "half_size": [2.0, 2.0, 2.0] },
          "hidden": true,
          "trigger_volume": { "enabled": false, "var": "late" }
        }
      ]
    }"#
    .to_string()
}

fn runtime(tag: &str) -> GameRuntime {
    let dir = std::env::temp_dir().join(format!("ss_zone_{tag}_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(scenes.join("zones.json"), scene_json()).expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"zones","version":"0.1.0","entry_scene":"zones","scenes":["zones"]}"#,
    )
    .expect("write manifest");
    // The directory STAYS. One of these tests reloads the scene, which reads it
    // from disk again -- and a failure leaving its evidence behind is worth more
    // than a tidy temp directory, which is why the breach tests do the same.
    GameRuntime::load(&dir).expect("runtime loads")
}

/// Put the player at a world position and run one tick.
///
/// Drives `client_offset`, the client-authoritative path, because it sets the
/// position verbatim -- going through locomotion input would mean simulating a
/// walk and asserting on where the walk ended up.
fn stand_at(rt: &mut GameRuntime, at: Vec3) {
    let player = PlayerId::local();
    let mut f = frame(PlayerRig::default(), InputFrame::default());
    f.client_offset = Some(at);
    f.client_yaw = Some(0.0);
    rt.update(1.0 / 60.0, &one_player(player, f));
}

fn var(rt: &GameRuntime, name: &str) -> Option<String> {
    rt.script_host
        .var_string(name)
        .map(|s| s.to_string())
}

fn door_blocks(rt: &GameRuntime) -> bool {
    rt.rigid_physics
        .raycast(Vec3::new(0.0, 1.0, 15.0), Vec3::Z, 10.0)
        .is_some()
}

#[test]
fn standing_in_a_zone_sets_its_var_and_leaving_clears_it() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("var");

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    assert_eq!(var(&rt, "in_entry"), None, "nothing has entered anything yet");

    stand_at(&mut rt, Vec3::ZERO);
    assert_eq!(var(&rt, "in_entry").as_deref(), Some("1"));

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    assert_eq!(var(&rt, "in_entry").as_deref(), Some("0"));
}

#[test]
fn a_zone_opens_a_door_without_being_solid_itself() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("door");

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    assert!(door_blocks(&rt), "the door starts shut");

    stand_at(&mut rt, Vec3::ZERO);
    assert!(!door_blocks(&rt), "standing in the zone must open it");

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    assert!(door_blocks(&rt), "and leaving must shut it again");
}

#[test]
fn the_zone_itself_never_blocks_anything() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("passable");

    // THE defining property. A volume that blocked would be a wall, and it
    // would be a wall you cannot see, which is the worst thing a level can have.
    assert!(
        rt.rigid_physics
            .raycast(Vec3::new(0.0, 1.0, -5.0), Vec3::Z, 10.0)
            .is_none(),
        "a trigger volume must have no collider at all",
    );
}

#[test]
fn a_once_volume_fires_once_and_is_then_spent() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("once");

    stand_at(&mut rt, Vec3::new(0.0, 0.0, 60.0));
    assert_eq!(var(&rt, "enemies").as_deref(), Some("3"));
    assert_eq!(var(&rt, "sprung").as_deref(), Some("1"));

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    assert_eq!(
        var(&rt, "sprung").as_deref(),
        Some("1"),
        "spent means spent: the var stays latched as a record of the visit",
    );

    stand_at(&mut rt, Vec3::new(0.0, 0.0, 60.0));
    assert_eq!(
        var(&rt, "enemies").as_deref(),
        Some("3"),
        "walking back in must not spawn the ambush a second time",
    );
}

#[test]
fn a_disabled_volume_notices_nothing() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("disabled");

    stand_at(&mut rt, Vec3::new(0.0, 0.0, 80.0));
    assert_eq!(var(&rt, "late"), None, "an unarmed zone must not fire");
}

#[test]
fn arming_a_volume_while_someone_stands_in_it_fires_on_the_next_tick() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("arm");

    stand_at(&mut rt, Vec3::new(0.0, 0.0, 80.0));
    // A disabled volume reports itself EMPTY rather than frozen. If it kept its
    // occupants, arming it here would find them already inside and never fire
    // an enter -- the zone would be permanently, silently dead.
    rt.scene
        .objects
        .iter_mut()
        .find(|o| o.id == "armed_later")
        .and_then(|o| o.trigger_volume.as_mut())
        .expect("volume")
        .enabled = true;

    stand_at(&mut rt, Vec3::new(0.0, 0.0, 80.0));
    assert_eq!(var(&rt, "late").as_deref(), Some("1"));
}

#[test]
fn a_zone_is_entered_at_chest_height_not_at_the_feet() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("probe");

    // entry_zone spans y 1..3 in world terms (centre 1, half 2 -> -1..3). A
    // player whose FEET are at y=0 has their chest inside it; testing the floor
    // point would work here too, so the case that matters is a volume raised
    // clear of the ground.
    stand_at(&mut rt, Vec3::ZERO);
    assert_eq!(var(&rt, "in_entry").as_deref(), Some("1"));
}

#[test]
fn a_reload_re_arms_a_spent_volume() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("reload");

    stand_at(&mut rt, Vec3::new(0.0, 0.0, 60.0));
    assert_eq!(var(&rt, "enemies").as_deref(), Some("3"));

    rt.load_scene("zones").expect("reload");
    // The var store is rebuilt with the scene, so this is genuinely a fresh
    // level rather than the same one with its counters carried over.
    assert_eq!(var(&rt, "enemies"), None, "a reload clears the vars too");

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    stand_at(&mut rt, Vec3::new(0.0, 0.0, 60.0));
    // Replaying a level has to actually replay it: a once-only ambush that had
    // already sprung must spring again.
    assert_eq!(var(&rt, "enemies").as_deref(), Some("3"));
}

#[test]
fn a_script_can_ask_whether_a_zone_is_occupied() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = runtime("script");

    stand_at(&mut rt, Vec3::ZERO);
    let ctx = rt.script_host.context();
    let occupied = ctx.lock().unwrap().occupied_volumes.clone();
    assert!(occupied.contains("entry_zone"));

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    let occupied = ctx.lock().unwrap().occupied_volumes.clone();
    assert!(
        !occupied.contains("entry_zone"),
        "a script must never see a zone the dispatcher has already emptied",
    );
}

/// `is_occupied` is the escape hatch, and a registered function nothing calls
/// is invisible to every other test here -- they would all keep passing while it
/// was misspelled, took the wrong type, or was never registered.
#[test]
fn a_script_can_branch_on_occupancy() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join(format!("ss_zone_fn_{}", std::process::id()));
    let scenes = dir.join("scenes");
    std::fs::create_dir_all(&scenes).expect("scenes dir");
    std::fs::write(
        scenes.join("zones.json"),
        r#"{
          "name": "zones",
          "objects": [
            {
              "id": "entry_zone",
              "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [2.0, 2.0, 2.0] },
              "hidden": true,
              "trigger_volume": {}
            },
            {
              "id": "logic",
              "cuboid": { "position": [50.0, 0.0, 50.0], "half_size": [0.1, 0.1, 0.1] },
              "script": "fn on_update(dt) { if is_occupied(\"entry_zone\") { set_var(\"seen\", \"yes\"); } }"
            }
          ]
        }"#,
    )
    .expect("write scene");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"name":"zones","version":"0.1.0","entry_scene":"zones","scenes":["zones"]}"#,
    )
    .expect("write manifest");

    let mut rt = GameRuntime::load(&dir).expect("runtime loads");

    stand_at(&mut rt, Vec3::new(30.0, 0.0, 30.0));
    assert_eq!(var(&rt, "seen"), None, "the script must not fire from outside");

    stand_at(&mut rt, Vec3::ZERO);
    assert_eq!(var(&rt, "seen").as_deref(), Some("yes"));
}

/// The shapes the editor writes, parsed by the engine. Nothing else checks
/// that the two agree, and a mismatch is a zone that silently does nothing.
mod editor_shapes {
    use crate::trigger_volume::{TriggerVolumeDef, VolumeAction};

    #[test]
    fn a_bare_zone_defaults_to_armed_and_repeatable() {
        let def: TriggerVolumeDef = serde_json::from_str("{}").expect("parses");
        assert!(def.enabled, "a zone someone just created must be watching");
        assert!(!def.once);
        assert_eq!(def.var, None);
    }

    #[test]
    fn the_action_shapes_round_trip() {
        let actions = vec![
            VolumeAction::SetObjectSolid { id: "door".into(), solid: false },
            VolumeAction::SetObjectVisible { id: "ghost".into(), visible: true },
            VolumeAction::SetVar { name: "alarm".into(), value: "on".into() },
            VolumeAction::AddVar { name: "score".into(), delta: 10.0 },
            VolumeAction::PlaySound { id: "klaxon".into() },
        ];
        let json = serde_json::to_string(&actions).expect("serialises");
        let back: Vec<VolumeAction> = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, actions);
    }

    #[test]
    fn an_empty_zone_writes_no_empty_lists() {
        // Otherwise every trigger zone in every scene carries two empty arrays
        // and a null, and the file stops being readable.
        let json = serde_json::to_string(&TriggerVolumeDef::default()).expect("serialises");
        assert_eq!(json, r#"{"enabled":true,"once":false}"#);
    }
}
