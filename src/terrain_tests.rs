//! Heightfield sampling and patch generation.
//!
//! `height_at` is the query everything else leans on -- locomotion, foot
//! placement, spawn resolution -- so it is tested against hand-computable
//! fields rather than against fixtures, and the interpolation is checked at the
//! places where an off-by-one in the index maths hides: sample points, cell
//! midpoints, and the far edge.

use glam::Vec3;

use crate::physics::Aabb;
use crate::terrain::{Heightfield, TerrainSource};

const FULL: u16 = u16::MAX;

/// A 3x3 field, 20m square, heights 0..10m. Rows run +z, columns +x.
fn ramp() -> Heightfield {
    // Height rises with x: 0, 5, 10 across each row.
    let half = FULL / 2;
    let samples = vec![
        0, half, FULL,
        0, half, FULL,
        0, half, FULL,
    ];
    Heightfield::new(samples, [3, 3], [20.0, 20.0], [0.0, 10.0], Vec3::ZERO).unwrap()
}

fn flat_at(height: u16) -> Heightfield {
    Heightfield::new(vec![height; 4], [2, 2], [10.0, 10.0], [0.0, 100.0], Vec3::ZERO).unwrap()
}

#[test]
fn rejects_a_sample_count_that_does_not_match_the_resolution() {
    // A truncated read would otherwise produce terrain silently flat at one edge.
    let err = Heightfield::new(vec![0; 8], [3, 3], [10.0, 10.0], [0.0, 1.0], Vec3::ZERO)
        .unwrap_err();
    assert!(err.contains("needs 9"), "unhelpful error: {err}");
}

#[test]
fn rejects_a_degenerate_resolution() {
    assert!(Heightfield::new(vec![0], [1, 1], [10.0, 10.0], [0.0, 1.0], Vec3::ZERO).is_err());
}

#[test]
fn rejects_a_zero_size() {
    assert!(Heightfield::new(vec![0; 4], [2, 2], [0.0, 10.0], [0.0, 1.0], Vec3::ZERO).is_err());
}

#[test]
fn decodes_little_endian_u16_samples() {
    // 0x0000 and 0xFFFF, four of them.
    let bytes = [0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF];
    let field =
        Heightfield::from_raw_le(&bytes, [2, 2], [10.0, 10.0], [0.0, 100.0], Vec3::ZERO).unwrap();
    assert_eq!(field.height_at(0.0, 0.0), Some(0.0));
    assert_eq!(field.height_at(10.0, 0.0), Some(100.0));
}

#[test]
fn rejects_an_odd_byte_count() {
    let err = Heightfield::from_raw_le(&[0, 0, 1], [2, 2], [1.0, 1.0], [0.0, 1.0], Vec3::ZERO)
        .unwrap_err();
    assert!(err.contains("whole number of u16"), "unhelpful error: {err}");
}

#[test]
fn height_matches_the_samples_at_sample_points() {
    let field = ramp();
    assert_eq!(field.height_at(0.0, 0.0), Some(0.0));
    assert!((field.height_at(20.0, 0.0).unwrap() - 10.0).abs() < 1e-3);
}

#[test]
fn interpolates_between_samples_rather_than_stepping() {
    // Halfway across the first 10m cell of a 0..5m ramp is 2.5m. Nearest-sample
    // lookup would answer 0 here, and the player would climb a staircase.
    let field = ramp();
    assert!((field.height_at(5.0, 0.0).unwrap() - 2.5).abs() < 1e-3);
}

#[test]
fn interpolates_along_z_too() {
    // Heights rise with z instead: 0 on the first row, 10 on the last.
    let half = FULL / 2;
    let field = Heightfield::new(
        vec![0, 0, 0, half, half, half, FULL, FULL, FULL],
        [3, 3],
        [20.0, 20.0],
        [0.0, 10.0],
        Vec3::ZERO,
    )
    .unwrap();
    assert!((field.height_at(0.0, 5.0).unwrap() - 2.5).abs() < 1e-3);
    assert!((field.height_at(0.0, 20.0).unwrap() - 10.0).abs() < 1e-3);
}

#[test]
fn answers_none_outside_the_field() {
    let field = ramp();
    assert_eq!(field.height_at(-0.1, 10.0), None);
    assert_eq!(field.height_at(20.1, 10.0), None);
    assert_eq!(field.height_at(10.0, -0.1), None);
    assert_eq!(field.height_at(10.0, 20.1), None);
}

#[test]
fn the_far_edge_is_inside_the_field() {
    // The exact maximum lands on the last sample; an unclamped index would run
    // off the end of the array here.
    let field = ramp();
    assert!(field.height_at(20.0, 20.0).is_some());
}

#[test]
fn origin_offsets_the_field_in_all_three_axes() {
    let field = Heightfield::new(
        vec![0; 4],
        [2, 2],
        [10.0, 10.0],
        [0.0, 100.0],
        Vec3::new(100.0, 7.0, -50.0),
    )
    .unwrap();

    assert_eq!(field.height_at(0.0, 0.0), None, "should be outside now");
    assert_eq!(field.height_at(105.0, -45.0), Some(7.0), "origin.y shifts the height");
}

#[test]
fn bounds_cover_the_whole_field_and_its_height_range() {
    let field = Heightfield::new(
        vec![0; 4],
        [2, 2],
        [30.0, 40.0],
        [-5.0, 12.0],
        Vec3::new(1.0, 2.0, 3.0),
    )
    .unwrap();

    let b = field.bounds();
    assert_eq!(b.min, Vec3::new(1.0, -3.0, 3.0));
    assert_eq!(b.max, Vec3::new(31.0, 14.0, 43.0));
}

// ------------------------------------------------------------------- patches

fn whole_of(field: &Heightfield) -> Aabb {
    field.bounds()
}

#[test]
fn a_full_patch_covers_every_sample() {
    let field = ramp();
    let patch = field.patch(whole_of(&field), 1);
    assert_eq!(patch.positions.len(), 9);
    // Two triangles per cell, four cells.
    assert_eq!(patch.indices.len(), 4 * 6);
}

#[test]
fn a_coarser_step_produces_fewer_vertices() {
    let field = flat_grid(5, 5);
    let fine = field.patch(whole_of(&field), 1).positions.len();
    let coarse = field.patch(whole_of(&field), 2).positions.len();
    assert!(coarse < fine, "step 2 gave {coarse}, step 1 gave {fine}");
}

#[test]
fn a_coarse_patch_still_reaches_the_far_edge() {
    // The reason the last index is appended explicitly: a step that does not
    // divide the field evenly would otherwise stop short and leave a gap
    // between this patch and its neighbour.
    let field = flat_grid(5, 5);
    let patch = field.patch(whole_of(&field), 3);
    let max_x = patch.positions.iter().fold(f32::MIN, |m, p| m.max(p.x));
    assert!((max_x - field.bounds().max.x).abs() < 1e-3, "stopped short at {max_x}");
}

#[test]
fn a_region_smaller_than_the_field_yields_fewer_vertices() {
    let field = flat_grid(5, 5);
    let corner = Aabb {
        min: Vec3::new(0.0, -100.0, 0.0),
        max: Vec3::new(10.0, 100.0, 10.0),
    };
    assert!(field.patch(corner, 1).positions.len() < field.patch(whole_of(&field), 1).positions.len());
}

#[test]
fn a_region_outside_the_field_yields_nothing() {
    let field = flat_grid(5, 5);
    let far = Aabb {
        min: Vec3::new(500.0, -1.0, 500.0),
        max: Vec3::new(600.0, 1.0, 600.0),
    };
    assert!(field.patch(far, 1).positions.is_empty());
}

#[test]
fn patch_indices_are_all_in_range() {
    let field = flat_grid(6, 4);
    let patch = field.patch(whole_of(&field), 2);
    let n = patch.positions.len() as u32;
    assert!(patch.indices.iter().all(|&i| i < n), "index out of range");
    assert_eq!(patch.indices.len() % 3, 0, "not whole triangles");
}

#[test]
fn patch_triangles_wind_counter_clockwise_seen_from_above() {
    // The renderer culls back faces with front_face(Ccw). Cuboids were once
    // wound the other way and every one of them lit inside-out while erroring
    // nowhere; terrain would fail the same way, over the whole map.
    let field = flat_grid(3, 3);
    let patch = field.patch(whole_of(&field), 1);

    for tri in patch.indices.chunks_exact(3) {
        let a = patch.positions[tri[0] as usize];
        let b = patch.positions[tri[1] as usize];
        let c = patch.positions[tri[2] as usize];
        let normal = (b - a).cross(c - a);
        assert!(normal.y > 0.0, "triangle {tri:?} faces downward (normal {normal:?})");
    }
}

fn flat_grid(nx: u32, nz: u32) -> Heightfield {
    Heightfield::new(
        vec![0; (nx * nz) as usize],
        [nx, nz],
        [(nx - 1) as f32 * 10.0, (nz - 1) as f32 * 10.0],
        [0.0, 100.0],
        Vec3::ZERO,
    )
    .unwrap()
}
