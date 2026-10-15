use glam::Vec3;

pub const CARD_FACES: usize = 6;

pub const CARD_RESOLUTION: u32 = 64;

pub const CARD_ENCODING: &str = "sqrt16+depth16+normal16+albedo16";

pub const CARD_ENCODING_WITHOUT_ALBEDO: &str = "sqrt16+depth16+normal16";

pub const CARD_MISS: f32 = 2.0;

pub fn card_axes(face: usize) -> (usize, usize, usize, f32) {
    let a = (face / 2).min(2);
    let sign = if face % 2 == 0 { 1.0 } else { -1.0 };
    (a, (a + 1) % 3, (a + 2) % 3, sign)
}

pub fn card_view(face: usize) -> Vec3 {
    let (a, _, _, sign) = card_axes(face);
    let mut d = Vec3::ZERO;
    d[a] = -sign;
    d
}

pub fn card_point(face: usize, u: f32, v: f32, t: f32, half: Vec3) -> Vec3 {
    let (a, b, c, sign) = card_axes(face);
    let mut p = Vec3::ZERO;
    p[a] = sign * half[a] * (1.0 - 2.0 * t);
    p[b] = (2.0 * u - 1.0) * half[b];
    p[c] = (2.0 * v - 1.0) * half[c];
    p
}

pub fn card_coords(face: usize, p: Vec3, half: Vec3) -> (f32, f32, f32) {
    let (a, b, c, sign) = card_axes(face);
    let h = half.max(Vec3::splat(1e-6));
    (p[b] / (2.0 * h[b]) + 0.5, p[c] / (2.0 * h[c]) + 0.5, 0.5 * (1.0 - sign * p[a] / h[a]))
}

pub fn encode_texel(rgb: Vec3, t: Option<f32>, range: f32) -> [u16; 4] {
    let unit = |v: f32| ((v / range.max(1e-6)).clamp(0.0, 1.0).sqrt() * 65535.0).round() as u16;
    let depth = t.map_or(0, |t| 1 + (t.clamp(0.0, 1.0) * 65534.0).round() as u16);
    [unit(rgb.x), unit(rgb.y), unit(rgb.z), depth]
}

pub fn encode_normal(n: Option<Vec3>) -> [u16; 4] {
    match n {
        Some(n) => {
            let unit = |v: f32| (((v.clamp(-1.0, 1.0) + 1.0) * 0.5) * 65535.0).round() as u16;
            [unit(n.x), unit(n.y), unit(n.z), u16::MAX]
        }
        None => [0; 4],
    }
}

pub fn decode_normal(texel: [u16; 4]) -> [f32; 3] {
    if texel[3] == 0 {
        return [0.0; 3];
    }
    let c = |v: u16| v as f32 / 65535.0 * 2.0 - 1.0;
    [c(texel[0]), c(texel[1]), c(texel[2])]
}

pub fn encode_albedo(a: Option<Vec3>) -> [u16; 4] {
    match a {
        Some(a) => {
            let unit = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
            [unit(a.x), unit(a.y), unit(a.z), u16::MAX]
        }
        None => [0; 4],
    }
}

pub fn decode_albedo(texel: [u16; 4]) -> [f32; 3] {
    if texel[3] == 0 {
        return [0.0; 3];
    }
    [texel[0], texel[1], texel[2]].map(|v| v as f32 / 65535.0)
}

pub fn decode_texel(texel: [u16; 4], range: f32) -> [f32; 4] {
    let linear = |v: u16| {
        let c = v as f32 / 65535.0;
        c * c * range
    };
    let t = if texel[3] == 0 { CARD_MISS } else { (texel[3] - 1) as f32 / 65534.0 };
    [linear(texel[0]), linear(texel[1]), linear(texel[2]), t]
}

pub const GLARE_BRIGHT: f32 = 4.0;

pub const GLARE_ROWS: usize = 18;
pub const GLARE_COLS: usize = 36;

pub const GLARE_BULB_ROWS: usize = 180;

pub fn glare_direction(row: usize, col: usize, rows: usize, cols: usize) -> Vec3 {
    let theta = (row as f32 + 0.5) / rows.max(1) as f32 * std::f32::consts::PI;
    let phi = (col as f32 + 0.5) / cols.max(1) as f32 * std::f32::consts::TAU;
    Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlareTable {
    pub rows: usize,
    pub cols: usize,

    pub flux: Vec<f32>,

    pub centre: Vec<Vec3>,

    pub split: Option<GlareSplit>,

    pub air: Option<GlareAir>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlareAir {

    pub min: Vec3,
    pub max: Vec3,

    pub dims: [usize; 3],

    pub bulb: Vec3,

    pub seen: Vec<f32>,

    pub reach: f32,
}

impl GlareAir {

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "min": self.min.to_array(),
            "max": self.max.to_array(),
            "dims": self.dims,
            "bulb": self.bulb.to_array(),
            "seen": self.seen.iter().map(|s| (s.clamp(0.0, 1.0) * 255.0).round() as u8).collect::<Vec<u8>>(),
            "reach": self.reach,
        })
    }

    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let vec3 = |key: &str| -> Option<Vec3> {
            let a = v.get(key)?.as_array()?;
            (a.len() == 3).then_some(())?;
            Some(Vec3::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32))
        };
        let dims = v.get("dims")?.as_array()?;
        let dims: Vec<usize> = dims.iter().map(|d| d.as_u64().map(|d| d as usize)).collect::<Option<_>>()?;
        let dims: [usize; 3] = dims.try_into().ok()?;
        let seen: Vec<f32> = v.get("seen")?.as_array()?.iter().map(|s| s.as_f64().map(|s| (s / 255.0) as f32)).collect::<Option<_>>()?;
        (dims.iter().all(|&d| d >= 2) && seen.len() == dims[0] * dims[1] * dims[2])
            .then_some(())?;

        let reach = v.get("reach").and_then(|r| r.as_f64()).map_or(0.0, |r| r as f32).max(0.0);
        Some(Self { min: vec3("min")?, max: vec3("max")?, dims, bulb: vec3("bulb")?, seen, reach })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlareSplit {

    pub bulb: Vec<f32>,

    pub bulb_centre: Vec<Vec3>,

    pub lit_centre: Vec<Vec3>,

    pub lit_spread: Vec<f32>,

    pub bulb_fine: Option<GlareBulbRows>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlareBulbRows {
    pub rows: usize,
    pub flux: Vec<f32>,
    pub centre: Vec<Vec3>,
}

impl GlareTable {

    pub fn to_json(&self) -> serde_json::Value {
        let flat = |v: &[Vec3]| v.iter().flat_map(|c| c.to_array()).collect::<Vec<f32>>();
        let mut out = serde_json::json!({
            "rows": self.rows,
            "cols": self.cols,
            "flux": self.flux,
            "centre": flat(&self.centre),
        });
        if let Some(split) = &self.split {
            out["bulb"] = serde_json::json!(split.bulb);
            out["bulb_centre"] = serde_json::json!(flat(&split.bulb_centre));
            out["lit_centre"] = serde_json::json!(flat(&split.lit_centre));
            out["lit_spread"] = serde_json::json!(split.lit_spread);
            if let Some(fine) = &split.bulb_fine {

                let short = |v: &[f32]| v.iter().map(|x| (*x as f64 * 1e4).round() / 1e4).collect::<Vec<f64>>();
                out["bulb_rows"] = serde_json::json!(fine.rows);
                out["bulb_fine"] = serde_json::json!(short(&fine.flux));
                out["bulb_fine_centre"] = serde_json::json!(short(&flat(&fine.centre)));
            }
        }
        if let Some(air) = &self.air {
            out["air"] = air.to_json();
        }
        out
    }

    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let rows = v.get("rows")?.as_u64()? as usize;
        let cols = v.get("cols")?.as_u64()? as usize;
        let numbers = |key: &str| -> Option<Vec<f32>> {
            v.get(key)?.as_array()?.iter().map(|x| x.as_f64().map(|x| x as f32)).collect()
        };
        let flux = numbers("flux")?;
        let flat = numbers("centre")?;
        let n = rows * cols;
        if n == 0 || flux.len() != n || flat.len() != 3 * n {
            return None;
        }
        let points = |flat: Vec<f32>| -> Vec<Vec3> { flat.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect() };
        let centre = points(flat);

        let split = (|| {
            let (bulb, bulb_centre) = (numbers("bulb")?, numbers("bulb_centre")?);
            let (lit_centre, lit_spread) = (numbers("lit_centre")?, numbers("lit_spread")?);

            let bulb_fine = (|| {
                let rows = v.get("bulb_rows")?.as_u64()? as usize;
                let (flux, centre) = (numbers("bulb_fine")?, numbers("bulb_fine_centre")?);
                (rows > 0 && flux.len() == rows * cols && centre.len() == 3 * rows * cols)
                    .then(|| GlareBulbRows { rows, flux, centre: points(centre) })
            })();
            (bulb.len() == n && bulb_centre.len() == 3 * n && lit_centre.len() == 3 * n && lit_spread.len() == n).then(|| {
                GlareSplit { bulb, bulb_centre: points(bulb_centre), lit_centre: points(lit_centre), lit_spread, bulb_fine }
            })
        })();

        let air = v.get("air").and_then(GlareAir::from_json);
        Some(Self { rows, cols, flux, centre, split, air })
    }
}

#[derive(Clone, Debug)]
pub struct LoadedCards {

    pub object_id: String,

    pub resolution: u32,

    pub texels: Vec<[f32; 4]>,

    pub normals: Vec<[f32; 3]>,

    pub albedo: Vec<[f32; 3]>,

    pub glare: Option<GlareTable>,
}

pub fn load_scene_cards(game_dir: &std::path::Path, scene_name: &str) -> Vec<LoadedCards> {
    let dir = crate::reflection_probe::probe_dir(game_dir, scene_name);
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else { return Vec::new() };
    let Ok(index) = serde_json::from_str::<serde_json::Value>(&raw) else { return Vec::new() };
    let Some(entries) = index.get("cards").and_then(|c| c.as_array()) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|e| {
            let id = e.get("object_id")?.as_str()?;

            let blocks = match e.get("encoding").and_then(|v| v.as_str()) {
                Some(CARD_ENCODING) => 3,
                Some(CARD_ENCODING_WITHOUT_ALBEDO) => 2,
                _ => {
                    eprintln!("cards of {id}: not in {CARD_ENCODING:?}; re-bake this scene's probes");
                    return None;
                }
            };
            let range = e.get("range").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let img = image::open(dir.join(e.get("file")?.as_str()?)).ok()?.to_rgba16();
            let res = img.width();
            if res == 0 || img.height() != blocks * res * CARD_FACES as u32 {
                eprintln!("cards of {id}: {}x{} is not {blocks} blocks of six square cards", img.width(), img.height());
                return None;
            }
            let block = (res * res) as usize * CARD_FACES;
            let pixels: Vec<[u16; 4]> = img.pixels().map(|p| p.0).collect();
            let texels = pixels[..block].iter().map(|&p| decode_texel(p, range)).collect();
            let normals = pixels[block..2 * block].iter().map(|&p| decode_normal(p)).collect();
            let albedo = pixels[2 * block..].iter().map(|&p| decode_albedo(p)).collect();

            let glare = e.get("glare").and_then(|f| f.as_str()).and_then(|f| {
                let table = std::fs::read_to_string(dir.join(f))
                    .ok()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                    .and_then(|v| GlareTable::from_json(&v));
                if table.is_none() {
                    eprintln!("cards of {id}: glare table {f} did not load; re-bake this scene's probes");
                }
                table
            });
            Some(LoadedCards { object_id: id.to_string(), resolution: res, texels, normals, albedo, glare })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glare_tables_directions_run_pole_to_pole_and_round() {
        let (rows, cols) = (GLARE_ROWS, GLARE_COLS);
        for row in 0..rows {
            for col in 0..cols {
                assert!((glare_direction(row, col, rows, cols).length() - 1.0).abs() < 1e-5);
            }
        }
        assert!(glare_direction(0, 0, rows, cols).y > 0.98, "row 0 looks down from above");
        assert!(glare_direction(rows - 1, 0, rows, cols).y < -0.98, "the last row looks up from below");
        let equator = rows / 2;
        let first = glare_direction(equator, 0, rows, cols);
        let quarter = glare_direction(equator, cols / 4, rows, cols);
        assert!(first.x > 0.98 && first.z > 0.0, "column 0 is just past +x: {first}");
        assert!(quarter.z > 0.98, "a quarter round is +z: {quarter}");
    }

    #[test]
    fn a_glare_table_round_trips_through_its_file() {
        let (rows, cols) = (3, 4);
        let t = GlareTable {
            rows,
            cols,
            flux: (0..rows * cols).map(|i| 0.125 * i as f32 + 0.003).collect(),
            centre: (0..rows * cols).map(|i| Vec3::new(i as f32 * 0.01, -0.07, 0.25 - i as f32 * 0.02)).collect(),
            split: None,
            air: None,
        };
        let back = GlareTable::from_json(&serde_json::from_str(&t.to_json().to_string()).unwrap());
        assert_eq!(back.as_ref(), Some(&t));
        let mut short = t.to_json();
        short["flux"].as_array_mut().unwrap().pop();
        assert_eq!(GlareTable::from_json(&short), None);

        let n = rows * cols;
        let split = GlareTable {
            split: Some(GlareSplit {
                bulb: (0..n).map(|i| 0.05 * i as f32).collect(),
                bulb_centre: (0..n).map(|i| Vec3::new(0.0, -0.1 * i as f32, 0.01)).collect(),
                lit_centre: (0..n).map(|i| Vec3::new(0.02 * i as f32, 0.03, 0.0)).collect(),
                lit_spread: (0..n).map(|i| 0.1 + 0.01 * i as f32).collect(),
                bulb_fine: Some(GlareBulbRows {
                    rows: 5,

                    flux: (0..5 * cols).map(|i| 0.25 * i as f32).collect(),
                    centre: (0..5 * cols).map(|i| Vec3::new(0.0, -0.5 * i as f32, 0.0125)).collect(),
                }),
            }),
            ..t.clone()
        };
        let back = GlareTable::from_json(&serde_json::from_str(&split.to_json().to_string()).unwrap());
        assert_eq!(back.as_ref(), Some(&split));
        let mut cut = split.to_json();
        cut["lit_spread"].as_array_mut().unwrap().pop();
        assert_eq!(GlareTable::from_json(&cut), Some(t.clone()));

        let air = GlareTable {
            air: Some(GlareAir {
                min: Vec3::new(-0.5, -1.25, -0.5),
                max: Vec3::new(0.5, -0.25, 0.5),
                dims: [2, 3, 4],
                bulb: Vec3::new(0.0, -0.75, 0.0),
                seen: (0..24).map(|i| (i * 11 % 256) as f32 / 255.0).collect(),
                reach: 0.375,
            }),
            ..t.clone()
        };
        let back = GlareTable::from_json(&serde_json::from_str(&air.to_json().to_string()).unwrap());
        assert_eq!(back.as_ref(), Some(&air));

        let mut unmeasured = air.to_json();
        unmeasured["air"].as_object_mut().unwrap().remove("reach");
        let back = GlareTable::from_json(&unmeasured).unwrap();
        assert_eq!(back.air.map(|a| a.reach), Some(0.0));
        let mut cut = air.to_json();
        cut["air"]["seen"].as_array_mut().unwrap().pop();
        assert_eq!(GlareTable::from_json(&cut), Some(t));
    }

    #[test]
    fn each_card_looks_in_through_its_own_face() {
        let half = Vec3::new(0.2, 0.7, 0.1);
        for face in 0..CARD_FACES {
            let front = card_point(face, 0.5, 0.5, 0.0, half);
            let back = card_point(face, 0.5, 0.5, 1.0, half);
            let (a, _, _, sign) = card_axes(face);
            assert!((front[a] - sign * half[a]).abs() < 1e-6, "card {face} starts at {front}");
            assert!((back[a] + sign * half[a]).abs() < 1e-6, "card {face} ends at {back}");
            assert!(((back - front).normalize() - card_view(face)).length() < 1e-6, "card {face} looks {}", card_view(face));
        }
    }

    #[test]
    fn a_point_is_found_where_its_card_shows_it() {
        let half = Vec3::new(0.3, 0.05, 0.9);
        for face in 0..CARD_FACES {
            for (u, v, t) in [(0.1, 0.2, 0.3), (0.9, 0.5, 0.0), (0.5, 0.95, 1.0), (0.0, 1.0, 0.61)] {
                let p = card_point(face, u, v, t, half);
                let (u2, v2, t2) = card_coords(face, p, half);
                assert!((u - u2).abs() < 1e-5 && (v - v2).abs() < 1e-5 && (t - t2).abs() < 1e-5, "card {face}: {:?}", (u2, v2, t2));
            }
        }
    }

    #[test]
    fn opposite_cards_see_one_point_at_complementary_depths() {
        let half = Vec3::new(0.5, 0.25, 0.4);
        let p = Vec3::new(0.3, -0.1, 0.2);
        let (u0, v0, t0) = card_coords(0, p, half);
        let (u1, v1, t1) = card_coords(1, p, half);
        assert!((u0 - u1).abs() < 1e-6 && (v0 - v1).abs() < 1e-6);
        assert!((t0 + t1 - 1.0).abs() < 1e-6, "{t0} + {t1}");
        assert!(t0 < t1, "the +x card sees a point on the +x side nearer");
    }

    #[test]
    fn a_texel_survives_the_file() {
        let range = 40.0;
        let rgb = Vec3::new(0.02, 3.5, 39.0);
        let [r, g, b, t] = decode_texel(encode_texel(rgb, Some(0.25), range), range);
        assert!((Vec3::new(r, g, b) - rgb).abs().max_element() < 0.02, "{r} {g} {b}");
        assert!((t - 0.25).abs() < 1e-4);
        assert_eq!(decode_texel(encode_texel(rgb, None, range), range)[3], CARD_MISS);

        assert_eq!(decode_texel(encode_texel(rgb, Some(0.0), range), range)[3], 0.0);
    }

    #[test]
    fn a_normal_survives_the_file() {
        for n in [Vec3::X, -Vec3::Y, Vec3::new(0.48, -0.6, 0.64)] {
            let back = Vec3::from(decode_normal(encode_normal(Some(n))));
            assert!((back - n).abs().max_element() < 1e-4, "{n} came back as {back}");
        }
        assert_eq!(decode_normal(encode_normal(None)), [0.0; 3]);
    }

    #[test]
    fn an_albedo_survives_the_file() {
        for a in [Vec3::new(0.62, 0.55, 0.47), Vec3::ZERO, Vec3::ONE, Vec3::new(0.037, 0.04, 0.05)] {
            let back = Vec3::from(decode_albedo(encode_albedo(Some(a))));
            assert!((back - a).abs().max_element() < 1e-4, "{a} came back as {back}");
        }
        assert_eq!(decode_albedo(encode_albedo(None)), [0.0; 3]);
        assert_eq!(decode_albedo(encode_albedo(Some(Vec3::splat(1.5)))), [1.0; 3]);
    }
}
