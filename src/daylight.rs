use std::path::{Path, PathBuf};

use crate::lightmaps::{decode_png_rgba, LightmapEncoding, LightmapEntry, LightmapIndex, LightmapTarget, LIGHTMAP_FORMAT_VERSION};

#[must_use]
pub fn daylight_dir(game_dir: &Path, scene_name: &str) -> PathBuf {
    game_dir.join("scenes").join(format!("{scene_name}.daylight"))
}

#[must_use]
pub fn daylight_probe_dir(game_dir: &Path, scene_name: &str) -> PathBuf {
    crate::reflection_probe::probe_dir(&daylight_dir(game_dir, scene_name), scene_name)
}

pub const BRUSH_LAMPS_ID: &str = "__brushes_lamps__";

pub const BRUSH_SKY_ID: &str = "__brushes_sky__";

pub const BRUSH_SUN_ID: &str = "__brushes_sunlight__";

pub use space_soup_sky::time_of_day::AtlasLayers as BrushLayers;

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

#[must_use]
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
    ((sw, sh) == (w, h) && (uw, uh) == (w, h)).then_some(BrushLayers { width: w, height: h, lamps, sky, sun, sun_keys: Vec::new() })
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
