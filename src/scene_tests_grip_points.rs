    use crate::events::Hand;

    #[test]
    fn authored_grip_points_deserialize_with_their_values() {
        let json = r#"{
            "name": "test",
            "objects": [{
                "id": "m4a1",
                "grip_points": [
                    {
                        "name": "trigger_hand",
                        "kind": "Snap",
                        "hand": "right",
                        "local_pos": [0.0, -0.02, 0.11],
                        "local_rot": [0.0, 0.0, 0.0, 1.0],
                        "hand_offset_scale": [1.0, 1.0, 1.0],
                        "finger_curl": {},
                        "grab_range": 0.12,
                        "hand_offset_pos": [0.0, -0.05, 0.02],
                        "hand_offset_rot": [0.0, 0.0, 0.0, 1.0]
                    },
                    {
                        "name": "foregrip",
                        "kind": "Pinch",
                        "hand": "left",
                        "local_pos": [0.0, -0.03, -0.14],
                        "grab_range": null
                    }
                ]
            }]
        }"#;
        let tmp = std::env::temp_dir().join("grip_points_authoring_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let obj = &scene.objects[0];
        assert_eq!(obj.grip_points.len(), 2);

        let right = obj.grip_point("trigger_hand").expect("right-hand point");
        assert_eq!(right.kind, GripKind::Snap);
        assert_eq!(right.hand, Hand::Right);
        assert_eq!(right.local_pos, [0.0, -0.02, 0.11]);
        assert_eq!(right.grab_range, Some(0.12));
        assert_eq!(right.hand_offset_pos, Some([0.0, -0.05, 0.02])); // decoupled hand transform
        assert_eq!(right.hand_offset_rot, Some([0.0, 0.0, 0.0, 1.0]));

        let left = obj.grip_point("foregrip").expect("left-hand point");
        assert_eq!(left.kind, GripKind::Pinch);
        assert_eq!(left.hand, Hand::Left);
        assert_eq!(left.grab_range, None);
        assert_eq!(left.local_rot, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(left.hand_offset_pos, None); // absent -> hand sits on the anchor

        let out = std::env::temp_dir().join("grip_points_authoring_test_out.json");
        scene.save(&out).unwrap();
        let reloaded = Scene::load(&out).unwrap();
        std::fs::remove_file(&out).ok();
        assert_eq!(reloaded.objects[0].grip_points, obj.grip_points);
    }
