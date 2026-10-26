//! Ragdolls, headless: a dropped body settles without coming apart and keeps to its joints'
//! limits; one thrown at a wall at 40 m/s stays on this side of it; limbs tear off by hand or by
//! their break force; a hand drags a body by a spring and lets go of one snagged on a wall
//! rather than pulling it through; a frozen corpse stays put; and a scene rebuilt with ragdolls
//! in it lets them go cleanly.

use std::collections::HashMap;
use std::path::Path;

use glam::{Quat, Vec3};

use crate::brush::{block_solid, BrushDef};
use crate::rigid_physics::PhysicsWorld;
use crate::rigid_physics_ragdoll::{ragdoll_bone_poses, RagdollBodyDef, RagdollDef, RagdollShape, RagdollStart, MONSTER_GRIP, PLAYER_GRIP};
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::{GameObject, Scene};

const DT: f32 = 1.0 / 60.0;

fn block(id: &str, min: [f64; 3], max: [f64; 3]) -> GameObject {
    GameObject { id: id.into(), brush: Some(BrushDef { solids: vec![block_solid(min, max, "m")], subtract: vec![] }), ..Default::default() }
}

fn world(objects: Vec<GameObject>) -> (PhysicsWorld, Scene) {
    let mut scene: Scene = serde_json::from_str(r#"{"name":"ragdoll","objects":[]}"#).unwrap();
    scene.objects = objects;
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    (physics, scene)
}

fn step(p: &mut PhysicsWorld, s: &mut Scene) {
    p.step(DT, s, &HashMap::new());
}

/// A Gorilla Tag-style body, as Unity's `LeglessTorso` spec: torso, head, and each arm in
/// three, 17.1 kg. In T-pose; bones at the joints.
fn body() -> RagdollDef {
    let y_up = Quat::from_rotation_arc(Vec3::X, Vec3::Y).to_array();
    let id = Quat::IDENTITY.to_array();
    let limb = |bone: &str, parent: usize, rest: [f32; 3], side: f32, radius: f32, half: f32, mass: f32, swing: f32, twist: f32| RagdollBodyDef {
        bone: bone.into(),
        parent: Some(parent),
        shape: RagdollShape::Capsule { radius, half_height: half },
        shape_pos: [side * (half + radius), 0.0, 0.0],
        shape_rot: id,
        mass,
        rest_pos: rest,
        rest_rot: id,
        axis: [side, 0.0, 0.0],
        swing_deg: swing,
        twist_deg: twist,
        break_force: 0.0,
        break_torque: 0.0,
    };
    let mut bodies = vec![
        RagdollBodyDef {
            bone: "torso".into(),
            parent: None,
            shape: RagdollShape::Capsule { radius: 0.16, half_height: 0.18 },
            shape_pos: [0.0, 0.2, 0.0],
            shape_rot: y_up,
            mass: 8.0,
            rest_pos: [0.0; 3],
            rest_rot: id,
            axis: [0.0, 1.0, 0.0],
            swing_deg: 0.0,
            twist_deg: 0.0,
            break_force: 0.0,
            break_torque: 0.0,
        },
        RagdollBodyDef {
            bone: "head".into(),
            parent: Some(0),
            shape: RagdollShape::Box { half: [0.12, 0.12, 0.12] },
            shape_pos: [0.0, 0.13, 0.0],
            shape_rot: id,
            mass: 2.5,
            rest_pos: [0.0, 0.56, 0.0],
            rest_rot: id,
            axis: [0.0, 1.0, 0.0],
            swing_deg: 40.0,
            twist_deg: 25.0,
            break_force: 0.0,
            break_torque: 0.0,
        },
    ];
    for side in [-1.0f32, 1.0] {
        let base = bodies.len();
        bodies.push(limb("upper_arm", 0, [side * 0.2, 0.42, 0.0], side, 0.05, 0.09, 1.6, 70.0, 25.0));
        bodies.push(limb("forearm", base, [side * 0.28, 0.0, 0.0], side, 0.045, 0.08, 1.1, 80.0, 15.0));
        bodies.push(limb("hand", base + 1, [side * 0.25, 0.0, 0.0], side, 0.04, 0.03, 0.6, 45.0, 20.0));
    }
    RagdollDef { bodies }
}

/// Every body's pose with the root at `root`, the rest of the body at rest.
fn at_rest(def: &RagdollDef, root: Vec3, turn: Quat) -> Vec<(Vec3, Quat)> {
    let mut poses: Vec<(Vec3, Quat)> = Vec::new();
    for b in &def.bodies {
        poses.push(match b.parent {
            None => (root, turn),
            Some(p) => {
                let (pp, pr) = poses[p];
                (pp + pr * Vec3::from(b.rest_pos), pr * Quat::from_array(b.rest_rot))
            }
        });
    }
    poses
}

fn start(def: &RagdollDef, root: Vec3, velocity: Vec3) -> RagdollStart {
    let n = def.bodies.len();
    RagdollStart { poses: at_rest(def, root, Quat::IDENTITY), linear: vec![velocity; n], angular: vec![Vec3::ZERO; n] }
}

/// How far each limb's joint has come apart: its bone from where its parent says it should be.
fn worst_gap(def: &RagdollDef, poses: &[(Vec3, Quat)]) -> f32 {
    def.bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.parent.map(|p| (poses[p].0 + poses[p].1 * Vec3::from(b.rest_pos)).distance(poses[i].0)))
        .fold(0.0, f32::max)
}

/// How far each limb has swung from its rest direction relative to its parent, degrees.
fn swings(def: &RagdollDef, poses: &[(Vec3, Quat)]) -> Vec<(usize, f32)> {
    def.bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let p = b.parent?;
            let axis = Vec3::from(b.axis);
            let rest = poses[p].1 * Quat::from_array(b.rest_rot) * axis;
            let now = poses[i].1 * axis;
            Some((i, rest.angle_between(now).to_degrees()))
        })
        .collect()
}

#[test]
fn a_dropped_body_settles_in_one_piece_within_its_limits() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0])]);
    let def = body();
    p.spawn_ragdoll("corpse", &def, &start(&def, Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.5, 0.0, 0.5))).unwrap();
    let (mut gap, mut lowest) = (0.0f32, f32::MAX);
    let mut asleep_at = None;
    for k in 0..(8.0 / DT) as usize {
        step(&mut p, &mut s);
        let poses = p.ragdoll_poses("corpse").unwrap();
        assert!(poses.iter().all(|(t, r)| t.is_finite() && r.is_finite()), "a body went to NaN");
        let g = worst_gap(&def, &poses);
        assert!(worst_gap(&def, &ragdoll_bone_poses(&def, &poses)) < 1e-4, "the posed skeleton stretched");
        if std::env::var_os("GAP").is_some() && g > 0.02 {
            let per: Vec<String> = def.bodies.iter().enumerate().filter_map(|(i, b)| b.parent.map(|pa| format!("{}:{:.0}", i, (poses[pa].0 + poses[pa].1 * Vec3::from(b.rest_pos)).distance(poses[i].0) * 1000.0))).collect();
            eprintln!("  t {:.2}: per joint mm {}", k as f32 * DT, per.join(" "));
        }
        gap = gap.max(g);
        lowest = lowest.min(poses.iter().map(|(t, _)| t.y).fold(f32::MAX, f32::min));
        if asleep_at.is_none() && !p.ragdoll_awake("corpse") {
            asleep_at = Some(k as f32 * DT);
        }
    }
    let poses = p.ragdoll_poses("corpse").unwrap();
    let worst_swing = swings(&def, &poses).into_iter().map(|(i, a)| a - def.bodies[i].swing_deg).fold(f32::MIN, f32::max);
    eprintln!("dropped: joints apart at most {:.1} mm, lowest bone {lowest:.3} m, asleep after {asleep_at:?} s, worst swing past its limit {worst_swing:.1} deg", gap * 1000.0);
    let rest_gap = worst_gap(&def, &poses);
    eprintln!("at rest: joints apart at most {:.1} mm", rest_gap * 1000.0);
    assert!(gap < 0.035, "a joint came apart by {:.0} mm at impact", gap * 1000.0);
    assert!(rest_gap < 0.005, "a joint {:.1} mm apart at rest", rest_gap * 1000.0);
    assert!(lowest > -0.02, "a bone went through the floor to {lowest:.3}");
    assert!(asleep_at.is_some(), "it never settled");
    assert!(worst_swing < 8.0, "a limb {worst_swing:.1} deg past its swing limit");
}

#[test]
fn a_body_thrown_at_a_wall_at_40_m_s_stays_this_side_of_it() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0]), block("wall", [3.0, 0.0, -10.0], [3.15, 4.0, 10.0])]);
    let def = body();
    p.spawn_ragdoll("thrown", &def, &start(&def, Vec3::new(0.0, 1.2, 0.0), Vec3::new(40.0, 3.0, 0.0))).unwrap();
    let mut worst = f32::MIN;
    for _ in 0..(3.0 / DT) as usize {
        step(&mut p, &mut s);
        let poses = p.ragdoll_poses("thrown").unwrap();
        worst = worst.max(def.bodies.iter().zip(&poses).map(|(b, (t, r))| {
            let mid = *t + *r * Vec3::from(b.shape_pos);
            let reach = match b.shape {
                RagdollShape::Capsule { radius, half_height } => radius + half_height * (*r * Quat::from_array(b.shape_rot) * Vec3::X).x.abs(),
                RagdollShape::Box { half } => Vec3::from(half).length(),
                RagdollShape::Sphere { radius } => radius,
            };
            mid.x + reach
        }).fold(f32::MIN, f32::max));
        assert!(poses.iter().all(|(t, _)| t.is_finite() && t.length() < 100.0), "it exploded");
    }
    eprintln!("thrown at 40 m/s: deepest collider edge at x {worst:.3} (wall face at 3.000, back at 3.150)");
    assert!(worst < 3.06, "a limb went {:.0} mm into the wall", (worst - 3.0) * 1000.0);
    assert!(worst_gap(&def, &p.ragdoll_poses("thrown").unwrap()) < 0.05, "it came apart against the wall");
    let mut tearable = body();
    for b in [1usize, 2, 5] {
        tearable.bodies[b].break_force = 600.0;
    }
    p.spawn_ragdoll("tearable", &tearable, &start(&tearable, Vec3::new(0.0, 1.2, 3.0), Vec3::new(40.0, 3.0, 0.0))).unwrap();
    for _ in 0..(3.0 / DT) as usize {
        step(&mut p, &mut s);
    }
    assert!(p.ragdoll_torn("tearable").is_empty(), "the impact tore {:?} off", p.ragdoll_torn("tearable"));
}

#[test]
fn limbs_tear_off_by_hand_and_by_their_break_force() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0])]);
    let def = body();
    p.spawn_ragdoll("a", &def, &start(&def, Vec3::new(0.0, 0.4, 0.0), Vec3::ZERO)).unwrap();
    p.tear_ragdoll_limb("a", 1);
    assert_eq!(p.ragdoll_torn("a"), vec![1]);
    p.push_ragdoll_limb("a", 1, Vec3::new(0.0, 0.0, 15.0));
    for _ in 0..60 {
        step(&mut p, &mut s);
    }
    let poses = p.ragdoll_poses("a").unwrap();
    assert!(poses[1].0.distance(poses[0].0) > 0.9, "the torn head stayed on");

    const BREAK: f32 = 600.0;
    let mut weak = body();
    weak.bodies[3].break_force = BREAK;
    p.spawn_ragdoll("b", &weak, &start(&weak, Vec3::new(3.0, 0.4, 0.0), Vec3::ZERO)).unwrap();
    for _ in 0..60 {
        step(&mut p, &mut s);
    }
    assert!(p.ragdoll_torn("b").is_empty(), "the forearm tore just landing, before any pull");
    let torso = p.ragdoll_poses("b").unwrap()[0].0;
    let hold = p.grip_ragdoll("b", 0, torso, MONSTER_GRIP).unwrap();
    let hand = p.ragdoll_poses("b").unwrap()[4].0;
    let grip = p.grip_ragdoll("b", 4, hand, MONSTER_GRIP).unwrap();
    let mut torn_at = None;
    for k in 0..240 {
        p.move_ragdoll_grip(hold, torso);
        p.move_ragdoll_grip(grip, hand + Vec3::new(-0.01 * k as f32, 0.005 * k as f32, 0.0));
        step(&mut p, &mut s);
        if torn_at.is_none() && p.ragdoll_torn("b").contains(&3) {
            torn_at = Some(k);
        }
    }
    eprintln!("a {BREAK} N forearm tore after {torn_at:?} steps of a monster's pull");
    assert!(torn_at.is_some(), "the weak joint never tore");
    assert!(!p.ragdoll_torn("b").contains(&1), "the wrong limb tore");

    p.spawn_ragdoll("c", &weak, &start(&weak, Vec3::new(-3.0, 0.4, 0.0), Vec3::ZERO)).unwrap();
    for _ in 0..60 {
        step(&mut p, &mut s);
    }
    let torso = p.ragdoll_poses("c").unwrap()[0].0;
    let hold = p.grip_ragdoll("c", 0, torso, MONSTER_GRIP).unwrap();
    let hand = p.ragdoll_poses("c").unwrap()[4].0;
    let grip = p.grip_ragdoll("c", 4, hand, PLAYER_GRIP).unwrap();
    for k in 0..240 {
        p.move_ragdoll_grip(hold, torso);
        p.move_ragdoll_grip(grip, hand + Vec3::new(-0.01 * k as f32, 0.005 * k as f32, 0.0));
        step(&mut p, &mut s);
        if std::env::var_os("LOAD").is_some() && k % 10 == 0 {
            eprintln!("  k {k}: forearm joint {:?} grip holds {} torn {:?}", p.ragdoll_joint_load("c", 3), p.ragdoll_grip_holds(grip), p.ragdoll_torn("c"));
        }
    }
    assert!(p.ragdoll_torn("c").is_empty(), "a player's hand tore an arm off");
    assert!(!p.ragdoll_grip_holds(grip), "the player's hand never let go");
}

#[test]
fn a_hand_drags_a_body_and_lets_go_of_one_snagged_on_a_wall() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0]), block("wall", [3.0, 0.0, -10.0], [3.3, 4.0, 10.0])]);
    let def = body();
    p.spawn_ragdoll("c", &def, &start(&def, Vec3::new(0.0, 0.25, 0.0), Vec3::ZERO)).unwrap();
    for _ in 0..60 {
        step(&mut p, &mut s);
    }
    let torso = p.ragdoll_poses("c").unwrap()[0].0;
    let hold = torso + Vec3::new(0.0, 0.2, 0.0);
    let grip = p.grip_ragdoll("c", 0, hold, PLAYER_GRIP).unwrap();
    for k in 1..=120 {
        p.move_ragdoll_grip(grip, hold + Vec3::new(2.0 * k as f32 / 120.0, 0.3, 0.0));
        step(&mut p, &mut s);
    }
    let dragged = p.ragdoll_poses("c").unwrap()[0].0;
    eprintln!("dragged 2 m: the torso came {:.2} m", dragged.x - torso.x);
    assert!(p.ragdoll_grip_holds(grip));
    assert!(dragged.x - torso.x > 1.5, "it did not come along");

    for k in 1..=120 {
        p.move_ragdoll_grip(grip, hold + Vec3::new(2.0 + 3.0 * k as f32 / 120.0, 0.3, 0.0));
        step(&mut p, &mut s);
    }
    let furthest = p.ragdoll_poses("c").unwrap().iter().map(|(t, _)| t.x).fold(f32::MIN, f32::max);
    eprintln!("pulled through a wall: furthest bone at x {furthest:.2} (wall face 3.00), grip holds: {}", p.ragdoll_grip_holds(grip));
    assert!(furthest < 3.0, "the body was pulled through the wall");
    assert!(!p.ragdoll_grip_holds(grip), "the hand kept hold through a wall");
    p.release_ragdoll_grip(grip);
}

#[test]
fn a_frozen_corpse_stays_put_and_a_rebuild_lets_ragdolls_go() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0])]);
    let def = body();
    p.spawn_ragdoll("d", &def, &start(&def, Vec3::new(0.0, 1.0, 0.0), Vec3::ZERO)).unwrap();
    for _ in 0..90 {
        step(&mut p, &mut s);
    }
    p.freeze_ragdoll("d", true);
    let before = p.ragdoll_poses("d").unwrap();
    assert!(!p.ragdoll_awake("d"));
    p.spawn_ragdoll("e", &def, &start(&def, Vec3::new(0.0, 2.5, 0.0), Vec3::ZERO)).unwrap();
    for _ in 0..90 {
        step(&mut p, &mut s);
    }
    assert_eq!(p.ragdoll_poses("d").unwrap(), before);
    p.freeze_ragdoll("d", false);
    assert!(p.ragdoll_awake("d"));
    let g = p.grip_ragdoll("e", 0, p.ragdoll_poses("e").unwrap()[0].0, PLAYER_GRIP).unwrap();
    p.rebuild(&s, Path::new("."));
    assert!(p.ragdoll_ids().is_empty());
    assert!(!p.ragdoll_grip_holds(g));
    for _ in 0..10 {
        step(&mut p, &mut s);
    }
    p.spawn_ragdoll("f", &def, &start(&def, Vec3::new(0.0, 1.0, 0.0), Vec3::ZERO)).unwrap();
    step(&mut p, &mut s);
    p.remove_ragdoll("f");
    assert!(p.ragdoll_poses("f").is_none());
}

/// What eight bodies falling at once cost a step (release builds only mean anything).
#[test]
fn eight_bodies_falling_at_once_cost() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0])]);
    let def = body();
    for k in 0..8 {
        let at = Vec3::new((k % 4) as f32 * 0.6 - 0.9, 1.0 + (k / 4) as f32 * 1.2, 0.0);
        p.spawn_ragdoll(&format!("r{k}"), &def, &start(&def, at, Vec3::new(0.5, 0.0, 0.0))).unwrap();
    }
    let mut times = Vec::new();
    for _ in 0..(3.0 / DT) as usize {
        let t = std::time::Instant::now();
        step(&mut p, &mut s);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let awake = (0..8).filter(|k| p.ragdoll_awake(&format!("r{k}"))).count();
    eprintln!(
        "8 ragdolls (64 bodies) falling into a pile: step p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms; {awake} still awake after 3 s",
        times[times.len() / 2],
        times[times.len() * 99 / 100],
        times[times.len() - 1]
    );
}

/// Measures joint loads (run with LOADS=1): a body landing or hitting a wall against a monster
/// pulling an arm off, to set how long a pull must last to tear.
#[test]
fn joint_loads_impacts_against_pulls() {
    if std::env::var_os("LOADS").is_none() {
        return;
    }
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, mut s) = world(vec![block("ground", [-20.0, -1.0, -20.0], [20.0, 0.0, 20.0]), block("wall", [6.0, 0.0, -10.0], [6.3, 4.0, 10.0])]);
    let def = body();
    let run = |p: &mut PhysicsWorld, s: &mut Scene, id: &str, steps: usize, mv: &mut dyn FnMut(&mut PhysicsWorld, usize)| -> Vec<f32> {
        let mut loads = vec![0.0f32; steps];
        for k in 0..steps {
            mv(p, k);
            step(p, s);
            loads[k] = (1..def.bodies.len()).filter_map(|b| p.ragdoll_joint_load(id, b)).map(|l| l.0).fold(0.0, f32::max);
        }
        loads
    };
    let over = |loads: &[f32], f: f32| loads.iter().filter(|l| **l > f).count();
    p.spawn_ragdoll("drop", &def, &start(&def, Vec3::new(0.0, 2.0, 0.0), Vec3::ZERO)).unwrap();
    let l = run(&mut p, &mut s, "drop", 180, &mut |_, _| {});
    eprintln!("2 m drop: peak {:.0} N, steps over 600 N: {}", l.iter().cloned().fold(0.0, f32::max), over(&l, 600.0));
    p.spawn_ragdoll("wall", &def, &start(&def, Vec3::new(3.0, 1.2, 4.0), Vec3::new(30.0, 2.0, 0.0))).unwrap();
    let l = run(&mut p, &mut s, "wall", 180, &mut |_, _| {});
    eprintln!("30 m/s into a wall: peak {:.0} N, steps over 600 N: {}", l.iter().cloned().fold(0.0, f32::max), over(&l, 600.0));
    p.spawn_ragdoll("pull", &def, &start(&def, Vec3::new(-6.0, 0.4, 0.0), Vec3::ZERO)).unwrap();
    for _ in 0..90 {
        step(&mut p, &mut s);
    }
    let torso = p.ragdoll_poses("pull").unwrap()[0].0;
    let hand = p.ragdoll_poses("pull").unwrap()[4].0;
    for (name, strength) in [("monster", MONSTER_GRIP), ("player", PLAYER_GRIP)] {
        let hold = p.grip_ragdoll("pull", 0, torso, MONSTER_GRIP).unwrap();
        let grip = p.grip_ragdoll("pull", 4, hand, strength).unwrap();
        let l = run(&mut p, &mut s, "pull", 150, &mut |p, k| {
            p.move_ragdoll_grip(hold, torso);
            p.move_ragdoll_grip(grip, hand + Vec3::new(-0.01 * k as f32, 0.005 * k as f32, 0.0));
        });
        let mut longest = 0;
        let mut run_len = 0;
        for x in &l {
            run_len = if *x > 600.0 { run_len + 1 } else { 0 };
            longest = longest.max(run_len);
        }
        eprintln!("{name} pulling an arm, body held: peak {:.0} N, steps over 600 N: {}, longest run over 600 N: {longest}", l.iter().cloned().fold(0.0, f32::max), over(&l, 600.0));
        p.release_ragdoll_grip(grip);
        p.release_ragdoll_grip(hold);
        for _ in 0..60 {
            step(&mut p, &mut s);
        }
    }
}
