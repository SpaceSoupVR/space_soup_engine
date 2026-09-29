//! Baked cubemap reflections: what a shiny surface sees when the screen cannot
//! say.
//!
//! WHY A PROBE AND NOT ONLY SCREEN-SPACE REFLECTIONS
//!
//! A screen-space reflection can only reflect what is already on screen, and it
//! marches in each eye's own screen space. Those two facts are the same bug in
//! stereo: the ray that leaves the left eye's frame is still inside the right
//! eye's, so one eye shows a marched reflection where the other shows a
//! fallback, and the boundary between them lands in a different place for each
//! eye. The viewer's fusion has nothing consistent to lock onto, and it reads
//! as an unnatural image rather than as a graphical artefact.
//!
//! A probe is a WORLD-SPACE lookup. Both eyes sample the same cubemap, in
//! directions that differ exactly as much as their positions do -- which is the
//! correct disparity, not an artefact of where the frame happens to end. It
//! also knows about the wall behind the viewer, which no screen-space technique
//! can.
//!
//! WHAT IT COSTS AND WHAT IT CANNOT DO
//!
//! One cubemap sample per reflective pixel, against a march plus refinement. In
//! exchange it is STATIC: baked from one point at bake time, so it cannot
//! reflect a hand, a door swinging, or anything else that moves. That is
//! precisely the case screen-space reflection is good at, which is why the two
//! belong together rather than in competition -- the probe is the base and the
//! floor, and the march refines it where it is confident.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The six cube faces, in the order a cubemap array expects them.
///
/// +X, -X, +Y, -Y, +Z, -Z. This is fixed by the graphics API, not by us, and
/// the baker writes its faces in this order so a layer index IS a face index.
pub const CUBE_FACES: usize = 6;

/// Default cubemap resolution per face.
///
/// 128, which is Unreal's default capture size, and the reason is polished stone.
///
/// 64 was chosen when a level had a probe per 2.5 m cell and memory was spent
/// thirty times over. At 64 the shader's blur floor left marble -- roughness
/// 0.05, close to a mirror -- reading a 16 px face, so a doorway or a lamp's
/// floor pool reflected as a soft blob several times its real size. On the
/// headset (2026-09-11) the lighting-sources view painted those blobs as the
/// light-and-dark patches that came and went on the hall walls.
///
/// A level now has one probe per room, so the budget that forced 64 is gone.
///
/// 256 since 2026-09-26: reflections are now TRACED against each probe's
/// per-texel distances, and at 128 a reflected silhouette -- a doorway's edge
/// on the ceiling -- stepped by whole depth texels. 256 halves the steps; the
/// streaming pool holds about sixteen 256 px probes in its 64 MB.
pub const DEFAULT_PROBE_RESOLUTION: u32 = 256;

/// Largest face resolution a probe may ask for.
///
/// A cap rather than a promise: the bake cost is six times the square of this,
/// and a probe is meant to be cheap enough to have several per level.
pub const MAX_PROBE_RESOLUTION: u32 = 256;

/// A baked cubemap reflection, captured at an object's position.
///
/// The object's own cuboid supplies BOTH the capture point and the parallax
/// box -- see [`box_project`] -- so a probe needs no geometry concept of its
/// own. Size it to the room it serves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionProbeDef {
    /// Pixels per cube face. Clamped to [`MAX_PROBE_RESOLUTION`].
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
    pub fn clamped_resolution(&self) -> u32 {
        self.resolution.clamp(4, MAX_PROBE_RESOLUTION)
    }
}

/// The direction to actually sample a probe in, corrected for parallax.
///
/// WHY THE RAW REFLECTION DIRECTION IS WRONG
///
/// A cubemap has no position; sampling it with the mirror direction says "the
/// reflected world is infinitely far away". Do that in a room and the walls
/// reflect a view from the room's centre no matter where you stand, so the
/// reflection slides across the floor as you walk instead of staying put -- the
/// single thing that makes a probe read as fake.
///
/// The fix is to treat the probe as a BOX the size of the room: intersect the
/// reflected ray with that box, and sample toward the point it actually hits,
/// as seen from the probe's own centre. The reflection is then anchored to the
/// walls. This is the standard box projection, and it is exact for a room that
/// really is a box -- which a brush room is.
///
/// `frag` and the box are in world space; the returned vector needs no
/// normalisation by the caller, but is normalised anyway so the two ends of
/// this cannot disagree about that.
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
    // Distance along the ray to each of the six planes. A zero component would
    // divide by zero, and the correct answer there is "never hits that pair",
    // which is what a huge number gives.
    let inv = Vec3::new(
        if dir.x.abs() > 1e-6 { 1.0 / dir.x } else { f32::MAX },
        if dir.y.abs() > 1e-6 { 1.0 / dir.y } else { f32::MAX },
        if dir.z.abs() > 1e-6 { 1.0 / dir.z } else { f32::MAX },
    );
    // CLAMPED INTO THE BOX, as the shader does (`probe_environment`): a room's
    // probe box IS its interior, so a wall fragment lies on its surface, and an
    // MSAA edge fragment extrapolated just past it belongs back on it.
    let frag = frag.clamp(box_min, box_max);
    let t_max = (box_max - frag) * inv;
    let t_min = (box_min - frag) * inv;
    // The nearer of each pair is behind us; the FARTHER is where the ray
    // leaves the box. The smallest of those three is the wall it leaves by.
    let furthest = t_max.max(t_min);
    let dist = furthest.x.min(furthest.y).min(furthest.z);
    // ZERO IS A HIT. A point on a wall whose reflection leaves through that
    // wall reflects the wall at itself -- `dist == 0` -- and rejecting it (the
    // old `<= 0.0`, and `> 0.0` in the shader) sent that one point to the raw
    // direction while every neighbour a hair inside used the corrected one.
    // See `a_point_on_the_wall_reflects_what_its_neighbours_just_inside_do`.
    let to_hit = frag + dir * dist.max(0.0) - probe_centre;
    if !dist.is_finite() || to_hit.length_squared() <= 1e-12 {
        return dir;
    }
    to_hit.normalize()
}

/// Which probe serves a point, by smallest box that contains it.
///
/// Smallest wins so a probe for an alcove beats the one for the hall it sits
/// in. A point in no probe's box gets `None` and falls back to the sky, which
/// is what every outdoor surface wants anyway.
pub fn probe_for_point<'a, T>(point: Vec3, probes: &'a [(T, Vec3, Vec3)]) -> Option<&'a T> {
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

/// The direction a cube face's texel looks along.
///
/// The face order and the axis conventions are the graphics API's, not ours:
/// layer 0 is +X, 1 is -X, 2 is +Y, 3 is -Y, 4 is +Z, 5 is -Z, and the
/// in-face axes are the ones a cube sampler assumes. Getting a sign wrong here
/// produces a probe that is mirrored or rotated on one face only, which reads
/// as a seam rather than as a wrong direction.
///
/// `u` and `v` are texel centres in 0..1 across the face.
pub fn face_direction(face: usize, u: f32, v: f32) -> Vec3 {
    // To -1..1, with v flipped because image rows run downward.
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

/// Where a scene's baked probes live, next to its lightmaps.
pub fn probe_dir(game_dir: &std::path::Path, scene_name: &str) -> std::path::PathBuf {
    game_dir.join("scenes").join(format!("{scene_name}.probes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10x4x10 room centred on the origin, floor at y=0.
    fn room() -> (Vec3, Vec3, Vec3) {
        let min = Vec3::new(-5.0, 0.0, -5.0);
        let max = Vec3::new(5.0, 4.0, 5.0);
        (min, max, (min + max) * 0.5)
    }

    /// THE point of box projection: where you stand changes what you see.
    ///
    /// Without it a cubemap is sampled by direction alone, so every point on a
    /// wall reflects the same thing and the reflection slides with the viewer
    /// instead of staying on the wall. This is the test that a raw mirror
    /// direction would fail.
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

    /// And each looks toward the part of the ceiling actually above it.
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

    /// From the probe's own position the correction must change nothing --
    /// there is no parallax to correct.
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

    /// A point outside the box is taken as the nearest point ON it -- where
    /// the wall it was extrapolated past actually is -- never as a vector built
    /// from a negative distance.
    #[test]
    fn outside_the_box_it_is_projected_from_the_surface() {
        let (min, max, centre) = room();
        let outside = box_project(Vec3::Y, Vec3::new(3.0, 100.0, 0.0), centre, min, max);
        let on = box_project(Vec3::Y, Vec3::new(3.0, max.y, 0.0), centre, min, max);
        assert!(outside.distance(on) < 1e-5, "{outside:?} vs {on:?}");
    }

    /// THE SEAM. A fragment clamped onto a wall whose reflection leaves through
    /// that wall must sample what its neighbours a millimetre inside sample.
    /// Rejecting the zero exit distance there sent exactly the pixels along a
    /// junction to the uncorrected direction: a one-pixel line of the wrong
    /// part of the photograph down every room seam (2026-09-23).
    #[test]
    fn a_point_on_the_wall_reflects_what_its_neighbours_just_inside_do() {
        let (min, max, centre) = room();
        // On the ceiling at the +x wall, reflecting out through that wall.
        let dir = Vec3::new(1.0, -0.3, 0.2).normalize();
        let on = box_project(dir, Vec3::new(max.x, max.y, 1.0), centre, min, max);
        let inside = box_project(dir, Vec3::new(max.x - 1e-3, max.y, 1.0), centre, min, max);
        let past = box_project(dir, Vec3::new(max.x + 0.02, max.y, 1.0), centre, min, max);
        assert!(on.distance(inside) < 1e-2, "on the wall {on:?}, just inside {inside:?}");
        assert!(past.distance(inside) < 1e-2, "extrapolated past it {past:?}, just inside {inside:?}");
    }

    /// Every face's centre must look along its own axis. A sign error here is a
    /// single mirrored face, which reads as a seam rather than as a wrong
    /// direction -- much harder to spot than it is to test.
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

    /// The six faces must cover the whole sphere without a gap or an overlap.
    ///
    /// Checked by the property that identifies a cube map: for every direction,
    /// the largest component picks the face, and that face's own texel really
    /// does point back the same way.
    #[test]
    fn the_faces_tile_the_whole_sphere() {
        let mut seen = [0usize; CUBE_FACES];
        for face in 0..CUBE_FACES {
            for i in 0..8 {
                for j in 0..8 {
                    let u = (i as f32 + 0.5) / 8.0;
                    let v = (j as f32 + 0.5) / 8.0;
                    let d = face_direction(face, u, v);
                    // Which face SHOULD own this direction.
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

    /// The smallest box wins, so an alcove's probe beats the hall's.
    #[test]
    fn the_tightest_probe_serving_a_point_is_chosen() {
        let probes = vec![
            ("hall", Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0)),
            ("alcove", Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.5, 2.0)),
        ];
        // Inside both: the alcove is smaller, so it wins.
        assert_eq!(probe_for_point(Vec3::new(1.0, 1.0, 1.0), &probes), Some(&"alcove"));
        // Inside only the hall.
        assert_eq!(probe_for_point(Vec3::new(-8.0, 1.0, -8.0), &probes), Some(&"hall"));
        // Outside everything -- the sky answers instead.
        assert_eq!(probe_for_point(Vec3::new(50.0, 1.0, 0.0), &probes), None);
    }
    /// Face ORIENTATION, not just which axis a face faces.
    ///
    /// `the_faces_tile_the_whole_sphere` cannot catch this: mirroring a face
    /// keeps every one of its texels pointing within that same face, so the
    /// tiling property still holds while the image on it is flipped. Verified
    /// by breaking it -- flipping face 0 left the tiling test green.
    ///
    /// These are the cube conventions the graphics API defines, not a choice:
    /// for +X the horizontal texel axis runs from +Z to -Z, and the vertical
    /// runs from +Y to -Y. A flipped face reads as a seam where it meets its
    /// neighbours rather than as an obviously wrong direction.
    #[test]
    fn each_face_is_the_right_way_round() {
        // (face, u, v, dominant axis the texel must lean toward)
        let cases: [(usize, f32, f32, Vec3); 12] = [
            (0, 0.0, 0.5, Vec3::Z),      // +X, left edge  -> +Z
            (0, 1.0, 0.5, Vec3::NEG_Z),  // +X, right edge -> -Z
            (1, 0.0, 0.5, Vec3::NEG_Z),  // -X, left edge  -> -Z
            (1, 1.0, 0.5, Vec3::Z),
            (2, 0.5, 0.0, Vec3::NEG_Z),  // +Y, top row    -> -Z
            (2, 0.5, 1.0, Vec3::Z),
            (3, 0.5, 0.0, Vec3::Z),      // -Y, top row    -> +Z
            (3, 0.5, 1.0, Vec3::NEG_Z),
            (4, 0.0, 0.5, Vec3::NEG_X),  // +Z, left edge  -> -X
            (4, 1.0, 0.5, Vec3::X),
            (5, 0.0, 0.5, Vec3::X),      // -Z, left edge  -> +X
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

    /// Every face's vertical axis runs the same way: v = 0 is the top.
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

/// The pixel encoding this loader understands, and the baker writes.
///
/// Square-rooted sixteen-bit samples over a per-probe linear range. See the
/// baker's `encode16` for why an eight-bit sRGB probe was not good enough: a
/// dim room's walls came back holding three distinct values, and every
/// reflection that fell back to one drew contour rings.
pub const PROBE_ENCODING: &str = "sqrt16";

/// One probe as loaded from disk: six faces of RGBA, smallest face first.
pub struct LoadedProbe {
    pub object_id: String,
    /// The room this probe photographs. See [`ProbeEntry::volume`].
    pub volume: String,
    /// Pixels per face.
    pub resolution: u32,
    /// The parallax box this probe is valid for, in WORLD space, as written
    /// by the baker.
    ///
    /// Read from the index rather than looked up from a scene object of the
    /// same name: one authored volume can bake into a grid of cells, and those
    /// cells correspond to no object in the scene.
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// WHERE THE PHOTOGRAPH WAS TAKEN, in world space.
    ///
    /// NOT the middle of `min`/`max`: those are the whole volume's box, shared
    /// by every cell of a subdivided room. Using that as the capture point gave
    /// all of a room's cells one centre, so choosing the nearest cell compared
    /// identical distances and every photograph was projected from the wrong
    /// origin.
    pub centre: [f32; 3],
    /// This probe's cell in its volume's grid, and that grid's size. `None`
    /// from a bake that predates them.
    pub cell: Option<[u32; 3]>,
    pub cells: Option<[u32; 3]>,
    /// Six faces, concatenated in cube order, as LINEAR RGBA half floats --
    /// `resolution^2 * 8` bytes each. Decoded here rather than on the GPU
    /// because the curve and the range are properties of the file, and the
    /// renderer that uploads these bytes does not read the file.
    pub faces: Vec<u8>,
}

/// Decode the baker's square-rooted samples to linear half floats.
fn probe_faces_f16(img: &image::ImageBuffer<image::Rgba<u16>, Vec<u16>>, range: f32) -> Vec<u8> {
    let raw = img.as_raw();
    let mut out = Vec::with_capacity(raw.len() * 2);
    for (i, &v) in raw.iter().enumerate() {
        let unit = v as f32 / u16::MAX as f32;
        // ALPHA IS COVERAGE, not radiance: never square-rooted, never scaled.
        // It says whether the ray hit anything, and a probe whose alpha went
        // through the colour curve would report every sky texel as a partial
        // hit and blend a baked black into the sky.
        let linear = if i % 4 == 3 { unit } else { unit * unit * range };
        out.extend_from_slice(&f32_to_f16(linear).to_le_bytes());
    }
    out
}

/// IEEE binary16 bits for `v`.
///
/// Written out rather than taken from a crate because it is twenty lines and
/// this is the only place in the workspace that needs it. The mantissa is
/// TRUNCATED rather than rounded to nearest: the error is under one part in a
/// thousand of the value, which is far below the quantisation this format
/// exists to remove, and truncation cannot carry into the exponent.
fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    let mant = bits & 0x007f_ffff;
    if exp > 15 {
        // Too large for binary16, or infinite, or NaN. A NaN must stay a NaN:
        // collapsing it to infinity would light a texel at full brightness
        // instead of leaving it obviously broken.
        return sign | if exp == 128 && mant != 0 { 0x7e00 } else { 0x7c00 };
    }
    if exp < -14 {
        // Below the smallest normal binary16: subnormal, or flushed to zero.
        let shift = (-14 - exp) as u32;
        if shift > 24 {
            return sign;
        }
        return sign | ((mant | 0x0080_0000) >> (shift + 13)) as u16;
    }
    sign | (((exp + 15) as u16) << 10) | ((mant >> 13) as u16)
}

/// One probe as the index describes it: everything but the pixels, which
/// [`decode_probe`] reads from `path` when they are wanted. A level streamed
/// through the renderer's probe pool reads each probe's file only when the
/// pool needs it, so nothing has to hold every probe at once.
#[derive(Clone, Debug)]
pub struct ProbeEntry {
    pub object_id: String,
    /// The room this probe photographs. Written by the baker; an older index
    /// without it falls back to the id before any `#cell` suffix, which is the
    /// authored object a subdivided volume's cells came from.
    pub volume: String,
    pub path: std::path::PathBuf,
    /// Pixels per face, from the index. 0 when an older index did not say.
    pub resolution: u32,
    pub range: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub centre: [f32; 3],
    pub cell: Option<[u32; 3]>,
    pub cells: Option<[u32; 3]>,
    /// Its per-texel distances, when the bake wrote them: the file, and the
    /// distance its full scale stands for. See [`decode_probe_depth`].
    pub depth: Option<(std::path::PathBuf, f32)>,
}

/// A doorway between two probe volumes, as the baker found it in the level's
/// carves. See `space_soup::renderer::uniforms::ProbePortal`.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPortal {
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// The axis the opening is thin along: 0 = x, 1 = y, 2 = z.
    pub axis: u32,
    /// The volume on the low side of the opening, and on the high side.
    pub low: String,
    pub high: String,
}

/// A scene's doorways between probe volumes, or none from an older bake.
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

/// Read and decode one probe's faces: `(resolution, linear half floats)`.
/// `None` for a file that is missing or is not six square faces stacked.
pub fn decode_probe(entry: &ProbeEntry) -> Option<(u32, Vec<u8>)> {
    let img = image::open(&entry.path).ok()?;
    let rgba = img.to_rgba16();
    let res = rgba.width();
    // Six square faces stacked vertically is the shape the baker writes.
    // Anything else is a file from another tool or another version, and
    // uploading it would put one face's pixels on another face.
    if res == 0 || rgba.height() != res * CUBE_FACES as u32 {
        return None;
    }
    Some((res, probe_faces_f16(&rgba, entry.range)))
}

/// The format a probe's depth file is written in: per texel, the PLANE of the
/// surface it saw. See `tools/bake`'s `encode_depth_plane`. An index naming
/// anything else -- the older plain distances included -- has its depth
/// ignored, and the renderer projects that probe onto its box instead.
pub const PROBE_DEPTH_ENCODING: &str = "plane16";

/// A probe's depth as the renderer binds it: per texel four half floats,
/// `(nx, ny, nz, h)`, the plane `dot(p - centre, n) = h` of the surface the
/// texel saw, the normal facing the capture point. A texel that saw no surface
/// is `(0, 0, 0, 1)`. Same six stacked faces as the radiance; `None` when the
/// probe has no depth or the file does not match its size.
///
/// All zero -- what an unfilled texture holds -- means "no depth at all", so
/// sky is marked with `h = 1` to stay distinguishable from it.
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
        let unit = |v: u16| v as f32 / 65535.0 * 2.0 - 1.0;
        let n = [unit(r), unit(g), unit(b)];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        let c = (a - 1) as f32 / 65534.0;
        let h = -(c * c) * far;
        out.extend_from_slice(&[h16(n[0] / len), h16(n[1] / len), h16(n[2] / len), h16(h)]);
    }
    Some(out)
}

/// Every probe baked for a scene, decoded, or an empty set when there are none.
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

/// Every probe the scene's index describes, without reading any pixels.
///
/// Deliberately not an error when the directory is missing, for the same reason
/// the lightmap loader is not: a level that has never been probe-baked is a
/// normal state, and it renders correctly with the sky answering instead.
/// A scene's BUILDING OUTSIDES: one cube per hollow brush that holds a room,
/// its six faces the building's six outside faces (the baker's
/// `probe::capture_exterior`), for reflections that leave one building and
/// meet another. Each comes back as a [`ProbeEntry`] -- so [`decode_probe`]
/// reads it -- whose box is the building's and whose volume is `"building"`.
/// Empty for an older bake, which then reflects only ground and sky outdoors.
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
                resolution: e.get("resolution").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                range: e.get("range").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                min,
                max,
                centre: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5],
                cell: None,
                cells: None,
                depth: None,
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
        // NAMED IN THE INDEX, not sniffed from the pixels. A probe set written
        // by an older baker holds eight-bit sRGB, which decodes through this
        // curve to something that is not stale but wrong -- so it is dropped
        // and the sky answers instead, which is exactly what an unbaked level
        // already does.
        let encoding = e.get("encoding").and_then(|v| v.as_str()).unwrap_or("");
        if encoding != PROBE_ENCODING {
            eprintln!(
                "probe {id}: encoding {encoding:?} is not {PROBE_ENCODING:?}; re-bake this scene",
            );
            continue;
        }
        let range = e.get("range").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
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
        // A probe whose box did not survive the bake would be placed at the
        // origin with zero size, which reaches no fragment -- silently doing
        // nothing. Dropping it says so instead.
        let (Some(min), Some(max)) = (vec3("min"), vec3("max")) else {
            eprintln!("probe {id}: index carries no box; re-bake this scene");
            continue;
        };
        // An older bake has no capture point. Fall back to the box centre --
        // correct for an undivided volume, wrong for every cell of a divided
        // one -- and SAY so, because the wrong answer renders without error.
        let centre = match vec3("centre") {
            Some(c) => c,
            None => {
                eprintln!(
                    "probe {id}: index carries no capture point; using the box centre, \
                     which is wrong for a subdivided volume -- re-bake this scene",
                );
                [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5]
            }
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
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| id.split('#').next().unwrap_or(id).to_string());
        out.push(ProbeEntry {
            object_id: id.to_string(),
            volume,
            path: dir.join(file),
            resolution: e.get("resolution").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            range,
            min,
            max,
            centre,
            cell: uvec3("cell"),
            cells: uvec3("cells"),
            // Named with its encoding like the radiance; anything else is left
            // out, and the renderer projects this probe onto its box as it did
            // before depth existed.
            depth: e
                .get("depth")
                .and_then(|v| v.as_str())
                .filter(|_| e.get("depth_encoding").and_then(|v| v.as_str()) == Some(PROBE_DEPTH_ENCODING))
                .zip(e.get("depth_far").and_then(|v| v.as_f64()))
                .map(|(f, far)| (dir.join(f), far as f32)),
        });
    }
    out
}

#[cfg(test)]
mod probe_decode_tests {
    use super::*;

    fn f16_to_f32(h: u16) -> f32 {
        let sign = ((h >> 15) & 1) as u32;
        let exp = ((h >> 10) & 0x1f) as i32;
        let mant = (h & 0x3ff) as u32;
        let bits = if exp == 0 {
            if mant == 0 {
                sign << 31
            } else {
                // Subnormal: renormalise into a float32 normal.
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

    /// The half-float encoder has to be right to better than the quantisation
    /// this whole format change exists to remove.
    #[test]
    fn half_floats_round_trip_across_the_range_a_probe_uses() {
        for &v in &[
            0.0, 1e-6, 1e-4, 0.000_607, 0.001, 0.01, 0.1, 0.5, 1.0, 2.5, 16.0, 1000.0,
        ] {
            let back = f16_to_f32(f32_to_f16(v));
            let err = (back - v).abs();
            // Relative below binary16's three decimal digits, plus one ulp of
            // its smallest SUBNORMAL. Values under 6.1e-5 are subnormal and
            // carry absolute precision rather than relative -- which is still
            // five thousand times finer than the 3e-4 step the eight-bit sRGB
            // encoding this replaces could manage anywhere.
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

    /// The curve is squared on the way back, and scaled by the probe's range.
    #[test]
    fn samples_decode_through_the_square_and_the_range() {
        let half = u16::MAX / 2;
        let bytes = probe_faces_f16(&probe_image(&[[half, half, half, u16::MAX]]), 4.0);
        assert_eq!(bytes.len(), 8, "a texel is four half floats");
        let got = f16_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]));
        // (0.5)^2 * 4.0 = 1.0
        assert!((got - 1.0).abs() < 2e-3, "decoded {got}, expected 1.0");
    }

    /// ALPHA IS COVERAGE. Squaring or scaling it turns every sky texel into a
    /// partial hit and blends baked black into the sky.
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

    /// Write a scene's probe directory: one 1x6 cube, plus whatever index
    /// fields the caller wants to test.
    /// The room and the doorways travel in the index, and the index can be
    /// read without decoding a single pixel -- which is what lets a large
    /// level stream its probes rather than hold them all.
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

    /// An index from before rooms were written names the room after the
    /// authored object a cell came from.
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

    /// The capture point and the grid cell survive the trip from disk.
    ///
    /// Deliberately NOT the box centre: the box is `[-1,-1,-1]..[1,1,1]`, whose
    /// middle is the origin, so a loader that quietly rebuilt the centre from
    /// the box -- the bug this field replaced -- would fail here.
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

    /// An older bake still loads, at the box centre, with no grid.
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

    /// THE GATE. A probe set from an older baker holds eight-bit sRGB in the
    /// same PNG shape, so nothing about the pixels reveals it. Loading one
    /// anyway would not look stale, it would look WRONG -- so an unrecognised
    /// encoding has to be dropped, leaving the sky to answer.
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

    /// A PROBE WITHOUT A BOX IS REFUSED.
    ///
    /// The box used to be looked up from a scene object of the same name; it
    /// now travels in the index, because one authored volume bakes into a grid
    /// of cells that match no object. A probe missing it would be placed at the
    /// origin with zero size -- reaching no fragment, and doing nothing without
    /// saying so.
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

    /// And the encoding this workspace writes is accepted, decoded, and the
    /// right size. Without this the test above would pass on a loader that
    /// refused everything.
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

    /// A DIM WALL KEEPS ITS DETAIL. This is the defect, measured on the
    /// loader's side of the file.
    #[test]
    fn a_dim_wall_survives_the_round_trip_as_more_than_three_values() {
        // The linear range the marble hall's walls occupied. See the baker.
        const WALL_MAX: f32 = 0.000_607;
        let codes: Vec<[u16; 4]> = (0..64)
            .map(|i| {
                let v = WALL_MAX * i as f32 / 63.0;
                let c = ((v / 1.0).sqrt() * u16::MAX as f32).round() as u16;
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
