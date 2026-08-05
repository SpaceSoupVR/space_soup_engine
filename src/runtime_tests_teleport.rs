use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;

use crate::events::InputFrame;
use crate::rig::PlayerRig;
use crate::runtime::GameRuntime;
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};

    fn empty_scene_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test","scenes":["test"]}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn spawn_point_seeds_a_fresh_players_position_and_yaw() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = empty_scene_dir("space_soup_engine_spawn_point_test");
        std::fs::write(
            dir.join("scenes/test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "start",
                        "cuboid": { "position": [3.0, 0.0, 4.0], "rotation": [0.0, 1.0, 0.0, 0.0] },
                        "spawn_point": {}
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
        rt.update(1.0 / 60.0, &one_player(player, frame(rig, InputFrame::default())));

        let locomotion = &rt.locomotions[&player];
        assert!(
            locomotion.player_offset.distance(Vec3::new(3.0, 0.0, 4.0)) < 1e-4,
            "expected the player to spawn at the spawn_point's position, got {:?}",
            locomotion.player_offset
        );
        let expected_yaw = GameRuntime::yaw_from_forward(Quat::from_xyzw(0.0, 1.0, 0.0, 0.0) * Vec3::NEG_Z);
        assert!(
            (locomotion.player_yaw - expected_yaw).abs() < 1e-4,
            "expected the player's yaw to match the spawn_point's own facing direction"
        );
    }

    #[test]
    fn no_spawn_point_falls_back_to_origin() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = empty_scene_dir("space_soup_engine_no_spawn_point_test");
        std::fs::write(
            dir.join("scenes/test.json"),
            r#"{ "name": "test", "objects": [] }"#,
        )
        .unwrap();

        let mut rt = GameRuntime::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        let player = PlayerId::new();
        let mut rig = PlayerRig::new();
        rig.set_head(Vec3::new(0.0, 1.7, 0.0), Quat::IDENTITY);
        rt.update(1.0 / 60.0, &one_player(player, frame(rig, InputFrame::default())));

        assert_eq!(rt.locomotions[&player].player_offset, Vec3::ZERO);
    }

    #[test]
    fn teleport_disarms_until_the_player_walks_off_the_destination_pad() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = empty_scene_dir("space_soup_engine_teleportal_test");
        std::fs::write(
            dir.join("scenes/test.json"),
            r#"{
                "name": "test",
                "objects": [
                    {
                        "id": "pad_a",
                        "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [1.0, 1.0, 1.0] },
                        "teleportal": { "target_id": "pad_b" }
                    },
                    {
                        "id": "pad_b",
                        "cuboid": { "position": [10.0, 0.0, 0.0], "half_size": [1.0, 1.0, 1.0] },
                        "teleportal": { "target_id": "pad_a" }
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
        let tick = |rt: &mut GameRuntime| {
            rt.update(1.0 / 60.0, &one_player(player, frame(rig.clone(), InputFrame::default())));
        };

        tick(&mut rt);
        assert!(
            rt.locomotions[&player].player_offset.distance(Vec3::new(10.0, 0.0, 0.0)) < 1e-4,
            "expected the player to be teleported onto pad_b, got {:?}",
            rt.locomotions[&player].player_offset
        );

        tick(&mut rt);
        assert!(
            rt.locomotions[&player].player_offset.distance(Vec3::new(10.0, 0.0, 0.0)) < 1e-4,
            "expected pad_b to stay disarmed while the player is still standing on it, got {:?}",
            rt.locomotions[&player].player_offset
        );

        rt.locomotions.get_mut(&player).unwrap().player_offset = Vec3::new(5.0, 0.0, 0.0);
        tick(&mut rt);
        assert!(
            rt.locomotions[&player].player_offset.distance(Vec3::new(5.0, 0.0, 0.0)) < 1e-4,
            "player standing on neither pad should not be teleported"
        );

        rt.locomotions.get_mut(&player).unwrap().player_offset = Vec3::new(10.0, 0.0, 0.0);
        tick(&mut rt);
        assert!(
            rt.locomotions[&player].player_offset.distance(Vec3::new(0.0, 0.0, 0.0)) < 1e-4,
            "expected re-entering pad_b after fully walking off to teleport again, got {:?}",
            rt.locomotions[&player].player_offset
        );
    }

    fn write_two_scene_game(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"test","version":"0.1.0","entry_scene":"test_a","scenes":["test_a","test_b"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("scenes/test_a.json"),
            r#"{
                "name": "test_a",
                "objects": [
                    {
                        "id": "portal",
                        "cuboid": { "position": [0.0, 0.0, 0.0], "half_size": [1.0, 1.0, 1.0] },
                        "teleportal": { "target_scene": "test_b" }
                    }
                ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("scenes/test_b.json"),
            r#"{
                "name": "test_b",
                "objects": [
                    {
                        "id": "arrival",
                        "cuboid": { "position": [5.0, 0.0, 7.0] },
                        "spawn_point": {}
                    }
                ]
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn cross_scene_teleportal_switches_scenes_and_repositions_the_player() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_cross_scene_teleport_test");
        write_two_scene_game(&dir);

        let mut rt = GameRuntime::load(&dir).unwrap();
        assert_eq!(rt.scene_name(), "test_a");

        let player = PlayerId::new();
        let mut rig = PlayerRig::new();
        rig.set_head(Vec3::new(0.0, 1.7, 0.0), Quat::IDENTITY);

        let (_, _, _, _, _, _, scene_change) = rt.update(
            1.0 / 60.0,
            &one_player(player, frame(rig, InputFrame::default())),
        );
        let next_scene = scene_change.expect("expected the portal to request a scene change");
        rt.load_scene(&next_scene).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(rt.scene_name(), "test_b");
        assert!(
            rt.locomotions[&player].player_offset.distance(Vec3::new(5.0, 0.0, 7.0)) < 1e-4,
            "expected the player to land on test_b's own spawn point, got {:?}",
            rt.locomotions[&player].player_offset
        );
    }

    #[test]
    fn load_scene_repositions_already_connected_players_to_the_new_spawn_point() {
        let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("space_soup_engine_load_scene_reposition_test");
        write_two_scene_game(&dir);

        let mut rt = GameRuntime::load(&dir).unwrap();

        let player = PlayerId::new();
        let mut rig = PlayerRig::new();
        rig.set_head(Vec3::new(0.0, 1.7, 0.0), Quat::IDENTITY);
        rt.update(1.0 / 60.0, &one_player(player, frame(rig, InputFrame::default())));

        rt.locomotions.get_mut(&player).unwrap().player_offset = Vec3::new(99.0, 0.0, 99.0);

        rt.load_scene("test_b").unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert!(
            rt.locomotions[&player].player_offset.distance(Vec3::new(5.0, 0.0, 7.0)) < 1e-4,
            "expected load_scene to reposition the already-connected player to the new scene's spawn point, got {:?}",
            rt.locomotions[&player].player_offset
        );
    }
