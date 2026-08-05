
    #[test]
    fn particle_emitters_and_lasers_load_from_lobby_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/scenes/lobby.json");
        let scene = Scene::load(&path).expect("lobby.json should parse");

        let smoke = scene
            .find_object("smoke_grenade")
            .expect("smoke_grenade exists");
        assert!(smoke.particle_emitter.is_some());

        let green = scene.find_object("laser_green").expect("laser_green exists");
        assert!(green.laser.is_some());

        let red = scene.find_object("laser_red").expect("laser_red exists");
        assert!(red.laser.is_some());
    }
