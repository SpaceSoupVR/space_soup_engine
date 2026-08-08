use glam::Vec3;
use space_soup_protocol::PlayerId;
use std::collections::HashMap;

use crate::events::InputFrame;
use crate::rig::PlayerRig;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

    #[test]
    fn scripted_raycast_hits_the_correct_object_and_reports_its_transform() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_raycast_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "wall",
                        "cuboid": { "position": [0.0, 0.0, -3.0], "half_size": [1.0, 1.0, 0.2] }
                    },
                    {
                        "id": "rotated_prop",
                        "cuboid": {
                            "position": [5.0, 0.0, 5.0],
                            "half_size": [0.3, 0.3, 0.3],
                            "rotation": [0.0, 0.3826834, 0.0, 0.9238795]
                        }
                    },
                    {
                        "id": "marker",
                        "cuboid": { "position": [10.0, 10.0, 10.0], "half_size": [0.1, 0.1, 0.1] }
                    },
                    {
                        "id": "shooter",
                        "cuboid": { "position": [10.0, 10.0, 10.0], "half_size": [0.1, 0.1, 0.1] },
                        "script": "fn on_update(dt) { move_object(\"marker\", get_object_rot_x(\"rotated_prop\"), get_object_rot_y(\"rotated_prop\"), get_object_rot_z(\"rotated_prop\")); if raycast(0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 10.0) && raycast_hit_object() == \"wall\" { set_var(\"hit\", raycast_hit_z()); } if raycast(0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 10.0) { set_var(\"miss_should_not_run\", true); } }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let inputs = HashMap::new();

        rt.update(dt, &inputs);

        let marker = rt.scene.find_object("marker").unwrap();
        assert!(
            (marker.cuboid.position.y - 0.3826834).abs() < 1e-4,
            "get_object_rot_y should read back rotated_prop's authored rotation, got {:?}",
            marker.cuboid.position
        );

        let vars = rt.script_host.context().lock().unwrap().vars.clone();
        assert!(
            vars.contains_key("hit"),
            "a ray straight ahead into the wall should hit it and identify it by id"
        );
        let hit_z = vars.get("hit").unwrap().as_float().unwrap();
        assert!(
            (hit_z - (-2.8)).abs() < 0.05,
            "hit point should land on the wall's near face around z=-2.8, got {hit_z}"
        );
        assert!(
            !vars.contains_key("miss_should_not_run"),
            "a ray that points away from every object should not report a hit"
        );
    }

    #[test]
    fn scripted_part_animation_blend_reaches_the_render_mesh() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_part_blend_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [0.2, 0.1, 0.5] },
                        "mesh": { "path": "models/gun.glb" },
                        "script": "fn on_update(dt) { play_part_animation(\"gun\", \"bolt_cycle\", 0.75); }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let inputs = HashMap::new();

        rt.update(dt, &inputs);

        let (_, meshes, _) = rt.render_lists();
        let gun = meshes.iter().find(|m| m.id == "gun").expect("gun mesh should render");
        assert_eq!(
            gun.manual_part_blends.get("bolt_cycle").copied(),
            Some(0.75),
            "play_part_animation should surface the blend on the render mesh for clients to apply"
        );
    }

    #[test]
    fn scripted_particle_burst_fires_once_ages_then_disappears() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_particle_burst_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [1.0, 2.0, 3.0], "half_size": [0.2, 0.1, 0.5] },
                        "particle_emitter": { "lifetime": 0.2, "speed": 3.0, "color": [255, 200, 100, 255] },
                        "script": "fn on_press(button) { if button == \"trigger\" { spawn_particle_burst(\"gun\", 8); } }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();

        let mut input = InputFrame::default();
        input.button_presses.push(crate::events::ButtonPress {
            button: "trigger".to_string(),
            object_id: Some("gun".to_string()),
            ..Default::default()
        });
        rt.update(dt, &one_player(player, frame(rig.clone(), input)));

        let (_, _, _, _, bursts, _, _) = rt.update(dt, &one_player(player, frame(rig.clone(), InputFrame::default())));
        assert_eq!(bursts.len(), 1, "spawn_particle_burst should produce exactly one active burst");
        let burst = &bursts[0];
        assert_eq!(burst.count, 8);
        assert_eq!(burst.position, Vec3::new(1.0, 2.0, 3.0));
        assert!(burst.elapsed > 0.0 && burst.elapsed < burst.lifetime, "burst should be mid-flight");

        let frames_to_expire = (burst.lifetime / dt).ceil() as usize + 2;
        let mut last_bursts = bursts;
        for _ in 0..frames_to_expire {
            let (_, _, _, _, bursts, _, _) = rt.update(dt, &one_player(player, frame(rig.clone(), InputFrame::default())));
            last_bursts = bursts;
        }
        assert!(
            last_bursts.is_empty(),
            "burst should have aged out and been dropped after its lifetime elapsed"
        );
    }

    #[test]
    fn scripted_spawn_and_impulse_launches_a_fresh_dynamic_object() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_spawn_impulse_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "casing_template",
                        "hidden": true,
                        "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [0.02, 0.02, 0.05] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 0.01 }
                    },
                    {
                        "id": "gun",
                        "cuboid": { "position": [1.0, 2.0, 3.0], "half_size": [0.2, 0.1, 0.5] },
                        "script": "fn on_press(button) { if button == \"trigger\" { spawn_object(\"casing_template\", \"casing1\", 1.0, 2.0, 3.0); apply_impulse(\"casing1\", 0.0, 2.0, 0.0); } }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();

        assert!(rt.scene().find_object("casing1").is_none(), "casing1 should not exist before firing");

        let mut input = InputFrame::default();
        input.button_presses.push(crate::events::ButtonPress {
            button: "trigger".to_string(),
            object_id: Some("gun".to_string()),
            ..Default::default()
        });
        rt.update(dt, &one_player(player, frame(rig.clone(), input)));

        let casing = rt.scene().find_object("casing1").expect("spawn_object should create a new live object");
        assert!(!casing.hidden, "spawned copy should not inherit the template's hidden flag");
        // spawn_object and apply_impulse both land in the same frame's command batch, which is
        // applied before that frame's physics step, so the position has already moved a bit off
        // the exact spawn point by the time this first update() call returns.
        assert!(
            casing.cuboid.position.distance(Vec3::new(1.0, 2.0, 3.0)) < 0.1,
            "spawned copy should appear near the requested spawn point, got {:?}",
            casing.cuboid.position
        );

        rt.update(dt, &one_player(player, frame(rig.clone(), InputFrame::default())));
        let y_after_impulse = rt.scene().find_object("casing1").unwrap().cuboid.position.y;
        assert!(
            y_after_impulse > 2.0,
            "apply_impulse should have launched the casing upward before gravity pulls it back down, got y={y_after_impulse}"
        );
    }

    #[test]
    fn socket_attach_carries_the_child_and_detach_hands_it_back_to_physics() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_socket_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [1.0, 5.0, 2.0], "half_size": [0.2, 0.1, 0.5] },
                        "sockets": [
                            { "name": "mag_well", "local_pos": [0.0, -0.1, 0.0] }
                        ],
                        "script": "fn on_press(button) { if button == \"attach\" { attach_to_socket(\"mag\", \"gun\", \"mag_well\"); } if button == \"detach\" { detach_from_socket(\"mag\"); } }"
                    },
                    {
                        "id": "mag",
                        "cuboid": { "position": [8.0, 8.0, 8.0], "half_size": [0.05, 0.15, 0.05] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 0.2 }
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();

        let press = |button: &str| {
            let mut input = InputFrame::default();
            input.button_presses.push(crate::events::ButtonPress {
                button: button.to_string(),
                object_id: Some("gun".to_string()),
                ..Default::default()
            });
            input
        };

        rt.update(dt, &one_player(player, frame(rig.clone(), press("attach"))));
        rt.update(dt, &one_player(player, frame(rig.clone(), InputFrame::default())));

        let gun_pos = rt.scene().find_object("gun").unwrap().cuboid.position;
        let mag_pos = rt.scene().find_object("mag").unwrap().cuboid.position;
        assert!(
            mag_pos.distance(gun_pos + Vec3::new(0.0, -0.1, 0.0)) < 0.01,
            "attached mag should sit at the gun's mag_well socket offset, gun={gun_pos:?} mag={mag_pos:?}"
        );

        rt.update(dt, &one_player(player, frame(rig.clone(), press("detach"))));
        let mag_y_at_detach = rt.scene().find_object("mag").unwrap().cuboid.position.y;

        for _ in 0..30 {
            rt.update(dt, &one_player(player, frame(rig.clone(), InputFrame::default())));
        }
        let mag_y_after_falling = rt.scene().find_object("mag").unwrap().cuboid.position.y;
        assert!(
            mag_y_after_falling < mag_y_at_detach - 0.05,
            "detached mag should fall freely under gravity again, went from {mag_y_at_detach} to {mag_y_after_falling}"
        );
    }

    #[test]
    fn rapid_fire_play_sound_layers_instead_of_cutting_off() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_rapid_fire_sound_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();

        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [1.0, 2.0, 3.0], "half_size": [0.2, 0.1, 0.5] },
                        "sound": { "clip": "shot.wav", "looping": false },
                        "script": "fn on_press(button) { if button == \"trigger\" { play_sound(\"gun\"); } }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();

        let press_trigger = || {
            let mut input = InputFrame::default();
            input.button_presses.push(crate::events::ButtonPress {
                button: "trigger".to_string(),
                object_id: Some("gun".to_string()),
                ..Default::default()
            });
            input
        };

        rt.update(dt, &one_player(player, frame(rig.clone(), press_trigger())));
        rt.update(dt, &one_player(player, frame(rig.clone(), press_trigger())));
        rt.update(dt, &one_player(player, frame(rig.clone(), press_trigger())));

        let sounds = rt.active_sounds();
        assert_eq!(
            sounds.len(),
            3,
            "three rapid trigger pulls should produce three independent voices instead of the later shots cutting off the earlier ones, got {sounds:?}"
        );
        let unique_ids: std::collections::HashSet<&str> =
            sounds.iter().map(|s| s.object_id.as_str()).collect();
        assert_eq!(unique_ids.len(), 3, "each voice should have a distinct id");
        assert!(sounds.iter().all(|s| s.clip == "shot.wav"));
    }


    // ── Button edges and polled axes ─────────────────────────────────────────

    #[test]
    fn button_up_and_hand_reach_scripts_and_axes_can_be_polled() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_button_edges_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();

        // Records what it saw into vars. `on_press` is kept alongside the new hooks
        // to prove the old one-arg form still fires for existing scripts.
        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [0.2, 0.1, 0.5] },
                        "script": "fn on_press(b) { set_var(\"legacy\", b); } fn on_button_down(b, hand) { set_var(\"down\", b + \":\" + hand); } fn on_button_up(b, hand) { set_var(\"up\", b + \":\" + hand); } fn on_update(dt) { set_var(\"trig\", get_trigger(\"right\")); set_var(\"stick\", get_stick_x(\"left\")); }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();
        let var = |rt: &GameRuntime, k: &str| rt.script_host.context().lock().unwrap().vars.get(k).cloned();

        let mut down = InputFrame::default();
        down.button_presses.push(crate::events::ButtonPress::new(
            "trigger",
            Some("gun".to_string()),
            crate::events::Hand::Left,
        ));
        down.axes.r_trigger = 0.75;
        down.axes.l_stick = [-0.5, 0.25];
        rt.update(dt, &one_player(player, frame(rig.clone(), down)));

        assert_eq!(
            var(&rt, "down").unwrap().into_string().unwrap(),
            "trigger:left",
            "on_button_down must carry both the button and the hand that pressed it"
        );
        assert_eq!(
            var(&rt, "legacy").unwrap().into_string().unwrap(),
            "trigger",
            "the one-arg on_press must keep working so existing scripts do not break"
        );
        assert!(var(&rt, "up").is_none(), "nothing was released yet");

        // Polled, not evented: this is what lets a script know the trigger is STILL
        // held, which an edge cannot say.
        assert!((var(&rt, "trig").unwrap().as_float().unwrap() - 0.75).abs() < 1e-6);
        assert!((var(&rt, "stick").unwrap().as_float().unwrap() - -0.5).abs() < 1e-6);

        let mut up = InputFrame::default();
        up.button_releases.push(crate::events::ButtonPress::new(
            "trigger",
            Some("gun".to_string()),
            crate::events::Hand::Left,
        ));
        rt.update(dt, &one_player(player, frame(rig.clone(), up)));
        assert_eq!(
            var(&rt, "up").unwrap().into_string().unwrap(),
            "trigger:left",
            "a button-up edge must reach scripts -- on_release means GRAB release"
        );
    }

    #[test]
    fn set_part_visible_toggles_hidden_parts_and_defaults_to_visible() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_part_visible_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();
        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [0.2, 0.1, 0.5] },
                        "script": "fn on_button_down(b, hand) { if b == \"grip\" { set_part_visible(\"gun\", \"mag_full\", false); set_part_visible(\"gun\", \"mag_empty\", true); } else { set_part_visible(\"gun\", \"mag_full\", true); } }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();
        let hidden = |rt: &GameRuntime| rt.scene().find_object("gun").unwrap().hidden_parts.clone();

        // Nothing is hidden until something says so -- "visible" is the default for
        // every part of every model that never mentions this.
        assert!(hidden(&rt).is_empty());

        let press = |button: &str| {
            let mut i = InputFrame::default();
            i.button_presses.push(crate::events::ButtonPress::new(
                button,
                Some("gun".to_string()),
                crate::events::Hand::Right,
            ));
            i
        };

        rt.update(dt, &one_player(player, frame(rig.clone(), press("grip"))));
        assert_eq!(hidden(&rt), vec!["mag_full".to_string()],
            "hiding one part must not hide the one that was made visible in the same batch");

        // Idempotent: holding the button must not stack duplicates.
        rt.update(dt, &one_player(player, frame(rig.clone(), press("grip"))));
        assert_eq!(hidden(&rt), vec!["mag_full".to_string()]);

        rt.update(dt, &one_player(player, frame(rig.clone(), press("trigger"))));
        assert!(hidden(&rt).is_empty(), "making a part visible removes it from hidden_parts");
    }

    #[test]
    fn a_blend_trigger_fires_once_per_crossing_and_detaches_to_physics() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_blend_trigger_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();
        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [1.0, 2.0, 3.0], "half_size": [0.2, 0.1, 0.5] },
                        "part_animations": [
                            { "clip": "mag_eject", "driver": "HoldGrip",
                              "triggers": [ { "at": 0.85,
                                              "action": { "DetachPart": { "part": "mag_full",
                                                                          "template": "mag_template",
                                                                          "impulse": [0.0, -1.0, 0.0] } } } ] }
                        ]
                    },
                    {
                        "id": "mag_template",
                        "hidden": true,
                        "cuboid": { "position": [0.0, -50.0, 0.0], "half_size": [0.05, 0.12, 0.02] },
                        "rigid_body": { "mode": "Dynamic", "mass": 0.3 }
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();

        let at_blend = |b: f32| {
            let mut i = InputFrame::default();
            let mut clips = std::collections::HashMap::new();
            clips.insert("mag_eject".to_string(), b);
            i.part_blends.insert("gun".to_string(), clips);
            i
        };
        let detached = |rt: &GameRuntime| {
            rt.scene().objects.iter().filter(|o| o.id.contains("#mag_full#")).count()
        };

        // Below the threshold nothing happens, however long it is held there.
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.5))));
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.8))));
        assert_eq!(detached(rt_ref(&rt)), 0, "a blend short of the threshold must not fire");

        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.9))));
        assert_eq!(detached(rt_ref(&rt)), 1, "crossing the threshold should detach exactly once");
        assert!(
            rt.scene().find_object("gun").unwrap().hidden_parts.contains(&"mag_full".to_string()),
            "the source part must stop being drawn, or the magazine appears twice"
        );

        // Held past the line, and jittering across it, must not fire again --
        // a hand near a threshold crosses it many times a second.
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.95))));
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.84))));
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.9))));
        assert_eq!(detached(rt_ref(&rt)), 1, "jitter around the threshold must not re-fire");

        // Fully released and pulled again is a new, deliberate motion.
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.0))));
        rt.update(dt, &one_player(player, frame(rig.clone(), at_blend(0.9))));
        assert_eq!(detached(rt_ref(&rt)), 2, "a fresh pull should detach again");
    }

    fn rt_ref(rt: &GameRuntime) -> &GameRuntime {
        rt
    }

    #[test]
    fn a_part_anchored_socket_and_detach_use_the_posed_part_position() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_part_socket_test");
        let scenes_dir = dir.join("scenes");
        std::fs::create_dir_all(&scenes_dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();
        std::fs::write(
            scenes_dir.join("test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "gun",
                        "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [0.2, 0.1, 0.5] },
                        "sockets": [ { "name": "ejection_port", "part": "bolt", "local_pos": [0.0, 0.1, 0.0] } ],
                        "part_animations": [
                            { "clip": "eject", "driver": "HoldGrip",
                              "triggers": [ { "at": 0.5,
                                              "action": { "DetachPart": { "part": "bolt",
                                                                          "template": "casing" } } } ] }
                        ]
                    },
                    { "id": "casing", "hidden": true,
                      "cuboid": { "position": [0.0, -50.0, 0.0], "half_size": [0.01, 0.02, 0.01] },
                      "rigid_body": { "mode": "Dynamic", "mass": 0.01 } }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;
        let player = PlayerId::new();
        let rig = PlayerRig::default();

        // The bolt is posed 3 units away from the gun's own origin.
        let mut input = InputFrame::default();
        let mut parts = std::collections::HashMap::new();
        parts.insert("bolt".to_string(), ([3.0f32, 0.0, 0.0], [0.0f32, 0.0, 0.0, 1.0]));
        input.part_transforms.insert("gun".to_string(), parts);
        let mut clips = std::collections::HashMap::new();
        clips.insert("eject".to_string(), 0.9);
        input.part_blends.insert("gun".to_string(), clips);

        rt.update(dt, &one_player(player, frame(rig.clone(), input)));

        let spawned = rt
            .scene()
            .objects
            .iter()
            .find(|o| o.id.contains("#bolt#"))
            .expect("crossing the threshold should detach");
        assert!(
            (spawned.cuboid.position.x - 3.0).abs() < 0.2,
            "the detached part must appear where the PART is, not at the object pivot -- got {:?}",
            spawned.cuboid.position
        );
    }
