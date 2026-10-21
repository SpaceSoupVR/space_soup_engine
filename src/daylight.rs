use std::path::{Path, PathBuf};

use crate::lightmaps::{decode_png_rgba, LightmapEncoding, LightmapEntry, LightmapIndex, LightmapTarget, LIGHTMAP_FORMAT_VERSION};

pub fn daylight_dir(game_dir: &Path, scene_name: &str) -> PathBuf {
    game_dir.join("scenes").join(format!("{scene_name}.daylight"))
}

pub fn daylight_probe_dir(game_dir: &Path, scene_name: &str) -> PathBuf {
    crate::reflection_probe::probe_dir(&daylight_dir(game_dir, scene_name), scene_name)
}

pub const BRUSH_LAMPS_ID: &str = "__brushes_lamps__";

pub const BRUSH_SKY_ID: &str = "__brushes_sky__";

pub const BRUSH_SUN_ID: &str = "__brushes_sunlight__";

pub use space_soup_sky::time_of_day::AtlasLayers as BrushLayers;

/// The sun keyframes' list in the daylight directory: which atlas images hold
/// the sun's bounce baked with the sun where, in the order of the day. See
/// `space_soup_sky::time_of_day::SunKey`.
pub const SUN_KEYS_FILE: &str = "sun_keys.json";

/// The id of sun keyframe `i`'s image.
pub fn sun_key_id(i: usize) -> String {
    format!("__brushes_sun_key_{i}__")
}

/// One sun keyframe as `SUN_KEYS_FILE` lists it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SunKeyEntry {
    pub id: String,
    pub hour: f32,
    /// Toward the sun, world.
    pub direction: [f32; 3],
}

/// Lists the sun keyframes `write_brush_layers` wrote; none removes the list.
pub fn write_sun_keys(game_dir: &Path, scene_name: &str, keys: &[SunKeyEntry]) -> std::io::Result<()> {
    let path = daylight_dir(game_dir, scene_name).join(SUN_KEYS_FILE);
    if keys.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::write(path, serde_json::to_string_pretty(keys).map_err(std::io::Error::other)?)
}

pub fn write_brush_layers(game_dir: &Path, scene_name: &str, images: &[(String, u32, u32, Vec<u8>)]) -> std::io::Result<PathBuf> {
    let dir = daylight_dir(game_dir, scene_name);
    std::fs::create_dir_all(&dir)?;
    let mut entries = Vec::new();
    for (object_id, width, height, png) in images {
        let file = format!("{}.png", object_id.trim_matches('_'));
        std::fs::write(dir.join(&file), png)?;
        entries.push(LightmapEntry {
            object_id: object_id.clone(),
            file,
            width: *width,
            height: *height,
            target: LightmapTarget::Brush,
            encoding: LightmapEncoding::F16,
        });
    }
    let index = LightmapIndex { version: LIGHTMAP_FORMAT_VERSION, entries };
    std::fs::write(dir.join("index.json"), serde_json::to_string_pretty(&index).map_err(std::io::Error::other)?)?;
    Ok(dir)
}

pub fn load_brush_layers(game_dir: &Path, scene_name: &str) -> Option<BrushLayers> {
    let dir = daylight_dir(game_dir, scene_name);
    let index: LightmapIndex = serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).ok()?).ok()?;
    if index.version != LIGHTMAP_FORMAT_VERSION {
        return None;
    }
    let read = |id: &str| -> Option<(Vec<f32>, u32, u32)> {
        let e = index.entries.iter().find(|e| e.object_id == id)?;
        let (_, linear, w, h) = decode_png_rgba(&std::fs::read(dir.join(&e.file)).ok()?, e.encoding)?;
        Some((linear?, w, h))
    };
    let (lamps, w, h) = read(BRUSH_LAMPS_ID)?;
    let (sky, sw, sh) = read(BRUSH_SKY_ID)?;
    let (sun, uw, uh) = read(BRUSH_SUN_ID)?;
    if (sw, sh) != (w, h) || (uw, uh) != (w, h) {
        return None;
    }
    // The keyframes, all or none: one missing or of another size leaves the
    // one bake standing for every sun.
    let listed: Vec<SunKeyEntry> =
        std::fs::read_to_string(dir.join(SUN_KEYS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let sun_keys = listed
        .iter()
        .map(|k| match read(&k.id) {
            Some((rgba, kw, kh)) if (kw, kh) == (w, h) => Some(space_soup_sky::time_of_day::SunKey::from_rgba(k.hour, k.direction, &rgba)),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    Some(BrushLayers { width: w, height: h, lamps, sky, sun, sun_keys })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers() -> BrushLayers {
        BrushLayers {
            width: 2,
            height: 1,
            lamps: vec![0.1, 0.1, 0.1, 1.0, 0.0, 0.0, 0.0, 0.5],
            sky: vec![0.2, 0.3, 0.4, 1.0, 0.1, 0.1, 0.1, 0.5],
            sun: vec![1.0, 0.9, 0.8, 1.0, 0.0, 0.0, 0.0, 0.5],
            sun_keys: Vec::new(),
        }
    }

    #[test]
    fn the_layers_add_back_to_the_bake_and_the_night_keeps_only_the_lamps() {
        let l = layers();
        let full = l.combine([1.0; 3], [1.0; 3]);
        assert!((full[0] - 1.3).abs() < 1e-6 && (full[7] - 0.5).abs() < 1e-6);
        assert!(l.matches(&full, 2, 1, 1e-4));
        let night = l.combine([0.0; 3], [0.0; 3]);
        assert_eq!(&night[..4], &l.lamps[..4]);

        let mut other = full.clone();
        other[0] *= 2.0;
        assert!(!l.matches(&other, 2, 1, 0.05));
        assert!(!l.matches(&full, 1, 2, 0.05));
    }

    #[test]
    fn written_layers_read_back() {
        let dir = std::env::temp_dir().join(format!("daylight_layers_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let l = layers();
        let png = |v: &[f32]| {
            let bits: Vec<u16> = v.iter().map(|x| crate::lightmaps::f32_to_f16_bits(*x)).collect();
            let img = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(2, 1, bits).unwrap();
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        write_brush_layers(
            &dir,
            "t",
            &[
                (BRUSH_LAMPS_ID.into(), 2, 1, png(&l.lamps)),
                (BRUSH_SKY_ID.into(), 2, 1, png(&l.sky)),
                (BRUSH_SUN_ID.into(), 2, 1, png(&l.sun)),
            ],
        )
        .unwrap();
        let back = load_brush_layers(&dir, "t").unwrap();
        assert!(back.matches(&l.combine([1.0; 3], [1.0; 3]), 2, 1, 1e-3));
        assert!(back.sun_keys.is_empty());

        // With keyframes, listed in the order of the day.
        let keys = [
            SunKeyEntry { id: sun_key_id(0), hour: 7.0, direction: [0.8, 0.6, 0.0] },
            SunKeyEntry { id: sun_key_id(1), hour: 12.0, direction: [0.0, 1.0, 0.0] },
        ];
        write_brush_layers(
            &dir,
            "t",
            &[
                (BRUSH_LAMPS_ID.into(), 2, 1, png(&l.lamps)),
                (BRUSH_SKY_ID.into(), 2, 1, png(&l.sky)),
                (BRUSH_SUN_ID.into(), 2, 1, png(&l.sun)),
                (sun_key_id(0), 2, 1, png(&l.sun)),
                (sun_key_id(1), 2, 1, png(&l.lamps)),
            ],
        )
        .unwrap();
        write_sun_keys(&dir, "t", &keys).unwrap();
        let back = load_brush_layers(&dir, "t").unwrap();
        assert_eq!(back.sun_keys.len(), 2);
        assert_eq!((back.sun_keys[1].hour, back.sun_keys[1].direction), (12.0, [0.0, 1.0, 0.0]));
        assert!((back.sun_keys[0].rgb[1].to_f32() - 0.9).abs() < 1e-3 && (back.sun_keys[1].rgb[0].to_f32() - 0.1).abs() < 1e-3);
        write_sun_keys(&dir, "t", &[]).unwrap();
        assert!(load_brush_layers(&dir, "t").unwrap().sun_keys.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
