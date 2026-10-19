//! Loose bodies in the hands and in the water: a free grab's weight and its
//! throw, floating and sinking, a going-in told once and by what it brings,
//! the props that come back, and what a step costs as the bodies multiply.

use std::collections::HashMap;
use std::path::Path;

use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;

use crate::brush::{block_solid, BrushDef};
use crate::events::{Hand, InputFrame};
use crate::rig::PlayerRig;
use crate::rigid_physics::PhysicsWorld;
use crate::rigid_physics_grab::{HAND_FORCE, HOLD_BREAK};
use crate::runtime_test_support::{frame, one_player, PHYSX_TEST_LOCK};
use crate::scene::{GameObject, Scene};

const DT: f32 = 1.0 / 60.0;

fn ground() -> GameObject {
    GameObject {
        id: "ground".into(),
        brush: Some(BrushDef { solids: vec![block_solid([-40.0, -1.0, -40.0], [40.0, 0.0, 40.0], "m")], subtract: vec![] }),
        ..Default::default()
    }
}

/// A box of `half` weighing `mass`, resting on the ground at x.
fn body(id: &str, x: f32, half: Vec3, mass: f32) -> GameObject {
    let mut o = GameObject { id: id.into(), ..Default::default() };
    o.cuboid.position = Vec3::new(x, half.y, 0.0);
    o.cuboid.half_size = half;
    o.rigid_body = Some(serde_json::from_value(serde_json::json!({ "mode": "Dynamic", "shape": "Box", "mass": mass })).unwrap());
    o
}

const PEBBLE: (Vec3, f32) = (Vec3::new(0.022, 0.015, 0.02), 0.1);
const ROCK: (Vec3, f32) = (Vec3::new(0.095, 0.065, 0.085), 8.0);

fn world(objects: Vec<GameObject>, water: Option<serde_json::Value>) -> (PhysicsWorld, Scene) {
    let mut scene: Scene = serde_json::from_str(r#"{"name":"hold","objects":[]}"#).unwrap();
    scene.objects = objects;
    scene.water = water.into_iter().map(|w| serde_json::from_value(w).unwrap()).collect();
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    (physics, scene)
}

struct Hands {
    player: PlayerId,
    rigs: HashMap<PlayerId, PlayerRig>,
}

impl Hands {
    fn new(physics: &mut PhysicsWorld, at: Vec3) -> Self {
        let player = PlayerId::new();
        let mut me = Self { player, rigs: HashMap::from([(player, PlayerRig::new())]) };
        me.put(Hand::Left, at + Vec3::new(0.0, 5.0, 3.0));
        me.put(Hand::Right, at);
        physics.ensure_player(player);
        me
    }

    fn put(&mut self, hand: Hand, at: Vec3) {
        self.rigs.get_mut(&self.player).unwrap().set_hand_grip(hand, at, Quat::IDENTITY);
    }
}

/// Carry `id` in the right hand (and the left too, if `both`): up to 1.2 m
/// over half a second, held there a second, then swung along +x, the hand
/// speeding up evenly to `speed` over 0.3 s, and let go. Returns how far the
/// body hung from the hand while held still, the hand's speed and the body's
/// at the letting go, and the body's speed a tenth of a second after.
fn carry_and_throw(physics: &mut PhysicsWorld, scene: &mut Scene, id: &str, both: bool, speed: f32) -> (f32, f32, Vec3, Vec3) {
    let start = scene.find_object(id).unwrap().cuboid.position;
    let top = start + Vec3::Y * scene.find_object(id).unwrap().cuboid.half_size.y;
    let mut hands = Hands::new(physics, top);
    let side = Vec3::new(0.0, 0.0, 0.06);
    if both {
        hands.put(Hand::Left, top + side);
        hands.put(Hand::Right, top - side);
    }
    for _ in 0..3 {
        physics.step(DT, scene, &hands.rigs);
    }
    assert!(physics.grab_free(hands.player, id, Hand::Right, if both { top - side } else { top }, Quat::IDENTITY));
    if both {
        assert!(physics.grab_free(hands.player, id, Hand::Left, top + side, Quat::IDENTITY));
    }
    let mut hand = top;
    let place = |hands: &mut Hands, hand: Vec3| {
        if both {
            hands.put(Hand::Left, hand + side);
            hands.put(Hand::Right, hand - side);
        } else {
            hands.put(Hand::Right, hand);
        }
    };
    for k in 0..30 {
        hand = top + Vec3::Y * (1.2 - top.y) * ((k + 1) as f32 / 30.0);
        place(&mut hands, hand);
        physics.step(DT, scene, &hands.rigs);
    }
    for _ in 0..60 {
        physics.step(DT, scene, &hands.rigs);
    }
    let held = physics.pose_of(id).unwrap();
    let grip = physics.grabs.get(&(hands.player, id.to_string(), Hand::Right)).and_then(|g| g.local).unwrap();
    let hung = (held.0 + held.1 * grip).distance(if both { hand - side } else { hand });
    let steps = 18;
    let mut v = 0.0;
    for _ in 0..steps {
        v += speed / steps as f32;
        hand.x += v * DT;
        place(&mut hands, hand);
        physics.step(DT, scene, &hands.rigs);
    }
    let at_release = physics.velocity_of(id).unwrap();
    physics.release(hands.player, id, Hand::Right);
    physics.release(hands.player, id, Hand::Left);
    assert!(!physics.is_held(id));
    for _ in 0..6 {
        physics.step(DT, scene, &hands.rigs);
    }
    (hung, v, at_release, physics.velocity_of(id).unwrap())
}

/// A pebble goes where the hand goes and leaves it at the hand's speed: the
/// throw is the hand's.
#[test]
fn a_pebble_follows_the_hand_and_is_thrown_at_the_hands_speed() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut physics, mut scene) = world(vec![ground(), body("pebble", 0.0, PEBBLE.0, PEBBLE.1)], None);
    let (hung, hand, at_release, after) = carry_and_throw(&mut physics, &mut scene, "pebble", false, 9.0);
    eprintln!("pebble: hangs {:.1} mm from the hand; hand {hand:.2} m/s, pebble {:.2} m/s at release, {:.2} m/s after", hung * 1000.0, at_release.x, after.x);
    assert!(hung < 0.005, "a held pebble hangs {hung} m from the hand");
    assert!(at_release.x > 0.85 * hand && at_release.x < 1.1 * hand, "thrown at {} of the hand's {hand} m/s", at_release.x);
    assert!((after.x - at_release.x).abs() < 0.3 && after.y < at_release.y, "{at_release} -> {after}");
    let p = scene.find_object("pebble").unwrap().cuboid.position;
    assert!(p.x > 1.0 && p.y > 0.5, "{p}");
}

/// An 8 kg rock can be lifted and carried in one hand, hangs a little under
/// it, and cannot be flung like a pebble: the hand goes on and the rock
/// comes after at what a hand's strength gives it. Two hands give it twice.
#[test]
fn a_heavy_rock_is_carried_lags_a_quick_hand_and_cannot_be_flung() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let throw = |both: bool| {
        let (mut physics, mut scene) = world(vec![ground(), body("rock", 0.0, ROCK.0, ROCK.1)], None);
        carry_and_throw(&mut physics, &mut scene, "rock", both, 9.0)
    };
    let (hung, hand, one, _) = throw(false);
    let (hung_two, _, two, _) = throw(true);
    eprintln!(
        "8 kg rock: one hand hangs {:.1} mm, thrown at {:.2} of the hand's {hand:.1} m/s; two hands hang {:.1} mm, {:.2} m/s",
        hung * 1000.0, one.x, hung_two * 1000.0, two.x
    );
    assert!(hung < 0.02, "one hand cannot hold it: {hung} m from the hand");
    assert!(one.x < 0.75 * hand, "flung at {} m/s by a {hand} m/s hand", one.x);
    assert!(one.x <= HAND_FORCE / ROCK.1 * 0.3 + 0.2, "faster than a hand's strength allows: {}", one.x);
    assert!(one.x > 2.0, "it hardly moved: {}", one.x);
    assert!(two.x > one.x + 1.0, "two hands threw it no harder: {} against {}", two.x, one.x);
}

/// A thing heavier than a hand can lift stays on the ground, and a hand that
/// goes on without it has let go of it.
#[test]
fn what_is_too_heavy_stays_down_and_the_hand_that_leaves_it_lets_go() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let half = Vec3::splat(0.2);
    let (mut physics, mut scene) = world(vec![ground(), body("anvil", 0.0, half, 60.0)], None);
    let top = Vec3::new(0.0, 0.4, 0.0);
    let mut hands = Hands::new(&mut physics, top);
    for _ in 0..3 {
        physics.step(DT, &mut scene, &hands.rigs);
    }
    assert!(physics.grab_free(hands.player, "anvil", Hand::Right, top, Quat::IDENTITY));
    for k in 0..60 {
        hands.put(Hand::Right, top + Vec3::Y * 0.4 * (k as f32 / 60.0));
        physics.step(DT, &mut scene, &hands.rigs);
    }
    let y = scene.find_object("anvil").unwrap().cuboid.position.y;
    assert!(y < 0.21, "a 60 kg anvil was lifted to {y} by one hand");
    assert!(physics.is_held("anvil"), "0.4 m is not yet out of reach");
    for k in 0..30 {
        hands.put(Hand::Right, top + Vec3::Y * (0.4 + (HOLD_BREAK) * (k as f32 / 20.0)));
        physics.step(DT, &mut scene, &hands.rigs);
    }
    assert!(!physics.is_held("anvil"), "a hand a metre off still holds it");
}

/// THE GAME'S OWN PATH: a grab event for a loose body with no grip named
/// (what the headset sends for a rock) holds it by physics -- it follows the
/// hand, which an attachment never made a dynamic body do -- and the release
/// event lets it fall.
#[test]
fn the_default_grab_of_a_dynamic_body_carries_it_and_the_release_drops_it() {
    use crate::runtime::GameRuntime;
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("space_soup_engine_free_grab_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("scenes")).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"name":"t","version":"0.1.0","entry_scene":"t","scenes":["t"]}"#).unwrap();
    std::fs::write(
        dir.join("scenes/t.json"),
        r#"{ "name": "t", "objects": [
            { "id": "floor", "cuboid": { "position": [0, -0.5, 0], "half_size": [20, 0.5, 20] }, "rigid_body": { "mode": "Static" } },
            { "id": "rock", "cuboid": { "position": [0, 0.05, 0], "half_size": [0.05, 0.05, 0.05] }, "rigid_body": { "mode": "Dynamic", "mass": 1.0 } }
        ] }"#,
    )
    .unwrap();
    let mut rt = GameRuntime::load(&dir).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let player = PlayerId::new();
    let rig_at = |p: Vec3| {
        let mut rig = PlayerRig::new();
        rig.set_hand_grip(Hand::Right, p, Quat::IDENTITY);
        rig.set_hand_grip(Hand::Left, Vec3::new(3.0, 1.0, 3.0), Quat::IDENTITY);
        rig
    };
    let hand = Vec3::new(0.0, 0.1, 0.0);
    for _ in 0..3 {
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    let mut grab = InputFrame::default();
    grab.grabbed.push(("rock".to_string(), Hand::Right, String::new()));
    rt.update(DT, &one_player(player, frame(rig_at(hand), grab)));
    assert!(rt.rigid_physics.is_held("rock"), "the grab event did not take hold of it");
    assert!(!rt.attachments.is_attached("rock"), "a dynamic body is held by physics, not attached");
    let mut at = hand;
    for k in 0..45 {
        at = hand + Vec3::new(0.3, 1.1, 0.0) * ((k + 1) as f32 / 45.0);
        rt.update(DT, &one_player(player, frame(rig_at(at), InputFrame::default())));
    }
    for _ in 0..30 {
        rt.update(DT, &one_player(player, frame(rig_at(at), InputFrame::default())));
    }
    let carried = rt.scene().find_object("rock").unwrap().cuboid.position;
    assert!(carried.distance(at - Vec3::Y * 0.05) < 0.02, "the rock is at {carried}, the hand at {at}");
    let mut release = InputFrame::default();
    release.released.push(("rock".to_string(), Hand::Right));
    rt.update(DT, &one_player(player, frame(rig_at(at), release)));
    assert!(!rt.rigid_physics.is_held("rock"));
    for _ in 0..90 {
        rt.update(DT, &one_player(player, frame(rig_at(at), InputFrame::default())));
    }
    let dropped = rt.scene().find_object("rock").unwrap().cuboid.position;
    assert!(dropped.y < 0.1, "let go, it did not fall: {dropped}");
}

fn pond() -> serde_json::Value {
    serde_json::json!({ "height": 0.0, "bounds": [2.0, -20.0, 40.0, 20.0] })
}

/// Ground a metre and a half under a pond over x 2..40 (and dry at y = 0
/// short of it).
fn pond_floor() -> Vec<GameObject> {
    vec![
        GameObject {
            id: "bed".into(),
            brush: Some(BrushDef { solids: vec![block_solid([-40.0, -3.0, -40.0], [40.0, -1.5, 40.0], "m")], subtract: vec![] }),
            ..Default::default()
        },
        GameObject {
            id: "bank".into(),
            brush: Some(BrushDef { solids: vec![block_solid([-40.0, -1.5, -40.0], [2.0, 0.0, 40.0], "m")], subtract: vec![] }),
            ..Default::default()
        },
    ]
}

fn dropped(id: &str, x: f32, z: f32, y: f32, half: Vec3, mass: f32) -> GameObject {
    let mut o = body(id, x, half, mass);
    o.cuboid.position = Vec3::new(x, y, z);
    o
}

/// A rock sinks to the bed at the speed its drag gives it, and a crate
/// lighter than its water floats: level, as deep as its weight, and asleep in the
/// end -- costing nothing.
#[test]
fn a_rock_sinks_and_a_crate_floats() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let crate_half = Vec3::splat(0.15);
    let mut objects = pond_floor();
    objects.push(dropped("rock", 6.0, 0.0, 1.0, ROCK.0, ROCK.1));
    let mut c = dropped("crate", 10.0, 0.0, 1.0, crate_half, 5.0);
    c.cuboid.rotation = Quat::from_rotation_z(0.5);
    objects.push(c);
    let (mut physics, mut scene) = world(objects, Some(pond()));
    let rigs = HashMap::new();
    let mut fastest_sinking = 0.0f32;
    for k in 0..(60 * 12) {
        physics.step(DT, &mut scene, &rigs);
        let rock = scene.find_object("rock").unwrap().cuboid.position;
        if rock.y < -0.4 && rock.y > -1.2 {
            fastest_sinking = fastest_sinking.max(-physics.velocity_of("rock").unwrap().y);
        }
        if k == 60 * 4 {
            assert!(rock.y < -1.3, "four seconds on the rock is at {rock}");
        }
    }
    let rock = scene.find_object("rock").unwrap().cuboid.position;
    let floating = scene.find_object("crate").unwrap().cuboid.clone();
    eprintln!("rock sank at up to {fastest_sinking:.2} m/s to {rock}; crate floats at y {:.3}", floating.position.y);
    assert!((rock.y - (-1.5 + ROCK.0.y)).abs() < 0.03, "the rock rests on the bed: {rock}");
    assert!(fastest_sinking > 0.5 && fastest_sinking < 3.2, "sinking at {fastest_sinking} m/s");
    assert!((floating.position.y - 0.094).abs() < 0.02, "the crate floats at {}", floating.position.y);
    let up = floating.rotation * Vec3::Y;
    assert!(up.abs().max_element() > 0.985, "it does not float level: {up}");
    assert_eq!(physics.is_asleep("crate"), Some(true), "afloat and still, it should sleep");
    assert_eq!(physics.is_asleep("rock"), Some(true));
    let stats = physics.step_stats();
    assert_eq!((stats.bodies, stats.awake), (2, 0), "{stats:?}");
}

/// A going-in is told with the energy it came with, and a body that comes
/// out and goes in again -- thrown up by its own buoyancy, bouncing off a
/// shallow bed -- is told again only if it brings a real share of that.
#[test]
fn a_body_is_told_of_once_however_it_bobs_and_bounces() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut objects = pond_floor();
    objects.push(dropped("crate", 10.0, 0.0, 3.0, Vec3::splat(0.15), 4.0));
    objects.push(GameObject {
        id: "shelf".into(),
        brush: Some(BrushDef { solids: vec![block_solid([18.0, -1.5, -2.0], [22.0, -0.2, 2.0], "m")], subtract: vec![] }),
        ..Default::default()
    });
    let mut bouncy = dropped("rock", 20.0, 0.0, 2.0, ROCK.0, ROCK.1);
    bouncy.rigid_body = Some(serde_json::from_value(serde_json::json!({ "mode": "Dynamic", "mass": 8.0, "restitution": 0.6 })).unwrap());
    objects.push(bouncy);
    let (mut physics, mut scene) = world(objects, Some(pond()));
    let rigs = HashMap::new();
    let mut entries = Vec::new();
    let mut crossings = HashMap::<String, u32>::new();
    let mut above = HashMap::<String, bool>::new();
    for _ in 0..(60 * 8) {
        physics.step(DT, &mut scene, &rigs);
        entries.extend(physics.take_water_entries());
        for id in ["crate", "rock"] {
            let o = scene.find_object(id).unwrap();
            let under = o.cuboid.position.y - o.cuboid.half_size.y;
            let was = above.insert(id.to_string(), under > 0.0).unwrap_or(true);
            if was && under <= 0.0 {
                *crossings.entry(id.to_string()).or_default() += 1;
            }
        }
    }
    for e in &entries {
        eprintln!("entry {} at {:.2} m/s, {:.1} J", e.id, e.velocity.length(), e.energy());
    }
    eprintln!("undersides crossed the surface downward: {crossings:?}");
    for id in ["crate", "rock"] {
        let told: Vec<_> = entries.iter().filter(|e| e.id == id).collect();
        assert!(!told.is_empty(), "{id} was never told of");
        assert!(told.len() <= 2, "{id} splashed {} times", told.len());
        for pair in told.windows(2) {
            assert!(pair[1].energy() >= 0.05 * pair[0].energy(), "{id}: a {} J plop after a {} J splash", pair[1].energy(), pair[0].energy());
        }
    }
    let first = entries.iter().find(|e| e.id == "crate").unwrap();
    assert!((first.velocity.length() - (2.0f32 * 9.81 * 2.85).sqrt()).abs() < 0.4, "{first:?}");
    assert!((first.energy() - 0.5 * 4.0 * 2.0 * 9.81 * 2.85).abs() < 12.0, "{} J", first.energy());
}

/// The rule itself, without physics: a skip that keeps most of its speed
/// splashes again, a bob that keeps a tenth of it does not, and a small plop
/// long after a big splash does.
#[test]
fn a_going_in_again_is_judged_by_the_energy_it_brings() {
    use crate::water::WaterState;
    let half = Vec3::splat(0.05);
    let down = |speed: f32| Vec3::new(3.0, -speed, 0.0);
    let again = |v0: f32, gap: f64, v1: f32| {
        let mut s = WaterState::default();
        assert!(s.see(Some(0.0), Vec3::Y * 0.5, half, down(v0), 1.0, Some(0.0)).is_none());
        let first = s.see(Some(0.1), Vec3::Y * 0.04, half, down(v0), 1.0, Some(0.0));
        assert!(first.is_some(), "the first going-in is always told");
        assert!(s.see(Some(0.2), Vec3::Y * 0.3, half, -down(v0), 1.0, Some(0.0)).is_none());
        s.see(Some(0.1 + gap), Vec3::Y * 0.04, half, down(v1), 1.0, Some(0.0)).is_some()
    };
    assert!(again(5.0, 0.3, 4.5), "a skip at nine tenths of the speed");
    assert!(!again(5.0, 0.3, 1.0), "a bob at a fifth of the speed, at once");
    assert!(again(5.0, 3.0, 1.0), "the same plop three seconds on");
    assert!(again(5.0, 0.12, 5.0));
    let mut s = WaterState::default();
    s.see(None, Vec3::Y * 0.5, half, down(5.0), 1.0, Some(0.0));
    assert!(s.see(None, Vec3::Y * 0.04, half, down(5.0), 1.0, Some(0.0)).is_some());
    s.see(None, Vec3::Y * 0.3, half, -down(5.0), 1.0, Some(0.0));
    assert!(s.see(None, Vec3::Y * 0.04, half, down(1.0), 1.0, Some(0.0)).is_some());
}

/// When more go in at once than are told of, the least energetic are the
/// ones left out, and the order of the rest is kept.
#[test]
fn the_cap_drops_the_least_energetic_first() {
    use crate::water::{keep_most_energetic, WaterEntry};
    let e = |id: &str, mass: f32, speed: f32| WaterEntry { id: id.into(), position: Vec3::ZERO, velocity: Vec3::NEG_Y * speed, radius: 0.1, mass };
    let mut all = vec![e("pebble_a", 0.1, 3.0), e("boulder", 80.0, 4.0), e("pebble_b", 0.1, 6.0), e("rock", 8.0, 5.0), e("pebble_c", 0.1, 3.0)];
    keep_most_energetic(&mut all, 3);
    assert_eq!(all.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["boulder", "pebble_b", "rock"]);
    let mut ties = vec![e("a", 1.0, 2.0), e("b", 1.0, 2.0), e("c", 1.0, 2.0)];
    keep_most_energetic(&mut ties, 2);
    assert_eq!(ties.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    keep_most_energetic(&mut ties, 5);
    assert_eq!(ties.len(), 2);
    keep_most_energetic(&mut ties, 0);
    assert!(ties.is_empty());
}

/// A prop comes back where it started when it has rested under water as long
/// as its author gave it, when it is asked for -- and not out of a hand.
#[test]
fn a_sunk_prop_comes_back_and_so_does_every_prop_when_asked() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut objects = pond_floor();
    let start = Vec3::new(1.0, ROCK.0.y, 0.0);
    let mut rock = dropped("rock", start.x, start.z, start.y, ROCK.0, ROCK.1);
    rock.rigid_body = Some(serde_json::from_value(serde_json::json!({ "mode": "Dynamic", "mass": 8.0, "respawn_sunk": 3.0 })).unwrap());
    objects.push(rock);
    objects.push(dropped("keeper", 1.0, 2.0, ROCK.0.y, ROCK.0, ROCK.1));
    objects.push(dropped("pebble", 1.0, -2.0, PEBBLE.0.y, PEBBLE.0, PEBBLE.1));
    let (mut physics, mut scene) = world(objects, Some(pond()));
    let rigs = HashMap::new();
    let at = |scene: &Scene, id: &str| scene.find_object(id).unwrap().cuboid.position;
    physics.apply_impulse("rock", Vec3::new(6.0, 4.0, 0.0));
    physics.apply_impulse("keeper", Vec3::new(6.0, 4.0, 0.0));
    let mut on_the_bed = None;
    let mut back_at = None;
    for k in 0..(60 * 14) {
        physics.step(DT, &mut scene, &rigs);
        let p = at(&scene, "rock");
        if on_the_bed.is_none() && p.y < -1.3 {
            on_the_bed = Some(k);
        }
        if on_the_bed.is_some() && back_at.is_none() && p.distance(start) < 0.05 {
            back_at = Some(k);
        }
    }
    let (sank, back) = (on_the_bed.expect("the rock never reached the bed"), back_at.expect("the sunk rock never came back"));
    let waited = (back - sank) as f32 * DT;
    eprintln!("sunk at {:.1} s, back {waited:.1} s later", sank as f32 * DT);
    assert!((3.0..5.0).contains(&waited), "it came back {waited} s after it sank, authored 3");
    assert!(at(&scene, "keeper").y < -1.3, "{}", at(&scene, "keeper"));
    physics.apply_impulse("pebble", Vec3::new(-3.0, 3.0, 0.0));
    for _ in 0..30 {
        physics.step(DT, &mut scene, &rigs);
    }
    assert!(at(&scene, "pebble").distance(Vec3::new(1.0, PEBBLE.0.y, -2.0)) > 0.5);
    physics.respawn_props();
    physics.step(DT, &mut scene, &rigs);
    assert_eq!(physics.step_stats().respawned, 3);
    for (id, home) in [("keeper", Vec3::new(1.0, ROCK.0.y, 2.0)), ("pebble", Vec3::new(1.0, PEBBLE.0.y, -2.0))] {
        assert!(at(&scene, id).distance(home) < 0.02, "{id} is at {}", at(&scene, id));
    }
    let mut hands = Hands::new(&mut physics, Vec3::new(1.0, 0.03, -2.0));
    for _ in 0..3 {
        physics.step(DT, &mut scene, &hands.rigs);
    }
    assert!(physics.grab_free(hands.player, "pebble", Hand::Right, Vec3::new(1.0, 0.03, -2.0), Quat::IDENTITY));
    for k in 0..30 {
        hands.put(Hand::Right, Vec3::new(1.0, 0.03 + k as f32 * 0.03, -2.0));
        physics.step(DT, &mut scene, &hands.rigs);
    }
    physics.respawn_props();
    physics.step(DT, &mut scene, &hands.rigs);
    assert!(at(&scene, "pebble").y > 0.6, "taken out of the hand: {}", at(&scene, "pebble"));
}

/// ROCKS INTO A POND, 1 / 16 / 128 AT ONCE: what a step costs while they
/// fall, while they go in, and once they lie on the bed -- and how many
/// splashes a step tells of, against how many it saw.
///
///     cargo test --release --lib rocks_into_a_pond -- --nocapture
#[test]
fn rocks_into_a_pond_cost_what_is_awake_and_tell_of_no_more_than_the_cap() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut table = Vec::new();
    for count in [1usize, 16, 128] {
        let mut objects = pond_floor();
        let side = (count as f32).sqrt().ceil() as usize;
        for i in 0..count {
            let (ix, iz) = (i % side, i / side);
            let (half, mass) = [PEBBLE, (Vec3::new(0.05, 0.035, 0.045), 1.0), ROCK][i % 3];
            objects.push(dropped(&format!("rock_{i}"), 8.0 + ix as f32 * 0.4, -2.0 + iz as f32 * 0.4, 2.0, half, mass));
        }
        let (mut physics, mut scene) = world(objects, Some(pond()));
        let rigs = HashMap::new();
        let (mut falling, mut entering, mut resting) = (Vec::new(), Vec::new(), Vec::new());
        let (mut seen, mut told, mut most_told, mut most_seen) = (0usize, 0usize, 0usize, 0usize);
        let mut awake_at_end = 0;
        for k in 0..(60 * 20) {
            let started = std::time::Instant::now();
            physics.step(DT, &mut scene, &rigs);
            let took = started.elapsed().as_secs_f64() * 1e6;
            let stats = physics.step_stats();
            seen += stats.entries_seen;
            told += stats.entries_told;
            most_seen = most_seen.max(stats.entries_seen);
            most_told = most_told.max(stats.entries_told);
            physics.take_water_entries();
            match k {
                0..=30 => falling.push(took),
                31..=90 => entering.push(took),
                1080.. => resting.push(took),
                _ => {}
            }
            awake_at_end = stats.awake;
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let worst = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
        table.push((count, mean(&falling), mean(&entering), worst(&entering), mean(&resting), seen, told, most_seen, most_told, awake_at_end));
        assert!(most_told <= crate::water::MAX_ENTRIES_A_STEP, "{most_told} told of in one step");
        assert_eq!(awake_at_end, 0, "{count} rocks: {awake_at_end} still awake after 20 s");
        assert!(seen >= count, "{seen} entries seen for {count} rocks");
        if count <= crate::water::MAX_ENTRIES_A_STEP {
            assert_eq!(told, seen, "under the cap, every going-in is told of");
        }
    }
    eprintln!("rocks | step us: falling | going in (mean, worst) | at rest | entries seen | told | most seen a step | most told a step");
    for (count, falling, entering, worst, resting, seen, told, most_seen, most_told, _) in &table {
        eprintln!("{count:5} | {falling:8.1} | {entering:8.1} {worst:8.1} | {resting:7.2} | {seen:5} | {told:5} | {most_seen:4} | {most_told:4}");
    }
    let (one, many) = (table[0].4, table[2].4);
    assert!(many < one + 60.0, "128 sleeping rocks cost {many:.1} us a step against {one:.1} for one");
}

/// THE SHIPPED PILES, in the real level: test_room's fourteen props -- by
/// the sea and by the pond, three pebbles, two rocks, a boulder and a crate
/// each -- settle where they were put and go to sleep; then the pond's
/// boulder, thrown out over the water, is told of once with the energy it
/// brought, sinks, and is back on the bank some seconds after it settled.
#[test]
fn the_shipped_rocks_rest_where_they_lie_and_one_thrown_in_the_pond_comes_back() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let game = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
    let Ok(mut scene) = Scene::load(&game.join("scenes/test_room.json")) else {
        eprintln!("skipping: no test_room");
        return;
    };
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, game);
    let props: Vec<(String, Vec3, f32)> = scene
        .objects
        .iter()
        .filter(|o| o.door.is_none() && o.rigid_body.as_ref().is_some_and(|b| b.mode == crate::scene_physics::BodyMode::Dynamic))
        .map(|o| (o.id.clone(), o.cuboid.position, o.rigid_body.as_ref().unwrap().mass.unwrap_or(0.0)))
        .collect();
    assert_eq!(props.len(), 14, "two piles of seven: {:?}", props.iter().map(|p| &p.0).collect::<Vec<_>>());
    for site in ["beach", "pond"] {
        let masses: Vec<f32> = props.iter().filter(|p| p.0.starts_with(site)).map(|p| p.2).collect();
        assert!(masses.iter().any(|m| *m <= 0.1) && masses.iter().any(|m| *m == 1.0) && masses.iter().any(|m| *m >= 8.0), "{site}: {masses:?}");
    }
    let rigs = HashMap::new();
    for _ in 0..(60 * 6) {
        physics.step(DT, &mut scene, &rigs);
    }
    let at = |scene: &Scene, id: &str| scene.find_object(id).unwrap().cuboid.position;
    for (id, home, _) in &props {
        let p = at(&scene, id);
        eprintln!("{id}: put at {home}, rests at {p}, {}", if physics.is_asleep(id) == Some(true) { "asleep" } else { "AWAKE" });
        assert!(p.distance(*home) < 0.25, "{id} did not stay where it was put: {home} -> {p}");
        assert_eq!(physics.is_asleep(id), Some(true), "{id} never came to rest");
    }
    assert!(physics.step_stats().awake <= 3, "{:?}", physics.step_stats());
    assert!(physics.take_water_entries().is_empty(), "a prop was put in the water");
    let home = at(&scene, "pond_boulder");
    physics.apply_impulse("pond_boulder", Vec3::new(-5.0, 4.0, -1.0));
    let (mut told, mut sank, mut back) = (Vec::new(), None, None);
    for k in 0..(60 * 20) {
        physics.step(DT, &mut scene, &rigs);
        told.extend(physics.take_water_entries());
        let p = at(&scene, "pond_boulder");
        if sank.is_none() && !told.is_empty() && physics.is_asleep("pond_boulder") == Some(true) {
            sank = Some((k, p));
        }
        if sank.is_some() && back.is_none() && p.distance(home) < 0.1 {
            back = Some(k);
        }
    }
    assert_eq!(told.len(), 1, "told of {} goings-in", told.len());
    let e = &told[0];
    eprintln!("the boulder went in at {:.1} m/s with {:.0} J, at {:?}", e.velocity.length(), e.energy(), e.position);
    assert!((e.mass - 8.0).abs() < 1e-3 && (20.0..200.0).contains(&e.energy()), "{e:?}");
    let ((sank_at, bed), back_at) = (sank.expect("the boulder never settled in the pond"), back.expect("the sunk boulder never came back"));
    eprintln!("settled at {bed} after {:.1} s; back on the bank {:.1} s later", sank_at as f32 * DT, (back_at - sank_at) as f32 * DT);
    assert!(bed.y < -0.368 - 0.065, "it settled above the water: {bed}");
}

/// THE SHIPPED GAME, BY THE EVENTS THE HEADSET SENDS (`quest_app::part_pull`):
/// a grip pressed at the pond's fist-sized rock holds it; carried to above the
/// pond and let go, it falls in and the world tells of one going-in with the
/// rock's own mass; the menu button's press puts it back on the bank. Then
/// the brick hall's door: a grip pressed at its handle, sent as the grab the
/// headset sends, draws the bolt and the hand swings it open.
#[test]
fn the_shipped_game_by_the_headsets_events_a_rock_into_the_pond_and_a_door_by_its_handle() {
    use crate::runtime::GameRuntime;
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let game = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
    let Ok(mut rt) = GameRuntime::load(game) else {
        eprintln!("skipping: the shipped game did not load");
        return;
    };
    if rt.scene().find_object("pond_rock_1").is_none() {
        eprintln!("skipping: not test_room");
        return;
    }
    let player = PlayerId::new();
    let rig_at = |right: Vec3| {
        let mut rig = PlayerRig::new();
        rig.set_head(right + Vec3::new(-0.45, 0.5, 0.0), Quat::IDENTITY);
        rig.set_hand_grip(Hand::Right, right, Quat::IDENTITY);
        rig.set_hand_grip(Hand::Left, right + Vec3::new(-0.5, -0.2, 0.4), Quat::IDENTITY);
        rig
    };
    let pos = |rt: &GameRuntime, id: &str| rt.scene().find_object(id).unwrap().cuboid.position;
    let home = pos(&rt, "pond_rock_1");
    let mut hand = home + Vec3::new(0.0, 0.06, 0.0);
    for _ in 0..120 {
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    let rest = pos(&rt, "pond_rock_1");
    hand = rest + Vec3::new(0.0, 0.05, 0.0);
    for _ in 0..10 {
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    let mut grab = InputFrame::default();
    grab.grabbed.push(("pond_rock_1".to_string(), Hand::Right, String::new()));
    rt.update(DT, &one_player(player, frame(rig_at(hand), grab)));
    assert!(rt.rigid_physics.is_held("pond_rock_1"), "the grip did not take the rock");
    let over = Vec3::new(-23.0, 1.0, -20.5);
    let from = hand;
    for k in 0..120 {
        hand = from.lerp(over, (k + 1) as f32 / 120.0);
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    let carried = pos(&rt, "pond_rock_1");
    assert!(carried.distance(hand) < 0.12, "the rock is at {carried}, the hand at {hand}");
    let mut release = InputFrame::default();
    release.released.push(("pond_rock_1".to_string(), Hand::Right));
    rt.update(DT, &one_player(player, frame(rig_at(hand), release)));
    let mut told: Vec<(u64, f32, f32)> = Vec::new();
    for _ in 0..120 {
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
        for e in rt.water_entries() {
            if !told.iter().any(|t| t.0 == e.serial) {
                told.push((e.serial, e.entry.mass, e.entry.energy()));
            }
        }
    }
    eprintln!("dropped from {:.2} m over the pond: {told:?}; the rock lies at {}", hand.y + 0.368, pos(&rt, "pond_rock_1"));
    assert_eq!(told.len(), 1, "told of {told:?}");
    assert!((told[0].1 - 1.0).abs() < 1e-3 && (5.0..30.0).contains(&told[0].2), "{told:?}");
    assert!(pos(&rt, "pond_rock_1").y < -0.4);
    let mut press = InputFrame::default();
    press.button_presses.push(crate::events::ButtonPress::new(crate::events::RESPAWN_PROPS_BUTTON, None, Hand::Left));
    rt.update(DT, &one_player(player, frame(rig_at(hand), press)));
    rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    assert!(pos(&rt, "pond_rock_1").distance(home) < 0.02, "not put back: {}", pos(&rt, "pond_rock_1"));

    let lever = pos(&rt, "brick_door_lever");
    let mut hand = lever + Vec3::new(-0.08, 0.0, 0.0);
    let stood = hand + Vec3::new(-0.45, 0.5, 0.0);
    let rig_at = |right: Vec3| {
        let mut rig = rig_at(right);
        rig.set_head(stood, Quat::IDENTITY);
        rig.set_hand_grip(Hand::Left, stood + Vec3::new(-0.2, -0.6, 0.0), Quat::IDENTITY);
        rig
    };
    for _ in 0..20 {
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    assert_eq!(rt.rigid_physics.door_bolted("brick_door_north"), Some(false), "a hand at the handle did not draw the bolt");
    let mut grab = InputFrame::default();
    grab.grabbed.push(("brick_door_north".to_string(), Hand::Right, "handle".to_string()));
    rt.update(DT, &one_player(player, frame(rig_at(hand), grab)));
    assert!(rt.rigid_physics.is_held("brick_door_north"), "the grip did not take the handle");
    let geom = crate::door::DoorGeom::of(rt.scene().find_object("brick_door_north").unwrap()).unwrap();
    let local = geom.closed_rot.inverse() * (hand - geom.closed_pos);
    for k in 0..60 {
        let (p, r) = geom.pose_at(-70f32.to_radians() * (k + 1) as f32 / 60.0);
        hand = p + r * local;
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    for _ in 0..30 {
        rt.update(DT, &one_player(player, frame(rig_at(hand), InputFrame::default())));
    }
    let open = rt.rigid_physics.door_angle("brick_door_north").unwrap().to_degrees();
    assert!((-75.0..-62.0).contains(&open), "the door stands at {open:.1} deg, the hand at 70");
    let mut release = InputFrame::default();
    release.released.push(("brick_door_north".to_string(), Hand::Right));
    rt.update(DT, &one_player(player, frame(rig_at(hand), release)));
    assert!(!rt.rigid_physics.is_held("brick_door_north"));
}
