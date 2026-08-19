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

    // Part-scoped grips: a grip on a moving part has to travel with that part.
    // Without this the hand is correct only while the part is at rest, which is
    // exactly when nobody is looking at it.

    fn grip(part: Option<&str>) -> crate::GripPointDef {
        serde_json::from_value(serde_json::json!({
            "name": "charging_handle_grip",
            "hand": "left",
            "local_pos": [0.0, 0.0, 0.0],
            "part": part,
        }))
        .unwrap()
    }

    #[test]
    fn a_part_scoped_grip_rides_its_part_and_a_plain_one_does_not() {
        use glam::{Quat, Vec3};
        let obj_pos = Vec3::new(1.0, 0.0, 0.0);
        // The bolt is 8.9 cm back from the receiver -- an AR-15's full stroke.
        let handle = (Vec3::new(1.0, 0.0, -0.089), Quat::IDENTITY);

        let (scoped, _) = grip(Some("charging_handle")).anchor_world(
            obj_pos,
            Quat::IDENTITY,
            Some(handle),
        );
        assert_eq!(scoped, handle.0, "a part-scoped grip must sit on the part");

        let (plain, _) =
            grip(None).anchor_world(obj_pos, Quat::IDENTITY, Some(handle));
        assert_eq!(
            plain, obj_pos,
            "a grip with no part must ignore the part transform entirely"
        );
    }

    #[test]
    fn a_part_scoped_grip_falls_back_to_the_object_when_the_part_is_unreported() {
        use glam::{Quat, Vec3};
        let obj_pos = Vec3::new(2.0, 1.0, 0.0);
        let (pos, _) = grip(Some("charging_handle")).anchor_world(obj_pos, Quat::IDENTITY, None);
        assert_eq!(
            pos, obj_pos,
            "an unposed mesh reports no parts -- the grip belongs at the pivot, \
             not at the world origin"
        );
    }

    #[test]
    fn the_part_rotation_carries_the_grip_offset() {
        use glam::{Quat, Vec3};
        // A part rotated a quarter turn about Y takes its grip offset with it:
        // +Z local becomes +X world. A grip that ignored part rotation would
        // stay on +Z and sit beside the part rather than on it.
        let g: crate::GripPointDef = serde_json::from_value(serde_json::json!({
            "name": "g", "hand": "left", "local_pos": [0.0, 0.0, 1.0], "part": "bolt",
        }))
        .unwrap();
        let part = (Vec3::ZERO, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let (pos, _) = g.anchor_world(Vec3::ZERO, Quat::IDENTITY, Some(part));
        assert!(
            (pos - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5,
            "expected the offset rotated onto +X, got {pos:?}"
        );
    }

    #[test]
    fn the_hand_pose_uses_hand_offset_and_still_rides_the_part() {
        use glam::{Quat, Vec3};
        // The reach anchor and the hand are authored separately: reach at the
        // handle's tip, hand 3 cm down it. Both must ride the part.
        let g: crate::GripPointDef = serde_json::from_value(serde_json::json!({
            "name": "g", "hand": "left",
            "local_pos": [0.0, 0.0, 0.0],
            "hand_offset_pos": [0.0, -0.03, 0.0],
            "part": "charging_handle",
        }))
        .unwrap();
        let part = (Vec3::new(0.0, 0.0, -0.089), Quat::IDENTITY);
        let (hand, _) = g.hand_world(Vec3::ZERO, Quat::IDENTITY, Some(part));
        assert!(
            (hand - Vec3::new(0.0, -0.03, -0.089)).length() < 1e-6,
            "hand offset must compose on top of the part pose, got {hand:?}"
        );
    }

    // A support grip (the offhand on a rifle's handguard) must not be a way to pick
    // the object up -- every authored grip used to be independently grabbable, so the
    // m4a1 could be carried by its barrel with no hand on the pistol grip.

    #[test]
    fn support_defaults_to_false_and_deserializes_when_set() {
        let plain: crate::GripPointDef =
            serde_json::from_value(serde_json::json!({"name": "main_grip"})).unwrap();
        assert!(
            !plain.support,
            "a grip with no `support` key must stay independently grabbable"
        );

        let support: crate::GripPointDef =
            serde_json::from_value(serde_json::json!({"name": "support_grip", "support": true}))
                .unwrap();
        assert!(support.support);
    }
