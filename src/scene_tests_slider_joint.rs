
    #[test]
    fn authored_slider_joint_deserializes_with_its_values() {
        let json = r#"{
            "name": "test",
            "objects": [{
                "id": "sliding_door",
                "slider_joint": {
                    "parent": "door_frame",
                    "axis": [0.0, 1.0, 0.0],
                    "travel": 1.2,
                    "spring_stiffness": 250.0,
                    "spring_damping": 15.0
                }
            }, {
                "id": "defaults_only",
                "slider_joint": { "parent": "rail" }
            }]
        }"#;
        let tmp = std::env::temp_dir().join("slider_joint_authoring_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let door = scene.objects[0].slider_joint.as_ref().expect("slider present");
        assert_eq!(door.parent, "door_frame");
        assert_eq!(door.axis, [0.0, 1.0, 0.0]);
        assert_eq!(door.travel, 1.2);
        assert_eq!(door.spring_stiffness, 250.0);
        assert_eq!(door.spring_damping, 15.0);

        let defs = scene.objects[1].slider_joint.as_ref().expect("slider present");
        assert_eq!(defs.parent, "rail");
        assert_eq!(defs.axis, [1.0, 0.0, 0.0]);
        assert_eq!(defs.travel, 0.02);
        assert_eq!(defs.spring_stiffness, 400.0);
        assert_eq!(defs.spring_damping, 20.0);

        let out = std::env::temp_dir().join("slider_joint_authoring_test_out.json");
        scene.save(&out).unwrap();
        let reloaded = Scene::load(&out).unwrap();
        std::fs::remove_file(&out).ok();
        assert_eq!(reloaded.objects[0].slider_joint, scene.objects[0].slider_joint);
    }
