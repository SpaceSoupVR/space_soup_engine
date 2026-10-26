use std::path::{Path, PathBuf};

use crate::scene::Scene;

fn game_dir() -> PathBuf {
    crate::test_game_dir()
}

fn shipped_scenes() -> Vec<PathBuf> {
    let dir = game_dir().join("scenes");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    out.sort();
    out
}

#[test]
fn every_shipped_scene_loads() {
    let scenes = shipped_scenes();
    assert!(!scenes.is_empty(), "no scenes found under game/scenes");
    for path in scenes {
        if let Err(e) = Scene::load(&path) {
            panic!("{} does not load: {e:#}", path.display());
        }
    }
}

#[test]
fn every_sky_a_scene_names_is_actually_installed() {

    for path in shipped_scenes() {
        let Ok(scene) = Scene::load(&path) else { continue };
        let Some(sky) = scene.sky.as_ref() else { continue };
        assert!(!sky.id.trim().is_empty(), "{} names an empty sky id", path.display());
        let hdr = game_dir().join("skies").join(&sky.id).join("sky.hdr");
        assert!(
            hdr.exists(),
            "{} asks for sky '{}', which is not installed ({} is missing)",
            path.display(),
            sky.id,
            hdr.display(),
        );
    }
}

#[test]
fn a_scene_round_trips_through_serde_unchanged() {

    for path in shipped_scenes() {
        let Ok(scene) = Scene::load(&path) else { continue };
        let text = serde_json::to_string(&scene).expect("a loaded scene should serialise");
        let again: Scene =
            serde_json::from_str(&text).expect("and deserialise back");
        assert_eq!(
            scene.objects.len(),
            again.objects.len(),
            "{} lost objects in a round trip",
            path.display(),
        );
        assert_eq!(
            scene.sky.is_some(),
            again.sky.is_some(),
            "{} lost its sky in a round trip",
            path.display(),
        );
    }
}
