    /// Particle emitters and lasers survive a scene round trip.
    ///
    /// This used to load game/scenes/lobby.json and assert that specific props --
    /// smoke_grenade, laser_green, laser_red -- were in it. They were, until a
    /// lobby redesign removed twenty-two objects, and then this failed. It was
    /// never testing the redesign: it is a parser test, and it broke because
    /// someone rearranged a room.
    ///
    /// A test that asserts against a live, hand-edited game asset fails whenever
    /// a designer does their job. This one owns its fixture, so it now fails only
    /// when the thing it names is actually broken.
    #[test]
    fn particle_emitters_and_lasers_round_trip_through_a_scene() {
        let dir = std::env::temp_dir().join(format!("space_soup_pl_scene_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.json");
        std::fs::write(
            &path,
            r#"{
                "name": "particles_and_lasers",
                "objects": [
                    {
                        "id": "smoke_grenade",
                        "cuboid": { "position": [0.0, 1.0, 0.0], "half_size": [0.05, 0.1, 0.05] },
                        "particle_emitter": { "lifetime": 2.5, "speed": 1.5, "color": [200, 200, 200, 180] }
                    },
                    {
                        "id": "laser_green",
                        "cuboid": { "position": [1.0, 1.0, 0.0], "half_size": [0.02, 0.02, 0.02] },
                        "laser": { "color": [0, 255, 0, 255], "max_distance": 25.0, "beam_width": 0.01 }
                    },
                    {
                        "id": "laser_red",
                        "cuboid": { "position": [2.0, 1.0, 0.0], "half_size": [0.02, 0.02, 0.02] },
                        "laser": { "color": [255, 0, 0, 255], "max_distance": 25.0, "beam_width": 0.01 }
                    },
                    {
                        "id": "plain_box",
                        "cuboid": { "position": [3.0, 1.0, 0.0], "half_size": [0.1, 0.1, 0.1] }
                    }
                ]
            }"#,
        )
        .unwrap();

        let scene = Scene::load(&path).expect("scene should parse");
        std::fs::remove_dir_all(&dir).ok();

        let smoke = scene.find_object("smoke_grenade").expect("smoke_grenade exists");
        let emitter = smoke.particle_emitter.as_ref().expect("emitter should survive the load");
        assert!((emitter.lifetime - 2.5).abs() < 1e-6, "emitter fields must round trip, not just be present");

        for (id, expected_green) in [("laser_green", true), ("laser_red", false)] {
            let obj = scene.find_object(id).unwrap_or_else(|| panic!("{id} exists"));
            let laser = obj.laser.as_ref().unwrap_or_else(|| panic!("{id} should have a laser"));
            assert!((laser.max_distance - 25.0).abs() < 1e-6);
            assert!((laser.beam_width - 0.01).abs() < 1e-6);
            let is_green = laser.color.1 > laser.color.0;
            assert_eq!(is_green, expected_green, "{id} kept the wrong colour through the load");
        }

        // An object with neither component must not acquire one by default.
        let plain = scene.find_object("plain_box").expect("plain_box exists");
        assert!(plain.particle_emitter.is_none());
        assert!(plain.laser.is_none());
    }
