use glam::{Quat, Vec3};

use crate::scene_cuboid::distance_to_oriented_box;

const RIFLE: Vec3 = Vec3::new(0.45, 0.12, 0.05);
const ORIGIN: Vec3 = Vec3::ZERO;

const GRAB_RANGE: f32 = 0.15;

#[test]
fn a_point_inside_the_box_is_zero_away() {
    let d = distance_to_oriented_box(ORIGIN, Quat::IDENTITY, RIFLE, Vec3::new(0.2, 0.0, 0.0));
    assert_eq!(d, 0.0);
}

#[test]
fn distance_is_measured_to_the_surface_not_the_centre() {

    let d = distance_to_oriented_box(ORIGIN, Quat::IDENTITY, RIFLE, Vec3::new(0.0, 0.0, 0.20));
    assert!((d - 0.15).abs() < 1e-5, "expected 0.15, got {d}");
}

#[test]
fn the_box_turns_with_the_object() {

    let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let hand = Vec3::new(0.40, 0.0, 0.0);

    let d = distance_to_oriented_box(ORIGIN, turned, RIFLE, hand);
    assert!(d > GRAB_RANGE, "a hand 40 cm off the turned rifle must not be grabbable, got {d}");

    let d_flat = distance_to_oriented_box(ORIGIN, Quat::IDENTITY, RIFLE, hand);
    assert_eq!(d_flat, 0.0, "40 cm along a 45 cm half-length is inside the box");
}

#[test]
fn turning_the_box_does_not_change_a_point_that_turns_with_it() {

    let point = Vec3::new(0.6, 0.3, -0.2);
    let flat = distance_to_oriented_box(ORIGIN, Quat::IDENTITY, RIFLE, point);
    for turns in 1..8 {
        let q = Quat::from_rotation_y(turns as f32 * 0.4)
            * Quat::from_rotation_x(turns as f32 * 0.17);
        let d = distance_to_oriented_box(ORIGIN, q, RIFLE, q * point);
        assert!((d - flat).abs() < 1e-5, "turn {turns}: expected {flat}, got {d}");
    }
}

#[test]
fn the_box_moves_with_the_object() {
    let centre = Vec3::new(1.78, 0.99, 3.51);
    let d = distance_to_oriented_box(centre, Quat::IDENTITY, RIFLE, centre + Vec3::new(0.0, 0.32, 0.0));
    assert!((d - 0.20).abs() < 1e-5, "expected 0.20, got {d}");
}

#[test]
fn a_hand_on_the_grip_is_in_range_and_one_across_the_room_is_not() {

    let grip = Vec3::new(-0.26, -0.1, 0.0);
    let on_grip = distance_to_oriented_box(ORIGIN, Quat::IDENTITY, RIFLE, grip);
    assert!(on_grip <= GRAB_RANGE, "the authored grip must be reachable, got {on_grip}");

    let across_the_room = distance_to_oriented_box(ORIGIN, Quat::IDENTITY, RIFLE, Vec3::new(4.0, 1.0, 2.0));
    assert!(across_the_room > GRAB_RANGE);
}

#[test]
fn a_cuboid_loads_the_extent_its_scene_declares() {
    let declared: crate::scene::CuboidDef =
        serde_json::from_str(r#"{"position":[0,1,0],"half_size":[2.0,1.0,0.25]}"#)
            .expect("parses");
    assert_eq!(declared.half_size, Vec3::new(2.0, 1.0, 0.25));

    let misspelt: crate::scene::CuboidDef =
        serde_json::from_str(r#"{"position":[0,1,0],"size":[2.0,1.0,0.25]}"#).expect("parses");
    assert_eq!(
        misspelt.half_size,
        Vec3::splat(0.5),
        "`size` is not a field: it is dropped and the default stands. If this \
         assertion ever fails because an alias was added, delete it -- but do \
         not make `size` mean something different from `half_size`."
    );
}
