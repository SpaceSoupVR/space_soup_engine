use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const LIGHTMAP_FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightmapEncoding {

    #[default]
    Srgb8,

    F16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LightmapTarget {

    #[default]
    Object,

    Brush,

    Terrain,
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

#[must_use]
pub fn lightmap_dir(game_dir: &Path, scene_name: &str) -> PathBuf {
    game_dir.join("scenes").join(format!("{scene_name}.lightmaps"))
}

pub const SCENE_BRUSH_DIRECTION_ID: &str = "__brushes_dir__";

pub const SCENE_BRUSH_SUN_MASK_ID: &str = "__brushes_sun__";

#[must_use]
pub fn scene_brush_stationary_id(layer: usize) -> String {
    format!("__brushes_stationary_{layer}__")
}

pub const SCENE_TERRAIN_SKY_ID: &str = "__terrain_sky__";

#[must_use]
pub fn scene_terrain_stationary_id(layer: usize) -> String {
    format!("__terrain_stationary_{layer}__")
}

#[must_use]
pub fn mesh_stationary_id(object_id: &str, layer: usize) -> String {
    format!("{object_id}{MESH_STATIONARY_SEPARATOR}{layer}")
}

const MESH_STATIONARY_SEPARATOR: &str = "#stationary_";

#[must_use]
pub fn mesh_stationary_of(id: &str) -> Option<(&str, usize)> {
    let (entity, layer) = id.rsplit_once(MESH_STATIONARY_SEPARATOR)?;
    Some((entity, layer.parse().ok()?))
}

#[cfg(test)]
mod mesh_stationary_id_tests {
    use super::*;

    #[test]
    fn a_mesh_mask_id_names_its_object_and_layer_and_nothing_else_does() {
        let id = mesh_stationary_id("hallway_sconce_2", 1);
        assert_eq!(mesh_stationary_of(&id), Some(("hallway_sconce_2", 1)));
        assert_eq!(mesh_stationary_of("hallway_sconce_2"), None);
        assert_eq!(mesh_stationary_of(&scene_brush_stationary_id(0)), None);
        assert_eq!(mesh_stationary_of(&scene_terrain_stationary_id(0)), None);
        assert_eq!(mesh_stationary_of(SCENE_BRUSH_SUN_MASK_ID), None);
    }
}

pub struct LoadedLightmap {
    pub object_id: String,
    pub width: u32,
    pub height: u32,

    pub rgba: Vec<u8>,
    pub target: LightmapTarget,

    pub linear: Option<Vec<f32>>,
}

#[must_use]
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

#[must_use]
pub fn by_object(maps: Vec<LoadedLightmap>) -> HashMap<String, LoadedLightmap> {
    maps.into_iter().map(|m| (m.object_id.clone(), m)).collect()
}

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

#[must_use]
pub fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

#[must_use]
pub fn linear_to_srgb(l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

#[must_use]
pub fn f16_bits_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let mant = f32::from(h & 0x3ff);
    match exp {
        0 => sign * mant * 2f32.powi(-24),
        31 => if mant == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        e => sign * (1.0 + mant / 1024.0) * 2f32.powi(e - 15),
    }
}

#[must_use]
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

#[must_use]
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

    fn tmp(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ss_lm_{label}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn the_directory_sits_beside_the_scene_json() {

        let dir = lightmap_dir(Path::new("/game"), "lobby");
        assert_eq!(dir, Path::new("/game/scenes/lobby.lightmaps"));
    }

    #[test]
    fn an_unbaked_scene_is_empty_rather_than_an_error() {

        let dir = tmp("an_unbaked_scene_is_empty_rather_than_an_error");
        assert!(load_scene_lightmaps(&dir, "nothing_here").is_empty());
    }

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

        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.join("scenes/level.json"), "{}").unwrap();

        let loaded = load_scene_lightmaps(&dir, "level");
        assert_eq!(loaded.len(), 1, "a stale bake must still load");
        assert_eq!(loaded[0].rgba[0], 9);
    }

    #[test]
    fn baked_sky_visibility_survives_the_png_round_trip() {

        let dir = std::env::temp_dir().join("ss_lm_sky_alpha");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(dir.join("scenes/level.json"), "{}").unwrap();

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
