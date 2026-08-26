//! The scenes that actually ship, loaded.
//!
//! Nothing loaded them. A scene file is edited by the web editor, by hand, and
//! by whatever writes a backup, and the first thing that discovers a broken one
//! is a headset failing to start a level. Every failure mode here is a parse
//! error or a dangling reference, both of which are free to check.
//!
//! Deliberately NOT asserting on contents. A test that knew the lobby had a
//! particular prop lived here once and was removed for exactly that reason:
//! it failed whenever anyone edited the level, which is what a level is for.

use std::path::{Path, PathBuf};

use crate::scene::Scene;

fn game_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../game")
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
    // A mistyped sky id is the quiet kind of broken: the scene loads, the level
    // runs, and the sky is simply absent with nothing said. The panorama lives
    // in a shared library outside the scene file, so this is the only place the
    // reference can be checked at all.
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
    // Load, write, load again. Catches a field that deserialises but does not
    // serialise back -- which turns the editor's next save into silent data
    // loss for whatever the field was.
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
