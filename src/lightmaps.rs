//! Baked lighting, on disk, so a level can ship without the editor behind it.
//!
//! WHY THIS EXISTS
//!
//! Baking was built as a live-preview feature: `tools/bake` printed its results
//! to stdout, the editor server held them in memory, and both the browser and
//! the headset picked them up over a WebSocket. Nothing was ever written to
//! disk. That works beautifully while the editor is running and produces a game
//! with no baked lighting at all, because a shipped Quest build has no server to
//! connect to -- it falls back to the white 1x1 texture and every light shines
//! through every wall.
//!
//! Nobody would notice from the code: the fallback is graceful, the renderer is
//! correct, and in the editor it looks right. It is only wrong where there is no
//! editor, which is the one place it has to be right.
//!
//! So a bake now writes a directory beside the scene, and the runtime reads it
//! at load. The WebSocket stays exactly as it was and overrides what was loaded,
//! which is what makes lighting edits still appear live while authoring.
//!
//! THE FORMAT
//!
//! ```text
//! <scene>.lightmaps/
//!     index.json      what is in here, and which file is whose
//!     000.png         one per object, numbered
//! ```
//!
//! Files are NUMBERED rather than named after their object. Object ids are
//! human names -- "work_light_stand/lamp (left)" is a legal id -- and every
//! scheme for turning those into filenames is a source of collisions, escaping
//! bugs and platform differences. The index carries the real mapping, so the
//! names on disk only have to be unique.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when the meaning of a baked image changes, so a stale bake is ignored
/// rather than silently shading a level the way an older baker thought it should.
pub const LIGHTMAP_FORMAT_VERSION: u32 = 1;

/// Which surfaces a baked image is for.
///
/// Separate because they are unwrapped differently and sampled by different
/// pipelines: a mesh carries its own second UV set, while a brush's is
/// generated from its faces. Mixing them up produces lighting that is subtly
/// wrong rather than obviously missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightmapTarget {
    /// Cuboids and imported meshes: the per-object atlas the original baker made.
    Object,
    /// Level brushes: rooms, walls, floors.
    Brush,
}

impl Default for LightmapTarget {
    fn default() -> Self {
        Self::Object
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightmapEntry {
    pub object_id: String,
    pub file: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub target: LightmapTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightmapIndex {
    pub version: u32,
    pub entries: Vec<LightmapEntry>,
}

/// The directory a scene's baked lighting lives in.
///
/// Takes the GAME directory and joins `scenes/` itself, which is the same
/// contract as `terrain::load_splat` and the brush and terrain loaders in
/// quest_app. Getting this wrong is invisible: the loader finds nothing, returns
/// an empty set exactly as it does for an unbaked level, and the game runs with
/// no baked lighting and no error. It is worth one function that cannot be
/// called with the wrong half of the path.
pub fn lightmap_dir(game_dir: &Path, scene_name: &str) -> PathBuf {
    game_dir.join("scenes").join(format!("{scene_name}.lightmaps"))
}

/// One object's baked image, as loaded.
pub struct LoadedLightmap {
    pub object_id: String,
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8.
    pub rgba: Vec<u8>,
    pub target: LightmapTarget,
}

/// Everything baked for a scene, or an empty set when there is nothing there.
///
/// Deliberately NOT an error when the directory is missing. A level that has
/// never been baked is a normal state -- it is what every level is before the
/// first bake -- and the renderer already draws it correctly with the white
/// default. Returning `Err` here would make "unbaked" indistinguishable from
/// "corrupt" at every call site, and the tempting fix for that is to ignore the
/// error, which also ignores the corrupt case.
pub fn load_scene_lightmaps(game_dir: &Path, scene_name: &str) -> Vec<LoadedLightmap> {
    let dir = lightmap_dir(game_dir, scene_name);
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else {
        return Vec::new();
    };
    let Ok(index) = serde_json::from_str::<LightmapIndex>(&raw) else {
        log_warn(&format!("lightmaps: {} is not readable, ignoring", dir.display()));
        return Vec::new();
    };
    if index.version != LIGHTMAP_FORMAT_VERSION {
        log_warn(&format!(
            "lightmaps: {} is version {} and this build reads {}; ignoring, so the level is \
             lit by its dynamic lights rather than by a bake that means something else",
            dir.display(),
            index.version,
            LIGHTMAP_FORMAT_VERSION,
        ));
        return Vec::new();
    }

    warn_if_stale(game_dir, scene_name, &dir);

    let mut out = Vec::new();
    for entry in index.entries {
        match std::fs::read(dir.join(&entry.file)) {
            Ok(bytes) => match decode_png_rgba(&bytes) {
                Some((rgba, w, h)) => out.push(LoadedLightmap {
                    object_id: entry.object_id,
                    width: w,
                    height: h,
                    rgba,
                    target: entry.target,
                }),
                None => log_warn(&format!("lightmaps: {} did not decode", entry.file)),
            },
            Err(e) => log_warn(&format!("lightmaps: {} could not be read: {e}", entry.file)),
        }
    }
    out
}

/// Write a set of baked images, replacing whatever was there.
///
/// The directory is rebuilt rather than merged: an object deleted from the scene
/// must not keep shading the level from a file nobody looks at any more.
pub fn write_scene_lightmaps(
    game_dir: &Path,
    scene_name: &str,
    images: &[(String, LightmapTarget, u32, u32, Vec<u8>)],
) -> std::io::Result<PathBuf> {
    let dir = lightmap_dir(game_dir, scene_name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;

    let mut entries = Vec::new();
    for (i, (object_id, target, width, height, png)) in images.iter().enumerate() {
        let file = format!("{i:03}.png");
        std::fs::write(dir.join(&file), png)?;
        entries.push(LightmapEntry {
            object_id: object_id.clone(),
            file,
            width: *width,
            height: *height,
            target: *target,
        });
    }

    let index = LightmapIndex { version: LIGHTMAP_FORMAT_VERSION, entries };
    std::fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&index).map_err(std::io::Error::other)?,
    )?;
    Ok(dir)
}

/// Index the loaded set by object id, which is how the renderer asks for one.
pub fn by_object(maps: Vec<LoadedLightmap>) -> HashMap<String, LoadedLightmap> {
    maps.into_iter().map(|m| (m.object_id.clone(), m)).collect()
}

/// Say so when the scene has been edited since it was baked.
///
/// The bake and the scene it came from have to ship together, and nothing
/// enforces that: a level saved without a rebake, or committed without its
/// lightmaps, loads perfectly and is lit by the wrong thing. On a headset there
/// is no editor to notice with, so the only chance of catching it is the log at
/// load -- which is where anybody debugging "the lighting looks wrong on device"
/// is already looking.
///
/// A warning rather than a refusal. A slightly stale bake still looks far better
/// than none, and a level that would not load because its lighting was out of
/// date would be a much worse failure than the one this is about.
fn warn_if_stale(game_dir: &Path, scene_name: &str, lightmap_dir: &Path) {
    let scene = game_dir.join("scenes").join(format!("{scene_name}.json"));
    let (Ok(scene_meta), Ok(index_meta)) = (
        std::fs::metadata(&scene),
        std::fs::metadata(lightmap_dir.join("index.json")),
    ) else {
        return;
    };
    let (Ok(scene_time), Ok(index_time)) = (scene_meta.modified(), index_meta.modified()) else {
        return;
    };
    if scene_time > index_time {
        log_warn(&format!(
            "lightmaps: {} was edited after it was baked -- the lighting you are seeing is \
             from an older version of this level. Re-save it in the editor, or run \
             `bake lightmap <scene> --write <game-dir>`",
            scene.display(),
        ));
    }
}

fn decode_png_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some((img.into_raw(), w, h))
}

fn log_warn(msg: &str) {
    // A bake that cannot be read is worth saying out loud exactly once. Silence
    // here is how the WebSocket-only era went unnoticed for as long as it did.
    log::warn!("{msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, v: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([v, v, v, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A directory of this test's own.
    ///
    /// Named after the caller rather than a timestamp: these run in parallel and
    /// a nanosecond clock is not unique enough, which showed up as one test
    /// deleting another's directory mid-write.
    fn tmp(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ss_lm_{label}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn the_directory_sits_beside_the_scene_json() {
        // quest_app's brush and terrain loaders both resolve
        // `game_dir/scenes/<name>.json`, and this has to land next to it or the
        // runtime looks somewhere nothing was ever written -- which reads as an
        // unbaked level rather than as a mistake, and so is never reported.
        let dir = lightmap_dir(Path::new("/game"), "lobby");
        assert_eq!(dir, Path::new("/game/scenes/lobby.lightmaps"));
    }

    #[test]
    fn an_unbaked_scene_is_empty_rather_than_an_error() {
        // Every level is unbaked before its first bake. If this were an error,
        // every call site would have to ignore it -- and would then also be
        // ignoring the corrupt case.
        let dir = tmp("an_unbaked_scene_is_empty_rather_than_an_error");
        assert!(load_scene_lightmaps(&dir, "nothing_here").is_empty());
    }

    #[test]
    fn a_written_bake_reads_back_identically() {
        let dir = tmp("a_written_bake_reads_back_identically");
        let images = vec![
            ("wall".to_string(), LightmapTarget::Brush, 4, 4, png(4, 4, 128)),
            ("crate".to_string(), LightmapTarget::Object, 2, 2, png(2, 2, 255)),
        ];
        write_scene_lightmaps(&dir, "level", &images).unwrap();

        let loaded = by_object(load_scene_lightmaps(&dir, "level"));
        assert_eq!(loaded.len(), 2);
        let wall = &loaded["wall"];
        assert_eq!((wall.width, wall.height), (4, 4));
        assert_eq!(wall.target, LightmapTarget::Brush);
        assert_eq!(wall.rgba[0], 128);
        assert_eq!(loaded["crate"].target, LightmapTarget::Object);
    }

    #[test]
    fn an_id_that_is_not_a_filename_survives_the_round_trip() {
        // The reason files are numbered. This id is legal in a scene and is not
        // a legal filename on every platform.
        let dir = tmp("an_id_that_is_not_a_filename_survives_the_round_trip");
        let id = "work_light_stand/lamp (left) #2".to_string();
        write_scene_lightmaps(
            &dir, "level",
            &[(id.clone(), LightmapTarget::Object, 1, 1, png(1, 1, 7))],
        ).unwrap();

        let loaded = by_object(load_scene_lightmaps(&dir, "level"));
        assert!(loaded.contains_key(&id), "got {:?}", loaded.keys().collect::<Vec<_>>());
    }

    #[test]
    fn rewriting_drops_objects_that_are_gone() {
        // A deleted object must not keep shading the level from a leftover file.
        let dir = tmp("rewriting_drops_objects_that_are_gone");
        write_scene_lightmaps(&dir, "level", &[
            ("old".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 9)),
        ]).unwrap();
        write_scene_lightmaps(&dir, "level", &[
            ("new".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 9)),
        ]).unwrap();

        let loaded = by_object(load_scene_lightmaps(&dir, "level"));
        assert!(!loaded.contains_key("old"));
        assert!(loaded.contains_key("new"));
    }

    #[test]
    fn a_bake_from_a_different_format_version_is_ignored() {
        // Being lit by the dynamic lights alone is a known, correct-looking
        // state. Being lit by a bake that meant something else is not.
        let dir = tmp("a_bake_from_a_different_format_version_is_ignored");
        write_scene_lightmaps(&dir, "level", &[
            ("wall".to_string(), LightmapTarget::Brush, 1, 1, png(1, 1, 200)),
        ]).unwrap();

        let index_path = lightmap_dir(&dir, "level").join("index.json");
        let mut index: LightmapIndex =
            serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
        index.version = LIGHTMAP_FORMAT_VERSION + 1;
        std::fs::write(&index_path, serde_json::to_string(&index).unwrap()).unwrap();

        assert!(load_scene_lightmaps(&dir, "level").is_empty());
    }

    #[test]
    fn a_corrupt_index_does_not_take_the_level_down() {
        let dir = tmp("a_corrupt_index_does_not_take_the_level_down");
        let lm = lightmap_dir(&dir, "level");
        std::fs::create_dir_all(&lm).unwrap();
        std::fs::write(lm.join("index.json"), "{ not json").unwrap();
        assert!(load_scene_lightmaps(&dir, "level").is_empty());
    }

    #[test]
    fn a_missing_image_skips_only_that_object() {
        // One unreadable file must not cost the whole level its lighting.
        let dir = tmp("a_missing_image_skips_only_that_object");
        write_scene_lightmaps(&dir, "level", &[
            ("a".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 10)),
            ("b".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 20)),
        ]).unwrap();
        std::fs::remove_file(lightmap_dir(&dir, "level").join("000.png")).unwrap();

        let loaded = by_object(load_scene_lightmaps(&dir, "level"));
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("b"));
    }
}

#[cfg(test)]
mod staleness_tests {
    use super::*;

    #[test]
    fn a_scene_edited_after_its_bake_still_loads() {
        // A warning, never a refusal: a slightly stale bake looks far better
        // than none, and a level that would not load because its lighting was
        // out of date is a worse failure than the one being guarded against.
        let dir = std::env::temp_dir().join("ss_lm_stale");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();

        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 9, 9, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        write_scene_lightmaps(&dir, "level", &[
            ("wall".to_string(), LightmapTarget::Brush, 1, 1, png),
        ]).unwrap();

        // Touched after the bake, which is exactly the drift being warned about.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.join("scenes/level.json"), "{}").unwrap();

        let loaded = load_scene_lightmaps(&dir, "level");
        assert_eq!(loaded.len(), 1, "a stale bake must still load");
        assert_eq!(loaded[0].rgba[0], 9);
    }
}
