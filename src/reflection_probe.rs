use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const CUBE_FACES: usize = 6;

pub const DEFAULT_PROBE_RESOLUTION: u32 = 256;

pub const MAX_PROBE_RESOLUTION: u32 = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionProbeDef {

    #[serde(default = "default_probe_resolution")]
    pub resolution: u32,
}

impl Default for ReflectionProbeDef {
    fn default() -> Self {
        Self { resolution: DEFAULT_PROBE_RESOLUTION }
    }
}

fn default_probe_resolution() -> u32 {
    DEFAULT_PROBE_RESOLUTION
}

impl ReflectionProbeDef {
    #[must_use]
    pub fn clamped_resolution(&self) -> u32 {
        self.resolution.clamp(4, MAX_PROBE_RESOLUTION)
    }
}

#[must_use]
pub fn box_project(
    reflect_dir: Vec3,
    frag: Vec3,
    probe_centre: Vec3,
    box_min: Vec3,
    box_max: Vec3,
) -> Vec3 {
    let dir = reflect_dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return Vec3::Y;
    }

    let inv = Vec3::new(
        if dir.x.abs() > 1e-6 { 1.0 / dir.x } else { f32::MAX },
        if dir.y.abs() > 1e-6 { 1.0 / dir.y } else { f32::MAX },
        if dir.z.abs() > 1e-6 { 1.0 / dir.z } else { f32::MAX },
    );

    let frag = frag.clamp(box_min, box_max);
    let t_max = (box_max - frag) * inv;
    let t_min = (box_min - frag) * inv;

    let furthest = t_max.max(t_min);
    let dist = furthest.x.min(furthest.y).min(furthest.z);

    let to_hit = frag + dir * dist.max(0.0) - probe_centre;
    if !dist.is_finite() || to_hit.length_squared() <= 1e-12 {
        return dir;
    }
    to_hit.normalize()
}

pub fn probe_for_point<T>(point: Vec3, probes: &[(T, Vec3, Vec3)]) -> Option<&T> {
    probes
        .iter()
        .filter(|(_, min, max)| {
            point.x >= min.x && point.x <= max.x
                && point.y >= min.y && point.y <= max.y
                && point.z >= min.z && point.z <= max.z
        })
        .min_by(|a, b| {
            let vol = |min: Vec3, max: Vec3| {
                let d = max - min;
                d.x * d.y * d.z
            };
            vol(a.1, a.2).partial_cmp(&vol(b.1, b.2)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(t, _, _)| t)
}

#[must_use]
pub fn face_direction(face: usize, u: f32, v: f32) -> Vec3 {

    let a = 2.0 * u - 1.0;
    let b = 1.0 - 2.0 * v;
    let d = match face {
        0 => Vec3::new(1.0, b, -a),
        1 => Vec3::new(-1.0, b, a),
        2 => Vec3::new(a, 1.0, -b),
        3 => Vec3::new(a, -1.0, b),
        4 => Vec3::new(a, b, 1.0),
        _ => Vec3::new(-a, b, -1.0),
    };
    d.normalize()
}

#[must_use]
pub fn probe_dir(game_dir: &std::path::Path, scene_name: &str) -> std::path::PathBuf {
    game_dir.join("scenes").join(format!("{scene_name}.probes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> (Vec3, Vec3, Vec3) {
        let min = Vec3::new(-5.0, 0.0, -5.0);
        let max = Vec3::new(5.0, 4.0, 5.0);
        (min, max, (min + max) * 0.5)
    }

    #[test]
    fn two_points_looking_the_same_way_sample_different_directions() {
        let (min, max, centre) = room();
        let up = Vec3::Y;
        let a = box_project(up, Vec3::new(-4.0, 1.0, 0.0), centre, min, max);
        let b = box_project(up, Vec3::new(4.0, 1.0, 0.0), centre, min, max);
        assert!(
            a.distance(b) > 0.5,
            "two points 8m apart looking straight up sampled {a:?} and {b:?}; the \
             projection is ignoring position and the reflection will slide",
        );
    }

    #[test]
    fn looking_up_samples_the_ceiling_above_you() {
        let (min, max, centre) = room();
        let left = box_project(Vec3::Y, Vec3::new(-4.0, 1.0, 0.0), centre, min, max);
        assert!(left.y > 0.0, "looking up did not sample upward: {left:?}");
        assert!(
            left.x < 0.0,
            "standing on the left of the room, the ceiling overhead is to the \
             left of the probe; sampled {left:?}",
        );
    }

    #[test]
    fn at_the_probe_itself_the_direction_is_unchanged() {
        let (min, max, centre) = room();
        for dir in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, -3.0).normalize()] {
            let got = box_project(dir, centre, centre, min, max);
            assert!(
                got.distance(dir.normalize()) < 1e-4,
                "at the probe centre, {dir:?} became {got:?}",
            );
        }
    }

    #[test]
    fn outside_the_box_it_is_projected_from_the_surface() {
        let (min, max, centre) = room();
        let outside = box_project(Vec3::Y, Vec3::new(3.0, 100.0, 0.0), centre, min, max);
        let on = box_project(Vec3::Y, Vec3::new(3.0, max.y, 0.0), centre, min, max);
        assert!(outside.distance(on) < 1e-5, "{outside:?} vs {on:?}");
    }

    #[test]
    fn a_point_on_the_wall_reflects_what_its_neighbours_just_inside_do() {
        let (min, max, centre) = room();

        let dir = Vec3::new(1.0, -0.3, 0.2).normalize();
        let on = box_project(dir, Vec3::new(max.x, max.y, 1.0), centre, min, max);
        let inside = box_project(dir, Vec3::new(max.x - 1e-3, max.y, 1.0), centre, min, max);
        let past = box_project(dir, Vec3::new(max.x + 0.02, max.y, 1.0), centre, min, max);
        assert!(on.distance(inside) < 1e-2, "on the wall {on:?}, just inside {inside:?}");
        assert!(past.distance(inside) < 1e-2, "extrapolated past it {past:?}, just inside {inside:?}");
    }

    #[test]
    fn each_face_centre_looks_along_its_own_axis() {
        let expect = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        for (face, want) in expect.iter().enumerate() {
            let got = face_direction(face, 0.5, 0.5);
            assert!(
                got.distance(*want) < 1e-5,
                "face {face} centre looks {got:?}, expected {want:?}",
            );
        }
    }

    #[test]
    fn the_faces_tile_the_whole_sphere() {
        let mut seen = [0usize; CUBE_FACES];
        for face in 0..CUBE_FACES {
            for i in 0..8 {
                for j in 0..8 {
                    let u = (i as f32 + 0.5) / 8.0;
                    let v = (j as f32 + 0.5) / 8.0;
                    let d = face_direction(face, u, v);

                    let (ax, mag) = [d.x, d.y, d.z]
                        .iter()
                        .enumerate()
                        .map(|(k, c)| (k, c.abs()))
                        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                        .unwrap();
                    let _ = mag;
                    let sign_positive = [d.x, d.y, d.z][ax] > 0.0;
                    let owner = ax * 2 + usize::from(!sign_positive);
                    assert_eq!(
                        owner, face,
                        "face {face} texel ({u}, {v}) points {d:?}, which belongs to \
                         face {owner} -- the faces do not tile",
                    );
                    seen[face] += 1;
                }
            }
        }
        assert!(seen.iter().all(|n| *n == 64));
    }

    #[test]
    fn the_tightest_probe_serving_a_point_is_chosen() {
        let probes = vec![
            ("hall", Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0)),
            ("alcove", Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.5, 2.0)),
        ];

        assert_eq!(probe_for_point(Vec3::new(1.0, 1.0, 1.0), &probes), Some(&"alcove"));

        assert_eq!(probe_for_point(Vec3::new(-8.0, 1.0, -8.0), &probes), Some(&"hall"));

        assert_eq!(probe_for_point(Vec3::new(50.0, 1.0, 0.0), &probes), None);
    }

    #[test]
    fn each_face_is_the_right_way_round() {

        let cases: [(usize, f32, f32, Vec3); 12] = [
            (0, 0.0, 0.5, Vec3::Z),
            (0, 1.0, 0.5, Vec3::NEG_Z),
            (1, 0.0, 0.5, Vec3::NEG_Z),
            (1, 1.0, 0.5, Vec3::Z),
            (2, 0.5, 0.0, Vec3::NEG_Z),
            (2, 0.5, 1.0, Vec3::Z),
            (3, 0.5, 0.0, Vec3::Z),
            (3, 0.5, 1.0, Vec3::NEG_Z),
            (4, 0.0, 0.5, Vec3::NEG_X),
            (4, 1.0, 0.5, Vec3::X),
            (5, 0.0, 0.5, Vec3::X),
            (5, 1.0, 0.5, Vec3::NEG_X),
        ];
        for (face, u, v, want) in cases {
            let d = face_direction(face, u, v);
            assert!(
                d.dot(want) > 0.5,
                "face {face} at ({u}, {v}) points {d:?}, which does not lean toward \
                 {want:?} -- that face is flipped",
            );
        }
    }

    #[test]
    fn every_side_face_has_up_at_the_top() {
        for face in [0usize, 1, 4, 5] {
            let top = face_direction(face, 0.5, 0.0);
            let bottom = face_direction(face, 0.5, 1.0);
            assert!(
                top.y > bottom.y,
                "face {face} has v=0 at {top:?} and v=1 at {bottom:?}; it is upside down",
            );
        }
    }

}

pub const PROBE_ENCODING: &str = "sqrt16";

pub struct LoadedProbe {
    pub object_id: String,

    pub volume: String,

    pub resolution: u32,

    pub min: [f32; 3],
    pub max: [f32; 3],

    pub centre: [f32; 3],

    pub cell: Option<[u32; 3]>,
    pub cells: Option<[u32; 3]>,

    pub faces: Vec<u8>,
}

fn probe_faces_f16(img: &image::ImageBuffer<image::Rgba<u16>, Vec<u16>>, range: f32) -> Vec<u8> {
    let raw = img.as_raw();
    let mut out = Vec::with_capacity(raw.len() * 2);
    for (i, &v) in raw.iter().enumerate() {
        let unit = f32::from(v) / f32::from(u16::MAX);

        let linear = if i % 4 == 3 { unit } else { unit * unit * range };
        out.extend_from_slice(&f32_to_f16(linear).to_le_bytes());
    }
    out
}

fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    let mant = bits & 0x007f_ffff;
    if exp > 15 {

        return sign | if exp == 128 && mant != 0 { 0x7e00 } else { 0x7c00 };
    }
    if exp < -14 {

        let shift = (-14 - exp) as u32;
        if shift > 24 {
            return sign;
        }
        return sign | ((mant | 0x0080_0000) >> (shift + 13)) as u16;
    }
    sign | (((exp + 15) as u16) << 10) | ((mant >> 13) as u16)
}

#[derive(Clone, Debug)]
pub struct ProbeEntry {
    pub object_id: String,

    pub volume: String,
    pub path: std::path::PathBuf,

    pub resolution: u32,
    pub range: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub centre: [f32; 3],
    pub cell: Option<[u32; 3]>,
    pub cells: Option<[u32; 3]>,

    pub depth: Option<(std::path::PathBuf, f32)>,

    pub irradiance: Option<[[f32; 3]; 9]>,
}

#[must_use]
pub fn parse_probe_irradiance(v: &serde_json::Value) -> Option<[[f32; 3]; 9]> {
    let a = v.as_array()?;
    if a.len() != 27 {
        return None;
    }
    let mut out = [[0.0f32; 3]; 9];
    for (i, x) in a.iter().enumerate() {
        out[i / 3][i % 3] = x.as_f64()? as f32;
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPortal {
    pub min: [f32; 3],
    pub max: [f32; 3],

    pub axis: u32,

    pub low: String,
    pub high: String,
}

#[must_use]
pub fn load_scene_portals(game_dir: &std::path::Path, scene_name: &str) -> Vec<LoadedPortal> {
    let dir = probe_dir(game_dir, scene_name);
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else { return Vec::new() };
    let Ok(index) = serde_json::from_str::<serde_json::Value>(&raw) else { return Vec::new() };
    let Some(entries) = index.get("portals").and_then(|p| p.as_array()) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|e| {
            let v3 = |key: &str| -> Option<[f32; 3]> {
                let a = e.get(key)?.as_array()?;
                Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
            };
            Some(LoadedPortal {
                min: v3("min")?,
                max: v3("max")?,
                axis: e.get("axis")?.as_u64()?.min(2) as u32,
                low: e.get("low")?.as_str()?.to_string(),
                high: e.get("high")?.as_str()?.to_string(),
            })
        })
        .collect()
}

#[must_use]
pub fn decode_probe(entry: &ProbeEntry) -> Option<(u32, Vec<u8>)> {
    let img = image::open(&entry.path).ok()?;
    let rgba = img.to_rgba16();
    let res = rgba.width();

    if res == 0 || rgba.height() != res * CUBE_FACES as u32 {
        return None;
    }
    Some((res, probe_faces_f16(&rgba, entry.range)))
}

pub const PROBE_DEPTH_ENCODING: &str = "plane16";

#[must_use]
pub fn decode_probe_depth(entry: &ProbeEntry, res: u32) -> Option<Vec<u16>> {
    let (path, far) = entry.depth.as_ref()?;
    let img = image::open(path).ok()?.to_rgba16();
    if img.width() != res || img.height() != res * CUBE_FACES as u32 {
        return None;
    }
    let h16 = crate::lightmaps::f32_to_f16_bits;
    let mut out = Vec::with_capacity((res * res * CUBE_FACES as u32 * 4) as usize);
    for p in img.pixels() {
        let [r, g, b, a] = p.0;
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, h16(1.0)]);
            continue;
        }
        let unit = |v: u16| f32::from(v) / 65535.0 * 2.0 - 1.0;
        let n = [unit(r), unit(g), unit(b)];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        let c = f32::from(a - 1) / 65534.0;
        let h = -(c * c) * far;
        out.extend_from_slice(&[h16(n[0] / len), h16(n[1] / len), h16(n[2] / len), h16(h)]);
    }
    Some(out)
}

#[must_use]
pub fn load_scene_probes(game_dir: &std::path::Path, scene_name: &str) -> Vec<LoadedProbe> {
    load_scene_probe_index(game_dir, scene_name)
        .into_iter()
        .filter_map(|e| {
            let (res, faces) = decode_probe(&e)?;
            Some(LoadedProbe {
                object_id: e.object_id,
                volume: e.volume,
                resolution: res,
                min: e.min,
                max: e.max,
                centre: e.centre,
                cell: e.cell,
                cells: e.cells,
                faces,
            })
        })
        .collect()
}

#[must_use]
pub fn load_scene_buildings(game_dir: &std::path::Path, scene_name: &str) -> Vec<ProbeEntry> {
    let dir = probe_dir(game_dir, scene_name);
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else { return Vec::new() };
    let Ok(index) = serde_json::from_str::<serde_json::Value>(&raw) else { return Vec::new() };
    let Some(entries) = index.get("buildings").and_then(|p| p.as_array()) else { return Vec::new() };
    let vec3 = |e: &serde_json::Value, key: &str| -> Option<[f32; 3]> {
        let a = e.get(key)?.as_array()?;
        (a.len() == 3).then_some(())?;
        Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
    };
    entries
        .iter()
        .filter_map(|e| {
            let id = e.get("object_id")?.as_str()?;
            let file = e.get("file")?.as_str()?;
            if e.get("encoding").and_then(|v| v.as_str()) != Some(PROBE_ENCODING) {
                eprintln!("building {id}: not in {PROBE_ENCODING:?}; re-bake this scene's probes");
                return None;
            }
            let (min, max) = (vec3(e, "min")?, vec3(e, "max")?);
            Some(ProbeEntry {
                object_id: id.to_string(),
                volume: "building".to_string(),
                path: dir.join(file),
                resolution: e.get("resolution").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32,
                range: e.get("range").and_then(serde_json::Value::as_f64).unwrap_or(1.0) as f32,
                min,
                max,
                centre: [f32::midpoint(min[0], max[0]), f32::midpoint(min[1], max[1]), f32::midpoint(min[2], max[2])],
                cell: None,
                cells: None,
                depth: None,
                irradiance: None,
            })
        })
        .collect()
}

pub fn load_scene_probe_index(game_dir: &std::path::Path, scene_name: &str) -> Vec<ProbeEntry> {
    let dir = probe_dir(game_dir, scene_name);
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else {
        return Vec::new();
    };
    let Ok(index) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(entries) = index.get("probes").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries {
        let (Some(id), Some(file)) = (
            e.get("object_id").and_then(|v| v.as_str()),
            e.get("file").and_then(|v| v.as_str()),
        ) else {
            continue;
        };

        let encoding = e.get("encoding").and_then(|v| v.as_str()).unwrap_or("");
        if encoding != PROBE_ENCODING {
            eprintln!(
                "probe {id}: encoding {encoding:?} is not {PROBE_ENCODING:?}; re-bake this scene",
            );
            continue;
        }
        let range = e.get("range").and_then(serde_json::Value::as_f64).unwrap_or(1.0) as f32;
        let vec3 = |key: &str| -> Option<[f32; 3]> {
            let a = e.get(key)?.as_array()?;
            if a.len() != 3 {
                return None;
            }
            let mut out = [0.0f32; 3];
            for (i, v) in a.iter().enumerate() {
                out[i] = v.as_f64()? as f32;
            }
            Some(out)
        };

        let (Some(min), Some(max)) = (vec3("min"), vec3("max")) else {
            eprintln!("probe {id}: index carries no box; re-bake this scene");
            continue;
        };

        let centre = if let Some(c) = vec3("centre") { c } else {
            eprintln!(
                "probe {id}: index carries no capture point; using the box centre, \
                 which is wrong for a subdivided volume -- re-bake this scene",
            );
            [f32::midpoint(min[0], max[0]), f32::midpoint(min[1], max[1]), f32::midpoint(min[2], max[2])]
        };
        let uvec3 = |key: &str| -> Option<[u32; 3]> {
            let a = e.get(key)?.as_array()?;
            if a.len() != 3 {
                return None;
            }
            let mut out = [0u32; 3];
            for (i, v) in a.iter().enumerate() {
                out[i] = u32::try_from(v.as_u64()?).ok()?;
            }
            Some(out)
        };
        let volume = e
            .get("volume")
            .and_then(|v| v.as_str()).map_or_else(|| id.split('#').next().unwrap_or(id).to_string(), str::to_string);
        out.push(ProbeEntry {
            object_id: id.to_string(),
            volume,
            path: dir.join(file),
            resolution: e.get("resolution").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32,
            range,
            min,
            max,
            centre,
            cell: uvec3("cell"),
            cells: uvec3("cells"),

            depth: e
                .get("depth")
                .and_then(|v| v.as_str())
                .filter(|_| e.get("depth_encoding").and_then(|v| v.as_str()) == Some(PROBE_DEPTH_ENCODING))
                .zip(e.get("depth_far").and_then(serde_json::Value::as_f64))
                .map(|(f, far)| (dir.join(f), far as f32)),
            irradiance: e.get("irradiance").and_then(parse_probe_irradiance),
        });
    }
    out
}

#[cfg(test)]
mod probe_decode_tests {
    use super::*;

    fn f16_to_f32(h: u16) -> f32 {
        let sign = u32::from((h >> 15) & 1);
        let exp = i32::from((h >> 10) & 0x1f);
        let mant = u32::from(h & 0x3ff);
        let bits = if exp == 0 {
            if mant == 0 {
                sign << 31
            } else {

                let mut e = -14;
                let mut m = mant;
                while m & 0x400 == 0 {
                    m <<= 1;
                    e -= 1;
                }
                (sign << 31) | (((e + 127) as u32) << 23) | ((m & 0x3ff) << 13)
            }
        } else if exp == 0x1f {
            (sign << 31) | 0x7f80_0000 | (mant << 13)
        } else {
            (sign << 31) | (((exp - 15 + 127) as u32) << 23) | (mant << 13)
        };
        f32::from_bits(bits)
    }

    #[test]
    fn half_floats_round_trip_across_the_range_a_probe_uses() {
        for &v in &[
            0.0, 1e-6, 1e-4, 0.000_607, 0.001, 0.01, 0.1, 0.5, 1.0, 2.5, 16.0, 1000.0,
        ] {
            let back = f16_to_f32(f32_to_f16(v));
            let err = (back - v).abs();

            assert!(
                err <= v * 1e-3 + 6e-8,
                "{v} encoded to {back}, an error of {err}; binary16 carries about \
                 three decimal digits and this is worse than that",
            );
        }
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan(), "a NaN became a finite radiance");
        assert!(f16_to_f32(f32_to_f16(1e30)).is_infinite());
    }

    fn probe_image(texels: &[[u16; 4]]) -> image::ImageBuffer<image::Rgba<u16>, Vec<u16>> {
        let mut img = image::ImageBuffer::new(texels.len() as u32, 1);
        for (x, t) in texels.iter().enumerate() {
            img.put_pixel(x as u32, 0, image::Rgba(*t));
        }
        img
    }

    #[test]
    fn samples_decode_through_the_square_and_the_range() {
        let half = u16::MAX / 2;
        let bytes = probe_faces_f16(&probe_image(&[[half, half, half, u16::MAX]]), 4.0);
        assert_eq!(bytes.len(), 8, "a texel is four half floats");
        let got = f16_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]));

        assert!((got - 1.0).abs() < 2e-3, "decoded {got}, expected 1.0");
    }

    #[test]
    fn alpha_is_neither_squared_nor_scaled() {
        let half = u16::MAX / 2;
        let bytes = probe_faces_f16(&probe_image(&[[0, 0, 0, half]]), 4.0);
        let a = f16_to_f32(u16::from_le_bytes([bytes[6], bytes[7]]));
        assert!(
            (a - 0.5).abs() < 2e-3,
            "alpha decoded to {a}; it went through the colour curve (0.25) or the \
             range (2.0) instead of straight through",
        );
        let sky = probe_faces_f16(&probe_image(&[[0, 0, 0, 0]]), 4.0);
        assert_eq!(f16_to_f32(u16::from_le_bytes([sky[6], sky[7]])), 0.0);
    }

    #[test]
    fn rooms_and_doorways_are_read_from_the_index() {
        let dir = std::env::temp_dir().join(format!("probe_rooms_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_probe_dir(&dir, ",\"encoding\":\"sqrt16\",\"volume\":\"hall\"");
        let idx = dir.join("scenes").join("s.probes").join("index.json");
        let raw = std::fs::read_to_string(&idx).unwrap();
        let with_portal = raw.trim_end().trim_end_matches('}').to_string()
            + ",\"portals\":[{\"min\":[2.5,0,-3.8],\"max\":[3.1,2.2,-2.2],\"axis\":0,\"low\":\"hall\",\"high\":\"hallway#0\"}]}";
        std::fs::write(&idx, with_portal).unwrap();

        let index = load_scene_probe_index(&dir, "s");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].volume, "hall");
        assert_eq!(index[0].resolution, 1);
        let (res, faces) = decode_probe(&index[0]).expect("the entry's file did not decode");
        assert_eq!((res, faces.len()), (1, 6 * 8));
        assert_eq!(load_scene_probes(&dir, "s")[0].volume, "hall");

        let portals = load_scene_portals(&dir, "s");
        assert_eq!(portals.len(), 1);
        assert_eq!(portals[0].axis, 0);
        assert_eq!((portals[0].low.as_str(), portals[0].high.as_str()), ("hall", "hallway#0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_index_takes_the_room_from_the_id() {
        let dir = std::env::temp_dir().join(format!("probe_rooms_old_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_probe_dir(&dir, ",\"encoding\":\"sqrt16\"");
        let idx = dir.join("scenes").join("s.probes").join("index.json");
        let raw = std::fs::read_to_string(&idx).unwrap().replace("\"object_id\":\"p\"", "\"object_id\":\"hall#3\"");
        std::fs::write(&idx, raw).unwrap();
        assert_eq!(load_scene_probe_index(&dir, "s")[0].volume, "hall");
        assert!(load_scene_portals(&dir, "s").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_probe_dir(dir: &std::path::Path, extra: &str) {
        let scenes = dir.join("scenes").join("s.probes");
        std::fs::create_dir_all(&scenes).unwrap();
        let mut img = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::new(1, 6);
        for y in 0..6 {
            img.put_pixel(0, y, image::Rgba([u16::MAX / 2, 0, 0, u16::MAX]));
        }
        img.save(scenes.join("0.png")).unwrap();
        std::fs::write(
            scenes.join("index.json"),
            format!(
                "{{\"probes\":[{{\"object_id\":\"p\",\"file\":\"0.png\",\
                 \"resolution\":1,\"min\":[-1,-1,-1],\"max\":[1,1,1]{extra}}}]}}",
            ),
        )
        .unwrap();
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("probe_gate_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_capture_point_and_grid_cell_are_read_from_the_index() {
        let d = temp_dir("centre");
        write_probe_dir(
            &d,
            &format!(
                ",\"encoding\":\"{PROBE_ENCODING}\",\"range\":1.0,\
                 \"centre\":[0.25,0.5,-0.75],\"cell\":[1,0,2],\"cells\":[2,1,3]"
            ),
        );
        let got = load_scene_probes(&d, "s");
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got.len(), 1, "the probe did not load");
        assert_eq!(got[0].centre, [0.25, 0.5, -0.75], "the capture point was not read from the index");
        assert_eq!(got[0].cell, Some([1, 0, 2]));
        assert_eq!(got[0].cells, Some([2, 1, 3]));
    }

    #[test]
    fn an_index_without_a_capture_point_falls_back_to_the_box_centre() {
        let d = temp_dir("nocentre");
        write_probe_dir(&d, &format!(",\"encoding\":\"{PROBE_ENCODING}\",\"range\":1.0"));
        let got = load_scene_probes(&d, "s");
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got.len(), 1, "an older bake without a capture point was dropped");
        assert_eq!(got[0].centre, [0.0, 0.0, 0.0]);
        assert_eq!(got[0].cells, None);
    }

    #[test]
    fn a_probe_without_this_encoding_is_refused() {
        for (tag, extra) in [
            ("missing", String::new()),
            ("other", ",\"encoding\":\"srgb8\"".to_string()),
        ] {
            let d = temp_dir(tag);
            write_probe_dir(&d, &extra);
            let got = load_scene_probes(&d, "s");
            assert!(
                got.is_empty(),
                "a probe declaring encoding {extra:?} was loaded; its samples mean \
                 something else and every reflection in the scene would be wrong",
            );
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn a_probe_without_a_box_is_refused() {
        let d = temp_dir("nobox");
        let scenes = d.join("scenes").join("s.probes");
        std::fs::create_dir_all(&scenes).unwrap();
        let mut img = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::new(1, 6);
        for y in 0..6 {
            img.put_pixel(0, y, image::Rgba([0, 0, 0, u16::MAX]));
        }
        img.save(scenes.join("0.png")).unwrap();
        std::fs::write(
            scenes.join("index.json"),
            format!(
                "{{\"probes\":[{{\"object_id\":\"p\",\"file\":\"0.png\",\
                 \"resolution\":1,\"encoding\":\"{PROBE_ENCODING}\"}}]}}",
            ),
        )
        .unwrap();
        assert!(
            load_scene_probes(&d, "s").is_empty(),
            "a probe with no box was loaded; it covers no volume and silently \
             contributes nothing",
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_probe_with_this_encoding_loads_as_half_floats() {
        let d = temp_dir("ok");
        write_probe_dir(&d, &format!(",\"encoding\":\"{PROBE_ENCODING}\",\"range\":4.0"));
        let got = load_scene_probes(&d, "s");
        assert_eq!(got.len(), 1, "the encoding this baker writes was refused");
        assert_eq!(got[0].resolution, 1);
        assert_eq!(got[0].faces.len(), 6 * 8, "six texels of four half floats");
        assert_eq!(got[0].min, [-1.0, -1.0, -1.0], "the box did not survive the index");
        assert_eq!(got[0].max, [1.0, 1.0, 1.0]);
        let r = f16_to_f32(u16::from_le_bytes([got[0].faces[0], got[0].faces[1]]));
        assert!((r - 1.0).abs() < 2e-3, "red decoded to {r}; (0.5)^2 * 4.0 is 1.0");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_dim_wall_survives_the_round_trip_as_more_than_three_values() {

        const WALL_MAX: f32 = 0.000_607;
        let codes: Vec<[u16; 4]> = (0..64)
            .map(|i| {
                let v = WALL_MAX * i as f32 / 63.0;
                let c = ((v / 1.0).sqrt() * f32::from(u16::MAX)).round() as u16;
                [c, c, c, u16::MAX]
            })
            .collect();
        let bytes = probe_faces_f16(&probe_image(&codes), 1.0);
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..codes.len() {
            seen.insert(u16::from_le_bytes([bytes[i * 8], bytes[i * 8 + 1]]));
        }
        assert!(
            seen.len() >= 60,
            "64 distinct wall radiances came back as {} distinct half floats; the \
             file carries the detail but the decode is throwing it away",
            seen.len(),
        );
    }
}
