use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;
use std::collections::HashMap;

use crate::events::{Hand, InputFrame};
use crate::rig::PlayerRig;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

    #[test]
    fn two_players_grab_different_objects_via_attachments() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_two_player_attach_test");
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
                        "id": "gun_a",
                        "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [0.1, 0.1, 0.1] },
                        "script": "fn on_grab(hand, point) { grab_at_joint(\"gun_a\", hand + \"_grip\", point); } fn on_release(hand) { detach(\"gun_a\", hand); }"
                    },
                    {
                        "id": "gun_b",
                        "cuboid": { "position": [5.0, 1.0, 0.0], "half_size": [0.1, 0.1, 0.1] },
                        "script": "fn on_grab(hand, point) { grab_at_joint(\"gun_b\", hand + \"_grip\", point); } fn on_release(hand) { detach(\"gun_b\", hand); }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;

        let player_a = PlayerId::new();
        let player_b = PlayerId::new();

        let mut rig_a = PlayerRig::new();
        rig_a.set_hand_grip(Hand::Right, Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY);
        let mut grab_a = InputFrame::default();
        grab_a
            .grabbed
            .push(("gun_a".to_string(), Hand::Right, String::new()));

        let mut rig_b = PlayerRig::new();
        rig_b.set_hand_grip(Hand::Right, Vec3::new(5.0, 1.0, 0.0), Quat::IDENTITY);
        let mut grab_b = InputFrame::default();
        grab_b
            .grabbed
            .push(("gun_b".to_string(), Hand::Right, String::new()));

        let mut inputs = HashMap::new();
        inputs.insert(player_a, frame(rig_a, grab_a));
        inputs.insert(player_b, frame(rig_b, grab_b));
        rt.update(dt, &inputs);

        let mut rig_a = PlayerRig::new();
        rig_a.set_hand_grip(Hand::Right, Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY);
        let mut rig_b = PlayerRig::new();
        rig_b.set_hand_grip(Hand::Right, Vec3::new(5.0, 3.0, 0.0), Quat::IDENTITY);

        let mut inputs = HashMap::new();
        inputs.insert(player_a, frame(rig_a, InputFrame::default()));
        inputs.insert(player_b, frame(rig_b, InputFrame::default()));
        rt.update(dt, &inputs);

        let gun_a_pos = rt.scene().find_object("gun_a").unwrap().cuboid.position;
        let gun_b_pos = rt.scene().find_object("gun_b").unwrap().cuboid.position;

        assert!(
            (gun_a_pos.y - 2.0).abs() < 0.05,
            "expected gun_a to follow player A's hand to y\u{2248}2.0, got {gun_a_pos:?}"
        );
        assert!(
            (gun_b_pos.y - 3.0).abs() < 0.05,
            "expected gun_b to follow player B's hand to y\u{2248}3.0, got {gun_b_pos:?}"
        );
        assert!(
            (gun_a_pos.x - 0.0).abs() < 0.05 && (gun_b_pos.x - 5.0).abs() < 0.05,
            "guns should not have swapped/crossed positions: gun_a={gun_a_pos:?} gun_b={gun_b_pos:?}"
        );
    }

    #[test]
    fn two_players_hand_anchors_drive_independently() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_two_player_physx_test");
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
                        "id": "box_a",
                        "cuboid": { "position": [-3.0, 3.0, 0.0], "half_size": [0.2, 0.2, 0.2] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 },
                        "grip_points": [
                            { "name": "handle", "kind": "Snap", "local_pos": [0.0, 0.0, 0.0] }
                        ],
                        "script": "fn on_grab(hand, point) { grab_at_point(\"box_a\", point, hand); } fn on_release(hand) { release_grip(\"box_a\", hand); }"
                    },
                    {
                        "id": "box_b",
                        "cuboid": { "position": [3.0, 3.0, 0.0], "half_size": [0.2, 0.2, 0.2] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 },
                        "grip_points": [
                            { "name": "handle", "kind": "Snap", "local_pos": [0.0, 0.0, 0.0] }
                        ],
                        "script": "fn on_grab(hand, point) { grab_at_point(\"box_b\", point, hand); } fn on_release(hand) { release_grip(\"box_b\", hand); }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;

        let player_a = PlayerId::new();
        let player_b = PlayerId::new();

        let mut rig_a = PlayerRig::new();
        rig_a.set_hand_grip(Hand::Right, Vec3::new(-3.0, 3.0, 0.0), Quat::IDENTITY);
        let mut grab_a = InputFrame::default();
        grab_a
            .grabbed
            .push(("box_a".to_string(), Hand::Right, "handle".to_string()));

        let mut rig_b = PlayerRig::new();
        rig_b.set_hand_grip(Hand::Right, Vec3::new(3.0, 3.0, 0.0), Quat::IDENTITY);
        let mut grab_b = InputFrame::default();
        grab_b
            .grabbed
            .push(("box_b".to_string(), Hand::Right, "handle".to_string()));

        let mut inputs = HashMap::new();
        inputs.insert(player_a, frame(rig_a, grab_a));
        inputs.insert(player_b, frame(rig_b, grab_b));
        rt.update(dt, &inputs);

        for i in 1..=30 {
            let mut rig_a = PlayerRig::new();
            rig_a.set_hand_grip(
                Hand::Right,
                Vec3::new(-3.0, 3.0 - i as f32 * 0.02, 0.0),
                Quat::IDENTITY,
            );
            let mut rig_b = PlayerRig::new();
            rig_b.set_hand_grip(Hand::Right, Vec3::new(3.0, 3.0, 0.0), Quat::IDENTITY);

            let mut inputs = HashMap::new();
            inputs.insert(player_a, frame(rig_a, InputFrame::default()));
            inputs.insert(player_b, frame(rig_b, InputFrame::default()));
            rt.update(dt, &inputs);
        }

        let box_a_y = rt.scene().find_object("box_a").unwrap().cuboid.position.y;
        let box_b_y = rt.scene().find_object("box_b").unwrap().cuboid.position.y;

        assert!(
            (box_a_y - 2.4).abs() < 0.1,
            "expected box_a to follow player A's hand down to y\u{2248}2.4, got {box_a_y}"
        );
        assert!(
            (box_b_y - 3.0).abs() < 0.1,
            "expected box_b to stay near its spawn height since player B's hand didn't move, got {box_b_y}"
        );
    }

    #[test]
    fn disconnected_player_is_cleaned_up_without_corrupting_the_scene() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_disconnect_test");
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
                        "id": "box_a",
                        "cuboid": { "position": [0.0, 3.0, 0.0], "half_size": [0.2, 0.2, 0.2] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 },
                        "grip_points": [
                            { "name": "handle", "kind": "Snap", "local_pos": [0.0, 0.0, 0.0] }
                        ],
                        "script": "fn on_grab(hand, point) { grab_at_point(\"box_a\", point, hand); } fn on_release(hand) { release_grip(\"box_a\", hand); }"
                    },
                    {
                        "id": "box_c",
                        "cuboid": { "position": [10.0, 3.0, 0.0], "half_size": [0.2, 0.2, 0.2] },
                        "rigid_body": { "mode": "Dynamic", "shape": "Box", "mass": 1.0 },
                        "grip_points": [
                            { "name": "handle", "kind": "Snap", "local_pos": [0.0, 0.0, 0.0] }
                        ],
                        "script": "fn on_grab(hand, point) { grab_at_point(\"box_c\", point, hand); } fn on_release(hand) { release_grip(\"box_c\", hand); }"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let dt = 1.0 / 60.0;

        let player_a = PlayerId::new();
        let mut rig_a = PlayerRig::new();
        rig_a.set_hand_grip(Hand::Right, Vec3::new(0.0, 3.0, 0.0), Quat::IDENTITY);
        let mut grab_a = InputFrame::default();
        grab_a
            .grabbed
            .push(("box_a".to_string(), Hand::Right, "handle".to_string()));
        rt.update(dt, &one_player(player_a, frame(rig_a, grab_a)));

        for _ in 0..10 {
            let mut rig_a = PlayerRig::new();
            rig_a.set_hand_grip(Hand::Right, Vec3::new(0.0, 3.0, 0.0), Quat::IDENTITY);
            rt.update(dt, &one_player(player_a, frame(rig_a, InputFrame::default())));
        }
        let held_y = rt.scene().find_object("box_a").unwrap().cuboid.position.y;
        assert!(
            (held_y - 3.0).abs() < 0.2,
            "expected box_a to be held near y=3.0 once settled, got {held_y}"
        );

        for _ in 0..30 {
            rt.update(dt, &HashMap::new());
        }
        let after_drop_y = rt.scene().find_object("box_a").unwrap().cuboid.position.y;
        assert!(
            after_drop_y < held_y - 0.1,
            "expected box_a to fall freely once its holder disconnected and the grab joint was released, went from {held_y} to {after_drop_y}"
        );

        let player_c = PlayerId::new();
        let mut rig_c = PlayerRig::new();
        rig_c.set_hand_grip(Hand::Right, Vec3::new(10.0, 3.0, 0.0), Quat::IDENTITY);
        let mut grab_c = InputFrame::default();
        grab_c
            .grabbed
            .push(("box_c".to_string(), Hand::Right, "handle".to_string()));
        rt.update(dt, &one_player(player_c, frame(rig_c, grab_c)));

        for i in 1..=30 {
            let mut rig_c = PlayerRig::new();
            rig_c.set_hand_grip(
                Hand::Right,
                Vec3::new(10.0, 3.0 - i as f32 * 0.02, 0.0),
                Quat::IDENTITY,
            );
            rt.update(dt, &one_player(player_c, frame(rig_c, InputFrame::default())));
        }
        let box_c_y = rt.scene().find_object("box_c").unwrap().cuboid.position.y;
        assert!(
            (box_c_y - 2.4).abs() < 0.1,
            "expected box_c to follow the new player C's hand down to y\u{2248}2.4, got {box_c_y}"
        );
    }

