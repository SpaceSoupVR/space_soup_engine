//! A STANDING MODEL AS REFLECTIONS SEE IT FROM EVERY SIDE: six small pictures
//! -- "cards" -- of one placed model, each looking in through one face of its
//! reflection proxy's box. Every texel holds the lit colour of the first
//! surface its ray met and how far into the box that surface lies.
//!
//! WHY THEY EXIST. The reflection trace finds exactly where a reflected ray
//! meets a model, from the model's distance field
//! (`reflection_proxy::ProxyField`), but it used to colour that hit from the
//! room's photographs, taken from a capture point metres away. Those
//! photographs had the model in them, so the WALL behind a model could only be
//! coloured from a picture with the model in front of it: the sconce's outline,
//! cut out along the photograph's texels, reflected in the polished doorway
//! jamb as a stepped shape the wall does not have (headset, 2026-09-29). The
//! photographs are now taken without the shaped models
//! (`reflection_proxy::shaped_models`), and each model is coloured from its own
//! cards -- the surface-cache cards of Unreal's Lumen, baked instead of
//! captured at run time, since the lighting they hold is the baked lighting.
//!
//! LAYOUT. Card `k = 2a + s` looks along axis `a` of the box's own frame:
//! `s = 0` stands on the box's `+a` face looking toward `-a`, so it shows the
//! sides of the model that face `+a`; `s = 1` stands on the `-a` face looking
//! toward `+a`. Across a card `u` runs along axis `(a + 1) % 3` and `v` along
//! `(a + 2) % 3`, both from the box's minimum to its maximum. A texel's `t` is
//! how far in the surface it saw lies, as a fraction of the box's depth along
//! `a`: 0 at the card's own face, 1 at the opposite one. In the file the six
//! cards are stacked top to bottom in card order, row `y` of a card at `v`,
//! and below them, in the same order, the six cards' NORMALS: which way, in
//! the box's frame, each texel's surface faces -- turned toward its card.
//!
//! WHY NORMALS. A lampshade is a shell millimetres thick, lit inside by its
//! bulb and dark outside. Where a reflection meets the outside, the card
//! looking up into the shade saw the glowing inside at the same place, within
//! any depth tolerance a card can afford -- and the underside of the collar
//! above the shade, which no card sees, sat a few millimetres behind that same
//! glowing inside. Each leak drew a white speck in the sconce's reflection
//! that came and went as the head moved (headset, 2026-09-29 23:25). What the
//! card saw there faces AWAY from the ray, and only its normal can say so.

use glam::Vec3;

/// Six cards a model: one for each face of its box.
pub const CARD_FACES: usize = 6;

/// Texels across a card. One size for every card of a level, so the renderer
/// finds a card in its atlas from the card's number alone: 64 puts a wall
/// sconce's 34 cm at 5 mm and a hanging lamp's 1.4 m at 2.2 cm, finer than the
/// half-resolution reflection pass resolves either.
pub const CARD_RESOLUTION: u32 = 64;

/// The index's name for the cards' pixel format: RGB the probes' square-rooted
/// sixteen bits over a linear `range`; A the depth, 0 for a texel whose rays
/// met nothing, else `1 + t * 65534`. Then the normal cards: RGB the normal's
/// components mapped from -1..1 to 0..65535, A 65535 where the card saw
/// something. See [`encode_normal`].
pub const CARD_ENCODING: &str = "sqrt16+depth16+normal16";

/// `t` for a texel whose rays met nothing: past the box, so no hit ever
/// matches it.
pub const CARD_MISS: f32 = 2.0;

/// Card `face`'s axes: `(a, b, c, sign)` -- the axis it looks along, the axes
/// `u` and `v` run along, and `+1` for the card on the `+a` face, `-1` for the
/// one on the `-a` face.
pub fn card_axes(face: usize) -> (usize, usize, usize, f32) {
    let a = (face / 2).min(2);
    let sign = if face % 2 == 0 { 1.0 } else { -1.0 };
    (a, (a + 1) % 3, (a + 2) % 3, sign)
}

/// The direction card `face` looks, in the box's own frame.
pub fn card_view(face: usize) -> Vec3 {
    let (a, _, _, sign) = card_axes(face);
    let mut d = Vec3::ZERO;
    d[a] = -sign;
    d
}

/// The point, in the box's own frame, that card `face` shows at `(u, v)`, `t`
/// of the way into a box of half-size `half`.
pub fn card_point(face: usize, u: f32, v: f32, t: f32, half: Vec3) -> Vec3 {
    let (a, b, c, sign) = card_axes(face);
    let mut p = Vec3::ZERO;
    p[a] = sign * half[a] * (1.0 - 2.0 * t);
    p[b] = (2.0 * u - 1.0) * half[b];
    p[c] = (2.0 * v - 1.0) * half[c];
    p
}

/// Where the box-frame point `p` lies on card `face`: `(u, v, t)`. The inverse
/// of [`card_point`]; what the renderer's `probe_card_colour` computes.
pub fn card_coords(face: usize, p: Vec3, half: Vec3) -> (f32, f32, f32) {
    let (a, b, c, sign) = card_axes(face);
    let h = half.max(Vec3::splat(1e-6));
    (p[b] / (2.0 * h[b]) + 0.5, p[c] / (2.0 * h[c]) + 0.5, 0.5 * (1.0 - sign * p[a] / h[a]))
}

/// One texel as the file stores it. `rgb` linear radiance, `t` as in the
/// module notes or `None` for a texel that saw nothing.
pub fn encode_texel(rgb: Vec3, t: Option<f32>, range: f32) -> [u16; 4] {
    let unit = |v: f32| ((v / range.max(1e-6)).clamp(0.0, 1.0).sqrt() * 65535.0).round() as u16;
    let depth = t.map_or(0, |t| 1 + (t.clamp(0.0, 1.0) * 65534.0).round() as u16);
    [unit(rgb.x), unit(rgb.y), unit(rgb.z), depth]
}

/// One normal texel as the file stores it: the unit normal, box frame, of the
/// surface the texel saw, turned toward its card; `None` for nothing.
pub fn encode_normal(n: Option<Vec3>) -> [u16; 4] {
    match n {
        Some(n) => {
            let unit = |v: f32| (((v.clamp(-1.0, 1.0) + 1.0) * 0.5) * 65535.0).round() as u16;
            [unit(n.x), unit(n.y), unit(n.z), u16::MAX]
        }
        None => [0; 4],
    }
}

/// [`encode_normal`] undone: the normal, or zero where the card saw nothing.
pub fn decode_normal(texel: [u16; 4]) -> [f32; 3] {
    if texel[3] == 0 {
        return [0.0; 3];
    }
    let c = |v: u16| v as f32 / 65535.0 * 2.0 - 1.0;
    [c(texel[0]), c(texel[1]), c(texel[2])]
}

/// [`encode_texel`] undone: linear RGB and `t`, [`CARD_MISS`] for nothing.
pub fn decode_texel(texel: [u16; 4], range: f32) -> [f32; 4] {
    let linear = |v: u16| {
        let c = v as f32 / 65535.0;
        c * c * range
    };
    let t = if texel[3] == 0 { CARD_MISS } else { (texel[3] - 1) as f32 / 65534.0 };
    [linear(texel[0]), linear(texel[1]), linear(texel[2]), t]
}

/// One placed model's six cards, decoded.
#[derive(Clone, Debug)]
pub struct LoadedCards {
    /// The object they picture: the renderer matches them to its proxy by it.
    pub object_id: String,
    /// Texels across each card.
    pub resolution: u32,
    /// `CARD_FACES * resolution^2` texels, card after card, row after row:
    /// linear RGB and `t` (see [`decode_texel`]).
    pub texels: Vec<[f32; 4]>,
    /// Which way each texel's surface faces, in the box's frame, laid out as
    /// `texels`: zero where the card saw nothing (see [`decode_normal`]).
    pub normals: Vec<[f32; 3]>,
}

/// Every model's cards a scene's probe bake wrote, or none: an older bake has
/// none, and its photographs still hold the models.
pub fn load_scene_cards(game_dir: &std::path::Path, scene_name: &str) -> Vec<LoadedCards> {
    let dir = crate::reflection_probe::probe_dir(game_dir, scene_name);
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else { return Vec::new() };
    let Ok(index) = serde_json::from_str::<serde_json::Value>(&raw) else { return Vec::new() };
    let Some(entries) = index.get("cards").and_then(|c| c.as_array()) else { return Vec::new() };
    entries
        .iter()
        .filter_map(|e| {
            let id = e.get("object_id")?.as_str()?;
            if e.get("encoding").and_then(|v| v.as_str()) != Some(CARD_ENCODING) {
                eprintln!("cards of {id}: not in {CARD_ENCODING:?}; re-bake this scene's probes");
                return None;
            }
            let range = e.get("range").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let img = image::open(dir.join(e.get("file")?.as_str()?)).ok()?.to_rgba16();
            let res = img.width();
            if res == 0 || img.height() != 2 * res * CARD_FACES as u32 {
                eprintln!("cards of {id}: {}x{} is not six square cards and their normals", img.width(), img.height());
                return None;
            }
            let half = (res * res) as usize * CARD_FACES;
            let pixels: Vec<[u16; 4]> = img.pixels().map(|p| p.0).collect();
            let texels = pixels[..half].iter().map(|&p| decode_texel(p, range)).collect();
            let normals = pixels[half..].iter().map(|&p| decode_normal(p)).collect();
            Some(LoadedCards { object_id: id.to_string(), resolution: res, texels, normals })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each card stands on its own face and looks through the box to the
    /// opposite one.
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

    /// `card_coords` is `card_point` undone, on every card, anywhere in the box.
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

    /// The card on +x and the one on -x show a point on the +x side at the
    /// same (u, v), at depths that add to one: two views of one box.
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
        // A surface on the card's own face is a hit, not a miss.
        assert_eq!(decode_texel(encode_texel(rgb, Some(0.0), range), range)[3], 0.0);
    }

    /// A normal comes back to within the file's precision, and a texel that
    /// saw nothing comes back as no direction at all.
    #[test]
    fn a_normal_survives_the_file() {
        for n in [Vec3::X, -Vec3::Y, Vec3::new(0.48, -0.6, 0.64)] {
            let back = Vec3::from(decode_normal(encode_normal(Some(n))));
            assert!((back - n).abs().max_element() < 1e-4, "{n} came back as {back}");
        }
        assert_eq!(decode_normal(encode_normal(None)), [0.0; 3]);
    }
}
