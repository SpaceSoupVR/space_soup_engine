use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;
use std::collections::HashMap;

use crate::events::{Hand, InputFrame};
use crate::locomotion::LocomotionInput;
use crate::rig::PlayerRig;
use crate::runtime::{GameRuntime, PlayerFrameInput};
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

    #[test]
    fn falls_lands_and_loops() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_rigid_physics_test");
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
                        "id": "floor",
                        "cuboid": { "position": [0.0, -0.5, 0.0], "half_size": [5.0, 0.5, 5.0] },
                        "rigid_body": { "mode": "Static", "shape": "Box" }
                    },
                    {
                        "id": "ball",
                        "cuboid": { "position": [0.0, 5.0, 0.0], "half_size": [0.5, 0.5, 0.5] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 }
                    },
                    {
                        "id": "looping_ball",
                        "cuboid": { "position": [2.0, 1.5, 0.0], "half_size": [0.5, 0.5, 0.5] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0, "respawn_interval": 1.5 }
                    },
                    {
                        "id": "handle_box",
                        "cuboid": { "position": [-3.0, 3.0, 0.0], "half_size": [0.2, 0.2, 0.2] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 },
                        "grip_points": [
                            { "name": "handle", "kind": "Snap", "local_pos": [0.0, 0.0, 0.0] }
                        ],
                        "script": "fn on_grab(hand, point) { grab_at_point(\"handle_box\", point, hand); } fn on_release(hand) { release_grip(\"handle_box\", hand); }"
                    }
                ]
            }"#,
        ).unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;

        let start_y = rt.scene().find_object("ball").unwrap().cuboid.position.y;
        assert!(
            (start_y - 5.0).abs() < 0.01,
            "expected ball to start at y=5.0, got {start_y}"
        );

        let before_grab_y = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;
        assert!((before_grab_y - 3.0).abs() < 0.05, "expected handle_box to still be at its spawn height before being grabbed, got {before_grab_y}");

        let player = PlayerId::local();

        let mut grab_input = InputFrame::default();
        grab_input
            .grabbed
            .push(("handle_box".to_string(), Hand::Right, "handle".to_string()));
        let mut rig = PlayerRig::new();
        rig.set_hand_grip(Hand::Right, Vec3::new(-3.0, 3.0, 0.0), Quat::IDENTITY);
        rt.update(dt, &one_player(player, frame(rig, grab_input)));

        for i in 1..=30 {
            let mut rig = PlayerRig::new();
            rig.set_hand_grip(
                Hand::Right,
                Vec3::new(-3.0, 3.0 - i as f32 * 0.02, 0.0),
                Quat::IDENTITY,
            );
            rt.update(dt, &one_player(player, frame(rig, InputFrame::default())));
        }
        let held_y = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;
        assert!(
            (held_y - 2.4).abs() < 0.1,
            "expected handle_box to follow the hand down to y\u{2248}2.4 while snap-grabbed (gravity should be overridden by the joint), got {held_y}"
        );

        let mut release_input = InputFrame::default();
        release_input
            .released
            .push(("handle_box".to_string(), Hand::Right));
        rt.update(
            dt,
            &one_player(player, frame(PlayerRig::new(), release_input)),
        );
        let y_at_release = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;

        for _ in 0..30 {
            rt.update(
                dt,
                &one_player(player, frame(PlayerRig::new(), InputFrame::default())),
            );
        }
        let y_after_release = rt
            .scene()
            .find_object("handle_box")
            .unwrap()
            .cuboid
            .position
            .y;
        assert!(
            y_after_release < y_at_release - 0.05,
            "expected handle_box to fall freely under gravity after release, went from {y_at_release} to {y_after_release}"
        );

        rt.update(
            dt,
            &one_player(player, frame(PlayerRig::new(), InputFrame::default())),
        );
        let after_one_step_y = rt.scene().find_object("ball").unwrap().cuboid.position.y;
        assert!(after_one_step_y < start_y, "expected gravity to have pulled the ball down from its start height by now, went from {start_y} to {after_one_step_y}");

        for _ in 0..180 {
            rt.update(
                dt,
                &one_player(player, frame(PlayerRig::new(), InputFrame::default())),
            );
        }
        let landed_y = rt.scene().find_object("ball").unwrap().cuboid.position.y;
        assert!(
            (landed_y - 0.5).abs() < 0.15,
            "expected the ball (half_size.y=0.5) to land resting on the floor's top surface (y=0.0) at y\u{2248}0.5, got {landed_y}"
        );

        let mut saw_high = false;
        let mut saw_low = false;
        for _ in 0..180 {
            rt.update(
                dt,
                &one_player(player, frame(PlayerRig::new(), InputFrame::default())),
            );
            let y = rt
                .scene()
                .find_object("looping_ball")
                .unwrap()
                .cuboid
                .position
                .y;
            if y > 1.2 {
                saw_high = true;
            }
            if y < 0.7 {
                saw_low = true;
            }
        }
        assert!(
            saw_high,
            "expected looping_ball to revisit its spawn height (respawn_interval loop)"
        );
        assert!(
            saw_low,
            "expected looping_ball to also reach the floor (it should still fall each cycle)"
        );
    }

    #[test]
    fn active_sounds_tracked_without_a_listener() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_sound_test");
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
                        "id": "beacon",
                        "cuboid": { "position": [1.0, 2.0, 3.0], "half_size": [0.2, 0.2, 0.2] },
                        "sound": { "clip": "nonexistent.wav", "autoplay": true, "looping": true }
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        rt.update(1.0 / 60.0, &HashMap::new());

        let sounds = rt.active_sounds();
        assert_eq!(
            sounds.len(),
            1,
            "expected the autoplay sound to be tracked as active, got {sounds:?}"
        );
        assert_eq!(sounds[0].object_id, "beacon");
        assert!(
            (sounds[0].position - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-4,
            "expected the reported position to match the object's, got {:?}",
            sounds[0].position
        );
        assert!(sounds[0].looping);
    }

    #[test]
    fn wall_collision_blocks_walking_through_solid_geometry() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_wall_collision_test");
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
                        "cuboid": { "position": [0.0, 1.0, -2.0], "half_size": [3.0, 2.0, 0.2] },
                        "rigid_body": { "mode": "Static", "shape": "Box" }
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        let player = PlayerId::new();
        let mut rig = PlayerRig::new();
        rig.set_head(Vec3::new(0.0, 1.7, 0.0), Quat::IDENTITY);

        let locomotion_input = LocomotionInput {
            move_stick: (0.0, 1.0),
            ..LocomotionInput::default()
        };

        for _ in 0..300 {
            rt.update(
                1.0 / 60.0,
                &one_player(
                    player,
                    PlayerFrameInput {
                        rig: rig.clone(),
                        input: InputFrame::default(),
                        locomotion_input: locomotion_input.clone(),
                        teleport_target: None,
                        client_offset: None,
                        client_yaw: None,
                    },
                ),
            );
        }

        let z = rt.locomotions[&player].player_offset.z;
        assert!(
            z > -2.0,
            "player should have been stopped before reaching the wall at z=-2.0, got z={z}"
        );
        assert!(
            z < -1.0,
            "player should have walked most of the way to the wall before being stopped, got z={z}"
        );
    }

    #[test]
    fn yaw_from_forward_round_trips_with_apply_to_heads_convention() {
        for tenths in -30..=30 {
            let yaw = tenths as f32 / 10.0;
            let fwd = Quat::from_rotation_y(yaw) * Vec3::NEG_Z;
            let recovered = GameRuntime::yaw_from_forward(fwd);
            assert!(
                (recovered - yaw).abs() < 1e-4,
                "yaw {yaw} -> forward {fwd:?} -> recovered {recovered}, expected round trip"
            );
        }
    }

