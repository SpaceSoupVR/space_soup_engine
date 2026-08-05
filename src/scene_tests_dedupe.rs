
    #[test]
    fn duplicate_ids_get_renamed_to_stay_unique() {
        let json = r#"{
            "name": "test",
            "objects": [
                {"id": "cloud", "cuboid": {"position": [0.0, 0.0, 0.0]}},
                {"id": "cloud", "cuboid": {"position": [1.0, 0.0, 0.0]}},
                {"id": "cloud", "cuboid": {"position": [2.0, 0.0, 0.0]}}
            ]
        }"#;
        let tmp = std::env::temp_dir().join("dedupe_object_ids_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["cloud", "cloud_2", "cloud_3"]);

        for id in &ids {
            assert!(scene.find_object(id).is_some());
        }
    }

    #[test]
    fn duplicate_of_an_already_numbered_id_skips_past_existing_ones() {
        let json = r#"{
            "name": "test",
            "objects": [
                {"id": "cloud_2", "cuboid": {"position": [0.0, 0.0, 0.0]}},
                {"id": "cloud_2", "cuboid": {"position": [1.0, 0.0, 0.0]}}
            ]
        }"#;
        let tmp = std::env::temp_dir().join("dedupe_object_ids_numbered_test.json");
        std::fs::write(&tmp, json).unwrap();
        let scene = Scene::load(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        let ids: Vec<&str> = scene.objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["cloud_2", "cloud_3"]);
    }
