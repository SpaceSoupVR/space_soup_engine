//! Scatter resolution.
//!
//! The headline property is stability under edit: raising density, adding a
//! stroke or changing the palette must not relocate instances the author has
//! already hand-placed. Nothing about that failure is loud -- the forest simply
//! comes back different, and every override is now attached to the wrong tree.

use glam::Vec3;

use crate::scatter::{
    resolve, slot_count, FlatGround, Ground, ScatterKey, ScatterLayer, ScatterOverride,
    ScatterPrototype, ScatterStroke,
};

fn proto(mesh: &str) -> ScatterPrototype {
    ScatterPrototype {
        mesh: mesh.into(),
        weight: 1.0,
        scale_range: [1.0, 1.0],
        max_slope_deg: 90.0,
    }
}

fn stroke(id: u32, density: f32) -> ScatterStroke {
    ScatterStroke { id, center: [0.0, 0.0], radius: 10.0, density }
}

fn layer(density: f32) -> ScatterLayer {
    ScatterLayer {
        id: "trees".into(),
        name: "Trees".into(),
        seed: 12345,
        prototypes: vec![proto("pine.glb")],
        strokes: vec![stroke(1, density)],
        overrides: vec![],
    }
}

const FLAT: FlatGround = FlatGround(0.0);

#[test]
fn resolution_is_deterministic() {
    let l = layer(0.2);
    let a = resolve(&l, &FLAT);
    let b = resolve(&l, &FLAT);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.key, y.key);
        assert_eq!(x.position, y.position);
        assert_eq!(x.rotation, y.rotation);
    }
}

#[test]
fn a_different_seed_gives_a_different_forest() {
    let mut other = layer(0.2);
    other.seed = 999;
    assert_ne!(resolve(&layer(0.2), &FLAT)[0].position, resolve(&other, &FLAT)[0].position);
}

#[test]
fn raising_density_appends_and_does_not_move_what_was_there() {
    // THE property. Get this wrong and every hand-placed edit in the level
    // silently attaches to a different object.
    let sparse = resolve(&layer(0.2), &FLAT);
    let dense = resolve(&layer(0.6), &FLAT);

    assert!(dense.len() > sparse.len(), "denser should mean more");
    for (old, new) in sparse.iter().zip(dense.iter()) {
        assert_eq!(old.key, new.key, "slot identity moved");
        assert_eq!(old.position, new.position, "an existing instance was relocated");
    }
}

#[test]
fn lowering_density_truncates_from_the_end() {
    let dense = resolve(&layer(0.6), &FLAT);
    let sparse = resolve(&layer(0.2), &FLAT);
    for (kept, original) in sparse.iter().zip(dense.iter()) {
        assert_eq!(kept.position, original.position);
    }
}

#[test]
fn adding_a_second_stroke_leaves_the_first_alone() {
    let one = resolve(&layer(0.3), &FLAT);

    let mut two = layer(0.3);
    two.strokes.push(ScatterStroke { id: 2, center: [50.0, 50.0], radius: 8.0, density: 0.3 });
    let both = resolve(&two, &FLAT);

    for (a, b) in one.iter().zip(both.iter()) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.position, b.position);
    }
}

#[test]
fn instances_land_inside_the_stroke() {
    for inst in resolve(&layer(0.5), &FLAT) {
        let d = (inst.position.x.powi(2) + inst.position.z.powi(2)).sqrt();
        assert!(d <= 10.001, "instance at radius {d} escaped a radius-10 stroke");
    }
}

#[test]
fn instances_are_spread_rather_than_piled_in_the_middle() {
    // sqrt on the radius is what makes the disc uniform; without it everything
    // clusters at the centre and every stroke has a bald rim.
    let all = resolve(&layer(0.8), &FLAT);
    let outer = all
        .iter()
        .filter(|i| (i.position.x.powi(2) + i.position.z.powi(2)).sqrt() > 7.07)
        .count();
    // Beyond r/sqrt(2) is half the disc's area, so roughly half should be there.
    let fraction = outer as f32 / all.len() as f32;
    assert!((0.35..0.65).contains(&fraction), "outer half held {fraction:.2} of instances");
}

#[test]
fn slot_count_follows_area_and_density() {
    assert_eq!(slot_count(&ScatterStroke { id: 1, center: [0.0, 0.0], radius: 10.0, density: 0.0 }), 0);
    let a = slot_count(&stroke(1, 0.1));
    let b = slot_count(&stroke(1, 0.2));
    assert!(b > a);
}

// ------------------------------------------------------------------ overrides

#[test]
fn a_removed_instance_stays_removed_through_a_regeneration() {
    let l = layer(0.3);
    let before = resolve(&l, &FLAT);
    let victim = before[3].key;

    let mut edited = l.clone();
    edited.overrides.push(ScatterOverride::Removed { key: victim });
    let after = resolve(&edited, &FLAT);

    assert_eq!(after.len(), before.len() - 1);
    assert!(!after.iter().any(|i| i.key == victim));
}

#[test]
fn a_moved_instance_keeps_its_new_position() {
    let l = layer(0.3);
    let target = resolve(&l, &FLAT)[5].key;

    let mut edited = l.clone();
    edited.overrides.push(ScatterOverride::Transform {
        key: target,
        position: Some([100.0, 7.0, -100.0]),
        rotation: None,
        scale: None,
    });

    let moved = resolve(&edited, &FLAT).into_iter().find(|i| i.key == target).unwrap();
    assert_eq!(moved.position, Vec3::new(100.0, 7.0, -100.0));
}

#[test]
fn a_hand_edit_survives_a_density_change() {
    // The whole point: scatter, nudge one, then change your mind about density.
    let mut l = layer(0.2);
    let target = resolve(&l, &FLAT)[4].key;
    l.overrides.push(ScatterOverride::Transform {
        key: target,
        position: Some([42.0, 1.0, 42.0]),
        rotation: None,
        scale: None,
    });

    let mut denser = l.clone();
    denser.strokes[0].density = 0.9;

    let still_there = resolve(&denser, &FLAT).into_iter().find(|i| i.key == target).unwrap();
    assert_eq!(still_there.position, Vec3::new(42.0, 1.0, 42.0));
}

#[test]
fn an_override_can_swap_the_prototype() {
    let mut l = layer(0.3);
    l.prototypes.push(proto("rock.glb"));
    let target = resolve(&l, &FLAT)[2].key;

    l.overrides.push(ScatterOverride::Prototype { key: target, prototype: 1 });
    let swapped = resolve(&l, &FLAT).into_iter().find(|i| i.key == target).unwrap();
    assert_eq!(swapped.prototype, 1);
}

#[test]
fn an_override_naming_a_prototype_that_does_not_exist_is_ignored() {
    let mut l = layer(0.3);
    let target = resolve(&l, &FLAT)[1].key;
    l.overrides.push(ScatterOverride::Prototype { key: target, prototype: 99 });

    let kept = resolve(&l, &FLAT).into_iter().find(|i| i.key == target).unwrap();
    assert_eq!(kept.prototype, 0, "should fall back rather than index out of bounds");
}

// -------------------------------------------------------------------- palette

#[test]
fn weights_bias_the_palette() {
    let mut l = layer(2.0);
    l.prototypes = vec![
        ScatterPrototype { weight: 9.0, ..proto("common.glb") },
        ScatterPrototype { weight: 1.0, ..proto("rare.glb") },
    ];
    let all = resolve(&l, &FLAT);
    let rare = all.iter().filter(|i| i.prototype == 1).count() as f32 / all.len() as f32;
    assert!((0.02..0.20).contains(&rare), "rare prototype appeared {rare:.3} of the time");
}

#[test]
fn a_zero_weight_prototype_is_never_placed() {
    // Kept in the palette but not placed -- more useful than deleting it while
    // iterating on a look.
    let mut l = layer(2.0);
    l.prototypes = vec![proto("used.glb"), ScatterPrototype { weight: 0.0, ..proto("unused.glb") }];
    assert!(!resolve(&l, &FLAT).iter().any(|i| i.prototype == 1));
}

#[test]
fn an_empty_palette_scatters_nothing_rather_than_panicking() {
    let mut l = layer(0.3);
    l.prototypes.clear();
    assert!(resolve(&l, &FLAT).is_empty());
}

#[test]
fn scale_stays_inside_the_prototype_range() {
    let mut l = layer(0.5);
    l.prototypes[0].scale_range = [0.8, 1.4];
    for inst in resolve(&l, &FLAT) {
        assert!((0.8..=1.4).contains(&inst.scale), "scale {} out of range", inst.scale);
    }
}

// ---------------------------------------------------------------------- slope

/// Ground that is flat on one side of x=0 and a wall on the other.
struct HalfCliff;

impl Ground for HalfCliff {
    fn height_at(&self, x: f32, _z: f32) -> Option<f32> {
        Some(if x > 0.0 { x * 4.0 } else { 0.0 })
    }
}

#[test]
fn steep_ground_rejects_placements() {
    let mut l = layer(1.0);
    l.prototypes[0].max_slope_deg = 20.0;

    let all = resolve(&l, &HalfCliff);
    assert!(!all.is_empty(), "the flat half should still be planted");
    assert!(
        all.iter().all(|i| i.position.x <= 0.5),
        "something was planted on the cliff face"
    );
}

#[test]
fn a_generous_slope_limit_plants_the_cliff_too() {
    let mut l = layer(1.0);
    l.prototypes[0].max_slope_deg = 89.0;
    assert!(resolve(&l, &HalfCliff).iter().any(|i| i.position.x > 1.0));
}

#[test]
fn a_hand_placed_instance_on_a_cliff_is_left_alone() {
    // Slope rejection is for the generator's guesses, not for the author's
    // decisions. Re-rejecting a moved tree would delete deliberate work.
    let mut l = layer(1.0);
    l.prototypes[0].max_slope_deg = 20.0;
    let target = resolve(&l, &HalfCliff)[0].key;

    l.overrides.push(ScatterOverride::Transform {
        key: target,
        position: Some([50.0, 200.0, 0.0]),
        rotation: None,
        scale: None,
    });
    let kept = resolve(&l, &HalfCliff).into_iter().find(|i| i.key == target);
    assert!(kept.is_some(), "an author-placed instance was removed by slope rejection");
}

#[test]
fn ground_that_reports_nothing_places_nothing() {
    struct Void;
    impl Ground for Void {
        fn height_at(&self, _x: f32, _z: f32) -> Option<f32> { None }
    }
    assert!(resolve(&layer(0.5), &Void).is_empty());
}

#[test]
fn instances_sit_on_the_ground_they_are_given() {
    let all = resolve(&layer(0.4), &FlatGround(17.5));
    assert!(all.iter().all(|i| (i.position.y - 17.5).abs() < 1e-4));
}

// ----------------------------------------------------------------- the mixer

/// Reference values for the mixer, pinned so another language can be checked
/// against them.
///
/// These are the contract, not an implementation detail: the editor's preview
/// has to place instances exactly where the runtime will, and if these numbers
/// ever change then every scattered level in existence changes with them.
///
/// Confirmed independently against a Python implementation of the same steps,
/// which is the actual claim being made -- that this is reproducible from the
/// written-down algorithm rather than from Rust's particular arithmetic.
#[test]
fn the_mixer_matches_its_reference_values() {
    use crate::scatter::mix32;
    assert_eq!(mix32(0), 0x0000_0000);
    assert_eq!(mix32(1), 0x6889_90c0);
    assert_eq!(mix32(42), 0x1727_33c2);
    assert_eq!(mix32(12345), 0x912e_fcf7);
}

// -------------------------------------------------- scatter on real terrain

/// The payoff: trees sit on sculpted ground, and the steep parts stay bare.
#[test]
fn scatter_follows_a_real_heightfield() {
    use crate::terrain::{Heightfield, TerrainSource};

    // A 17x17 field over 64m: flat at 0, rising to a steep ridge along +x.
    let n = 17usize;
    let mut samples = Vec::with_capacity(n * n);
    for _iz in 0..n {
        for ix in 0..n {
            // Flat for the first half, then a sharp climb.
            let t = if ix < 8 { 0.0 } else { (ix - 8) as f32 / 8.0 };
            samples.push((t * u16::MAX as f32) as u16);
        }
    }
    let field = Heightfield::new(
        samples,
        [n as u32, n as u32],
        [64.0, 64.0],
        [0.0, 40.0],
        Vec3::new(-32.0, 0.0, -32.0),
    )
    .unwrap();

    let mut l = layer(0.4);
    l.strokes[0].radius = 30.0;
    l.prototypes[0].max_slope_deg = 25.0;

    let all = resolve(&l, &field);
    assert!(!all.is_empty(), "nothing was planted at all");

    // Every instance sits ON the ground rather than at y=0.
    for inst in &all {
        let expected = TerrainSource::height_at(&field, inst.position.x, inst.position.z).unwrap();
        assert!(
            (inst.position.y - expected).abs() < 1e-3,
            "instance floated: y={} but ground is {expected}",
            inst.position.y
        );
    }

    // And the steep half is bare, because the ridge exceeds 25 degrees.
    let on_flat = all.iter().filter(|i| i.position.x < -4.0).count();
    let on_ridge = all.iter().filter(|i| i.position.x > 8.0).count();
    assert!(on_flat > 0, "the flat half should be planted");
    assert_eq!(on_ridge, 0, "{on_ridge} instances were planted on the steep ridge");
}
