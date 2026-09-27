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
///
/// 2: every lightmap that holds LIGHT is stored as half floats -- see
/// `LightmapEncoding::F16`. A version-1 reader would misread the file.
pub const LIGHTMAP_FORMAT_VERSION: u32 = 2;

/// How a baked image's texels are stored.
///
/// WHY LIGHT IS HALF FLOAT. As 8-bit sRGB, a byte step at the light level of
/// a dim room (0.003) is 10% of the value, and eye adaptation lifts such rooms
/// by up to 16x -- enough to show those steps as contour bands. And 8 bits
/// clipped at 1.0, which bounce beside a lamp exceeds and a Baked lamp's own
/// light exceeds a hundredfold (132 on a wall 30 cm from a bright bulb). No
/// fixed-range integer encoding serves both ends; a half float does, at 0.1%
/// from 6e-5 to 65504. The reflection probes were already stored this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightmapEncoding {
    /// RGBA8, RGB sRGB-encoded. For DATA whose 0..1 range 256 steps already
    /// resolve -- directions, masks, visibility -- and for older bakes.
    #[default]
    Srgb8,
    /// A 16-bit RGBA PNG whose values are the raw bits of IEEE half floats:
    /// RGB linear light in the engine's units, A a 0..1 fraction. Lossless;
    /// a browser cannot display it, which is why the editor is sent a
    /// separate 8-bit preview over the wire.
    F16,
}

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
    /// The terrain's sky-visibility map, over the whole footprint.
    ///
    /// Not lighting at all, unlike the other two: a single greyscale value per
    /// point saying how much of the sky reaches it. Terrain has no second UV
    /// set and no chart layout to hold a lightmap, so it is sampled by the same
    /// normalised footprint coordinate the splat map uses.
    Terrain,
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
    #[serde(default)]
    pub encoding: LightmapEncoding,
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
/// Reserved id for the brush bounce DIRECTION atlas.
///
/// Lives here rather than in the baker because both ends need to agree on it:
/// the baker writes it, and the client has to recognise it to avoid feeding a
/// map of unit vectors into the slot meant for a map of light. It carries
/// `LightmapTarget::Brush` like the atlas it accompanies -- they share the
/// same charts, the same texels and the same uv2 -- so the ID is the only
/// thing that tells them apart.
pub const SCENE_BRUSH_DIRECTION_ID: &str = "__brushes_dir__";

/// Reserved id for the brush SUN-VISIBILITY mask.
///
/// The sky's sun is shaded live on brushes, multiplied by how much of the sun
/// each point can see -- which is this. Red is that visibility (0..255, the
/// fraction of rays across the texel and the sun's disc that reach the sky);
/// green is 255 wherever the baker wrote it, so a runtime can tell a baked
/// shadow from the neutral texel an older bake leaves in its place. At
/// `brush_lightmap::SUN_MASK_SCALE` times the lightmap's density, on the same
/// charts, so it is sampled with the same uv2.
pub const SCENE_BRUSH_SUN_MASK_ID: &str = "__brushes_sun__";

pub struct LoadedLightmap {
    pub object_id: String,
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8. For a half-float map, its light sRGB-encoded and
    /// clipped at 1 -- a preview for anything that only reads 8 bits.
    pub rgba: Vec<u8>,
    pub target: LightmapTarget,
    /// A half-float map at full precision: RGBA f32, RGB linear light in the
    /// engine's units, A a 0..1 fraction. `None` for an 8-bit map.
    pub linear: Option<Vec<f32>>,
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
            Ok(bytes) => match decode_png_rgba(&bytes, entry.encoding) {
                Some((rgba, linear, w, h)) => out.push(LoadedLightmap {
                    object_id: entry.object_id,
                    width: w,
                    height: h,
                    rgba,
                    target: entry.target,
                    linear,
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
    images: &[(String, LightmapTarget, u32, u32, Vec<u8>, LightmapEncoding)],
) -> std::io::Result<PathBuf> {
    let dir = lightmap_dir(game_dir, scene_name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;

    let mut entries = Vec::new();
    for (i, (object_id, target, width, height, png, encoding)) in images.iter().enumerate() {
        let file = format!("{i:03}.png");
        std::fs::write(dir.join(&file), png)?;
        entries.push(LightmapEntry {
            object_id: object_id.clone(),
            file,
            width: *width,
            height: *height,
            target: *target,
            encoding: *encoding,
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

/// sRGB-decode a normalised value.
pub fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// sRGB-encode a linear value in 0..1.
pub fn linear_to_srgb(l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

/// IEEE half-float bits to f32.
pub fn f16_bits_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let mant = (h & 0x3ff) as f32;
    match exp {
        0 => sign * mant * 2f32.powi(-24),
        31 => if mant == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        e => sign * (1.0 + mant / 1024.0) * 2f32.powi(e - 15),
    }
}

/// f32 to IEEE half-float bits, rounding to nearest; past 65504 saturates.
pub fn f32_to_f16_bits(v: f32) -> u16 {
    if v.is_nan() {
        return 0x7e00;
    }
    let sign = if v.is_sign_negative() { 0x8000u16 } else { 0 };
    let a = v.abs().min(65504.0);
    if a < 2f32.powi(-24) * 0.5 {
        return sign;
    }
    if a < 2f32.powi(-14) {
        return sign | (a / 2f32.powi(-24)).round() as u16;
    }
    let e = a.log2().floor() as i32;
    let mut m = ((a / 2f32.powi(e) - 1.0) * 1024.0).round() as u32;
    let mut e = e;
    if m == 1024 {
        m = 0;
        e += 1;
    }
    sign | (((e + 15) as u16) << 10) | m as u16
}

/// A PNG as RGBA8, plus -- for a half-float one -- its full-precision light.
/// Public for the live path, which receives the same files over the wire.
pub fn decode_png_rgba(bytes: &[u8], encoding: LightmapEncoding) -> Option<(Vec<u8>, Option<Vec<f32>>, u32, u32)> {
    let dynamic = image::load_from_memory(bytes).ok()?;
    let (w, h) = (dynamic.width(), dynamic.height());
    match (encoding, &dynamic) {
        (LightmapEncoding::F16, image::DynamicImage::ImageRgba16(img)) => {
            let linear: Vec<f32> = img.as_raw().iter().map(|b| f16_bits_to_f32(*b)).collect();
            let rgba = linear
                .chunks(4)
                .flat_map(|p| {
                    let c = |v: f32| (linear_to_srgb(v) * 255.0).round() as u8;
                    [c(p[0]), c(p[1]), c(p[2]), (p[3].clamp(0.0, 1.0) * 255.0).round() as u8]
                })
                .collect();
            Some((rgba, Some(linear), w, h))
        }
        (LightmapEncoding::F16, _) => None,
        (LightmapEncoding::Srgb8, _) => Some((dynamic.to_rgba8().into_raw(), None, w, h)),
    }
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

    /// HALF-FLOAT LIGHT: a dim value keeps its precision, a value far past
    /// 1.0 is not clipped, and alpha survives -- through the file and back.
    #[test]
    fn half_float_light_reads_back_exactly_across_its_range() {
        let dir = tmp("half_float_light_reads_back_exactly_across_its_range");
        let values = [0.003f32, 0.0031, 132.5, 0.0];
        let img = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(4, 1, |x, _| {
            let b = f32_to_f16_bits(values[x as usize]);
            image::Rgba([b, b, b, f32_to_f16_bits(0.5)])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba16(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        write_scene_lightmaps(&dir, "level", &[("__brushes__".into(), LightmapTarget::Brush, 4, 1, bytes, LightmapEncoding::F16)]).unwrap();
        let m = &by_object(load_scene_lightmaps(&dir, "level"))["__brushes__"];
        let lin = m.linear.as_ref().expect("half-float light decodes to linear light");
        for (x, want) in values.iter().enumerate() {
            let got = lin[x * 4];
            assert!((got - want).abs() <= want * 0.001 + 1e-7, "texel {x}: {got} vs {want}");
        }
        assert!(lin[4] > lin[0], "0.003 and 0.0031 are distinct -- in 8 bits they were one byte");
        assert!((lin[3] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn half_float_bits_round_trip() {
        for v in [0.0f32, 6.1e-5, 0.003, 0.5, 1.0, 132.5, 65504.0] {
            let back = f16_bits_to_f32(f32_to_f16_bits(v));
            assert!((back - v).abs() <= v * 0.001, "{v} -> {back}");
        }
    }

    #[test]
    fn a_written_bake_reads_back_identically() {
        let dir = tmp("a_written_bake_reads_back_identically");
        let images = vec![
            ("wall".to_string(), LightmapTarget::Brush, 4, 4, png(4, 4, 128), LightmapEncoding::Srgb8),
            ("crate".to_string(), LightmapTarget::Object, 2, 2, png(2, 2, 255), LightmapEncoding::Srgb8),
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
            &[(id.clone(), LightmapTarget::Object, 1, 1, png(1, 1, 7), LightmapEncoding::Srgb8)],
        ).unwrap();

        let loaded = by_object(load_scene_lightmaps(&dir, "level"));
        assert!(loaded.contains_key(&id), "got {:?}", loaded.keys().collect::<Vec<_>>());
    }

    #[test]
    fn rewriting_drops_objects_that_are_gone() {
        // A deleted object must not keep shading the level from a leftover file.
        let dir = tmp("rewriting_drops_objects_that_are_gone");
        write_scene_lightmaps(&dir, "level", &[
            ("old".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 9), LightmapEncoding::Srgb8),
        ]).unwrap();
        write_scene_lightmaps(&dir, "level", &[
            ("new".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 9), LightmapEncoding::Srgb8),
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
            ("wall".to_string(), LightmapTarget::Brush, 1, 1, png(1, 1, 200), LightmapEncoding::Srgb8),
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
            ("a".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 10), LightmapEncoding::Srgb8),
            ("b".to_string(), LightmapTarget::Object, 1, 1, png(1, 1, 20), LightmapEncoding::Srgb8),
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
            ("wall".to_string(), LightmapTarget::Brush, 1, 1, png, LightmapEncoding::Srgb8),
        ]).unwrap();

        // Touched after the bake, which is exactly the drift being warned about.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.join("scenes/level.json"), "{}").unwrap();

        let loaded = load_scene_lightmaps(&dir, "level");
        assert_eq!(loaded.len(), 1, "a stale bake must still load");
        assert_eq!(loaded[0].rgba[0], 9);
    }

    #[test]
    fn baked_sky_visibility_survives_the_png_round_trip() {
        // A brush atlas carries sky visibility in ALPHA, and every step between
        // the baker and the shader is a place it can be quietly dropped: a PNG
        // written as RGB, a decode that discards the channel, an upload that
        // takes three bytes per texel. Any of those loses it in silence -- the
        // level still loads and still lights, just with every interior as bright
        // as open ground, which reads as a shading bug rather than a lost byte.
        let dir = std::env::temp_dir().join("ss_lm_sky_alpha");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(dir.join("scenes/level.json"), "{}").unwrap();

        // Deliberately NOT 255: the neutral would pass even if the decoder
        // filled a missing channel with opaque.
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 9, 9, 40]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        write_scene_lightmaps(&dir, "level", &[
            ("wall".to_string(), LightmapTarget::Brush, 1, 1, png, LightmapEncoding::Srgb8),
        ]).unwrap();

        let loaded = load_scene_lightmaps(&dir, "level");
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded[0].rgba.len(),
            4,
            "the upload takes 4 bytes per texel; anything else has dropped a channel",
        );
        assert_eq!(loaded[0].rgba[3], 40, "sky visibility was lost in the round trip");
    }
}
