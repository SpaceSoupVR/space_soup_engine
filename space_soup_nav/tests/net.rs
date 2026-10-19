//! Multiplayer safety: headsets with different levels find out and say how, door states carry
//! over, and a new host takes over a crowd mid-scenario without losing anyone.

mod common;

use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::Turn;
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput};
use space_soup_nav::crowd::{AgentParams, AgentSnapshot, Crowd, CrowdSettings, Goal, Player};
use space_soup_nav::golden;
use space_soup_nav::net::{apply_gate_states, gate_states, LevelDigest};
use space_soup_nav::obj::TriMesh;
use space_soup_nav::scenario::{self, Goal as SGoal, Scenario};
use space_soup_nav::world::{NavWorld, RoomGraph};

#[test]
fn headsets_with_the_same_level_agree_and_different_ones_find_out_why() {
    let kit = common::bake_kit("human_cs10_monotone");
    let layouts = common::layouts();
    let host = LevelDigest::of(&common::assemble(&kit, &layouts[0], 0));
    // Built again, from scratch, on "another headset": the same.
    let again = LevelDigest::of(&common::assemble(&common::bake_kit("human_cs10_monotone"), &layouts[0], 0));
    assert_eq!(again, host);
    assert_eq!(again.hash(), host.hash());
    assert!(again.differences(&host, 10).is_empty());
    // It goes over the wire as JSON and comes back the same.
    let wire: LevelDigest = serde_json::from_str(&serde_json::to_string(&host).unwrap()).unwrap();
    assert_eq!(wire.hash(), host.hash());

    // Another seed: a different level.
    let other = LevelDigest::of(&common::assemble(&kit, &layouts[1], 0));
    assert_ne!(other.hash(), host.hash());
    assert!(!other.differences(&host, 10).is_empty());

    // One room baked differently (an older bake of one piece): named, by room.
    let mut stale_kit = kit.clone();
    let k = layouts[0].pieces[3].kind;
    let mut old = (*stale_kit[k]).clone();
    old.vertices[0].y += 10;
    stale_kit[k] = Arc::new(old);
    let stale = LevelDigest::of(&common::assemble(&stale_kit, &layouts[0], 0));
    assert_ne!(stale.hash(), host.hash());
    let why = stale.differences(&host, 3);
    assert!(why[0].starts_with("room") && why[0].contains(&kit[k].name) && why[0].contains("different bake"), "{why:?}");
    assert!(why.len() <= 3);

    // The other size class's bake: every room differs.
    let large = LevelDigest::of(&common::assemble(&common::bake_kit("large_cs10_layers"), &layouts[0], 0));
    assert_ne!(large.hash(), host.hash());

    // The whole level turned a quarter: same rooms, placed differently.
    let turned = LevelDigest::of(&common::assemble(&kit, &layouts[0], 1));
    assert!(turned.differences(&host, 1)[0].contains("placed at"), "{:?}", turned.differences(&host, 1));

    // A join missing (a door the host has that this headset never made).
    let mut fewer = host.clone();
    fewer.joins.pop();
    assert_ne!(fewer.hash(), host.hash());
    assert!(fewer.differences(&host, 10).iter().any(|d| d.contains("joins here")));
    // A door where the host has an open doorway.
    let mut door = host.clone();
    door.joins[0].door = !door.joins[0].door;
    assert_ne!(door.hash(), host.hash());
}

#[test]
fn door_states_carry_over_to_another_headset() {
    let kit = common::bake_kit("human_cs10_monotone");
    let layout = &common::layouts()[0];
    let mut host = common::assemble(&kit, layout, 0);
    let mut late = common::assemble(&kit, layout, 0);
    for g in (0..host.gates.len() as u32).step_by(3) {
        host.set_gate(g, false);
    }
    let shut = host.gates.iter().filter(|g| !g.open).count();
    let events = apply_gate_states(&mut late, &gate_states(&host)).unwrap();
    assert_eq!(events.len(), shut);
    assert_eq!(gate_states(&late), gate_states(&host));
    // The worlds now agree on what joins what.
    for i in 0..host.islands.len() as u32 {
        let p = space_soup_nav::world::PolyRef { island: i, poly: 0 };
        let q = space_soup_nav::world::PolyRef { island: 0, poly: 0 };
        if host.islands[i as usize].asset.polys.is_empty() {
            continue;
        }
        assert_eq!(host.same_component(p, q), late.same_component(p, q));
    }
    // Applying the same states again changes nothing; a different level's doors are refused.
    assert!(apply_gate_states(&mut late, &gate_states(&host)).unwrap().is_empty());
    assert!(apply_gate_states(&mut late, &[true; 3]).is_err());
}

// --- host migration -------------------------------------------------------------------------

const EXT: DVec3 = DVec3::new(500.0, 2000.0, 500.0);

fn mm(v: [f32; 3]) -> DVec3 {
    DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64) * 1000.0
}

/// The scenario's world, as `tests/crowd.rs` builds it. Each headset bakes its own.
fn world_for(s: &Scenario) -> NavWorld {
    let mesh = TriMesh::load(&golden::test_data_dir().join("meshes").join(&s.mesh)).unwrap();
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
    let mut settings = common::settings(&common::config("human_cs10_monotone"));
    settings.agent_radius = s.agents[0].radius;
    let input = PieceInput { name: &s.name, vertices: &verts, triangles: &mesh.triangles, sockets: &[], links: Vec::new() };
    let mut w = NavWorld::new();
    w.add_island(Arc::new(bake_piece(&input, &settings, [1.0; 3], &ExportOptions::default()).unwrap().0), Turn(0), IVec3::ZERO);
    w
}

fn scenario(name: &str) -> Scenario {
    scenario::load_all(&golden::test_data_dir()).unwrap().into_iter().find(|s| s.name == name).unwrap()
}

fn start(s: &Scenario, w: &NavWorld) -> Crowd {
    let mut crowd = Crowd::new(w, CrowdSettings::default());
    for a in &s.agents {
        let id = crowd.add_agent(
            AgentParams { radius_mm: a.radius as f64 * 1000.0, max_speed_mm_s: a.max_speed as f64 * 1000.0, priority: 0 },
            w.nearest_poly(mm(a.start), EXT).unwrap().point,
        );
        crowd.set_goal(id, match a.goal {
            SGoal::Point(p) => Goal::Point(w.nearest_poly(mm(p), EXT).unwrap().point),
            SGoal::Player(i) => Goal::Player(i),
        });
    }
    crowd
}

/// One tick the way the game does it: players in, crowd update, agents moved along the floor.
/// Returns the deepest overlap between agents this tick, mm.
fn tick(s: &Scenario, w: &NavWorld, g: &RoomGraph, crowd: &mut Crowd, t: f64) -> f64 {
    let dt = s.dt as f64;
    for i in 0..s.players.len() {
        let (now, next) = (mm(s.player_at(i, t as f32).to_array()), mm(s.player_at(i, (t + dt) as f32).to_array()));
        crowd.set_player(i, Player { pos: now, vel: (next - now) / dt, radius_mm: s.players[i].radius as f64 * 1000.0 });
    }
    crowd.update(w, g, dt);
    for i in 0..crowd.agents.len() {
        let a = &crowd.agents[i];
        let here = w.nearest_poly(a.pos, EXT).unwrap();
        let (pos, _) = w.slide(here.poly, a.pos, a.pos + a.desired * dt);
        crowd.set_agent(i, pos, (pos - a.pos) / dt);
    }
    let flat = |a: DVec3, b: DVec3| ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt();
    let mut worst: f64 = 0.0;
    for i in 0..crowd.agents.len() {
        for j in i + 1..crowd.agents.len() {
            let (a, b) = (&crowd.agents[i], &crowd.agents[j]);
            worst = worst.max(a.params.radius_mm + b.params.radius_mm - flat(a.pos, b.pos));
        }
    }
    worst
}

fn all_arrived(s: &Scenario, crowd: &Crowd) -> bool {
    let r = s.pass.arrive_radius as f64 * 1000.0;
    crowd.agents.iter().all(|a| match a.goal {
        Goal::Point(p) => ((a.pos.x - p.x).powi(2) + (a.pos.z - p.z).powi(2)).sqrt() <= r,
        _ => true,
    })
}

/// A scenario run, handed to a new host at `handover_s` if given: when everyone with a point goal
/// had arrived, when an agent first reached a player, and the deepest overlap, mm.
fn run(s: &Scenario, handover_s: Option<f64>) -> (Option<f64>, Option<f64>, f64) {
    let dt = s.dt as f64;
    let steps = (s.duration_s as f64 / dt) as usize;
    let reach = s.pass.reach as f64 * 1000.0;
    let mut w = world_for(s);
    let mut g = w.room_graph();
    let mut crowd = start(s, &w);
    let (mut arrived, mut contact, mut overlap) = (None, None, 0.0f64);
    let mut handover = handover_s;
    let mut t = 0.0;
    for _ in 0..steps {
        if handover.is_some_and(|h| t >= h) {
            handover = None;
            // Only what the host replicates reaches the new host, through the wire format, and
            // the new host bakes its own world.
            let agents: Vec<AgentSnapshot> = serde_json::from_str(&serde_json::to_string(&crowd.snapshot()).unwrap()).unwrap();
            let new_world = world_for(s);
            assert_eq!(LevelDigest::of(&new_world).hash(), LevelDigest::of(&w).hash());
            let new_host = Crowd::from_snapshot(&new_world, CrowdSettings::default(), &agents, &crowd.players);
            // A second headset building from the same snapshot simulates the same thing.
            let mut twin = Crowd::from_snapshot(&new_world, CrowdSettings::default(), &agents, &crowd.players);
            let mut probe = Crowd::from_snapshot(&new_world, CrowdSettings::default(), &agents, &crowd.players);
            let ng = new_world.room_graph();
            for k in 0..(2.0 / dt) as usize {
                let tk = t + k as f64 * dt;
                tick(s, &new_world, &ng, &mut probe, tk);
                tick(s, &new_world, &ng, &mut twin, tk);
            }
            assert!(probe.agents.iter().zip(&twin.agents).all(|(a, b)| a.pos == b.pos), "{}: two new hosts disagree", s.name);
            (w, g, crowd) = (new_world, ng, new_host);
        }
        overlap = overlap.max(tick(s, &w, &g, &mut crowd, t));
        t += dt;
        if arrived.is_none() && crowd.agents.iter().any(|a| matches!(a.goal, Goal::Point(_))) && all_arrived(s, &crowd) {
            arrived = Some(t);
        }
        let flat = |a: DVec3, b: DVec3| ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt();
        if contact.is_none() && crowd.players.iter().any(|p| crowd.agents.iter().any(|a| flat(a.pos, p.pos) <= reach)) {
            contact = Some(t);
        }
    }
    (arrived, contact, overlap)
}

#[test]
fn a_new_host_takes_over_any_scenario_without_it_showing() {
    for s in scenario::load_all(&golden::test_data_dir()).unwrap() {
        let (arrived, contact, _) = run(&s, None);
        let max_overlap = s.pass.max_overlap as f64 * 1000.0;
        let end = arrived.or(contact).expect("the uninterrupted run finishes");
        // Hand over at several moments up to the end, the busiest (queued and in the doorway,
        // closing on a player) included.
        for k in 1..=4 {
            let h = (end * k as f64 / 5.0 / s.dt as f64).round() * s.dt as f64;
            let (a, c, overlap) = run(&s, Some(h));
            eprintln!("{:24} handed over at {h:4.1} s: arrived {a:?} (uninterrupted {arrived:?}), contact {c:?} ({contact:?}), overlap {overlap:.0} mm", s.name);
            assert!(overlap <= max_overlap, "{}: overlap {overlap:.0} mm", s.name);
            // The scenario's own limits hold, and a host change costs at most half a second.
            if let (Some(limit), Some(base)) = (s.pass.all_arrive_within_s, arrived) {
                let a = a.unwrap_or_else(|| panic!("{} handed over at {h} s: not everyone arrived", s.name));
                assert!(a <= limit as f64 && a <= base + 0.5, "{} handed over at {h} s: arrived {a:.1} s, uninterrupted {base:.1} s", s.name);
            }
            if let (Some(limit), Some(base)) = (s.pass.first_contact_within_s, contact) {
                let c = c.unwrap_or_else(|| panic!("{} handed over at {h} s: never reached the player", s.name));
                assert!(c <= limit as f64 && c <= base + 0.5, "{} handed over at {h} s: contact {c:.1} s, uninterrupted {base:.1} s", s.name);
            }
        }
    }
}

#[test]
fn replanning_everyone_at_once_loses_nobody() {
    let s = scenario("doorway_swap");
    let w = world_for(&s);
    let g = w.room_graph();
    let mut crowd = start(&s, &w);
    let dt = s.dt as f64;
    let mut t = 0.0;
    let mut arrived = None;
    for step in 0..(s.duration_s as f64 / dt) as usize {
        // Every four seconds, as if the door states had been replaced wholesale.
        if step > 0 && step % (4.0 / dt) as usize == 0 {
            crowd.replan_all();
        }
        tick(&s, &w, &g, &mut crowd, t);
        t += dt;
        if arrived.is_none() && all_arrived(&s, &crowd) {
            arrived = Some(t);
        }
    }
    let arrived = arrived.expect("not everyone arrived");
    eprintln!("doorway_swap replanned every 4 s: all arrived at {arrived:.1} s");
    assert!(arrived <= s.pass.all_arrive_within_s.unwrap() as f64);
}

#[test]
fn turns_and_slots_survive_a_snapshot_exactly() {
    for (name, at) in [("doorway_swap", 8.0), ("converge", 2.5)] {
        let s = scenario(name);
        let w = world_for(&s);
        let g = w.room_graph();
        let mut crowd = start(&s, &w);
        let mut t = 0.0;
        while t < at {
            tick(&s, &w, &g, &mut crowd, t);
            t += s.dt as f64;
        }
        let snap = crowd.snapshot();
        assert!(snap.iter().any(|a| a.turn.is_some() || a.slot.is_some()), "{name}: nothing to carry over at {at} s");
        let rebuilt = Crowd::from_snapshot(&w, CrowdSettings::default(), &snap, &crowd.players);
        assert_eq!(rebuilt.snapshot(), snap, "{name}");
        for c in 0..crowd.chokes.len() {
            assert_eq!(rebuilt.choke_state(c), crowd.choke_state(c), "{name}: choke {c}");
        }
    }
    // A snapshot from before turns and slots were carried still reads: those agents ask again.
    let old = r#"[{"pos":[0.0,0.0,0.0],"vel":[0.0,0.0,0.0],"params":{"radius_mm":400.0,"max_speed_mm_s":3000.0,"priority":0},"goal":"Idle"}]"#;
    let agents: Vec<AgentSnapshot> = serde_json::from_str(old).unwrap();
    assert!(agents[0].turn.is_none() && agents[0].slot.is_none());
}

/// Every headset must bake the same bytes. `tests/data/bake_hashes.json` holds every kit
/// piece's content hash and every test level's digest hash, per agent config, as made on the
/// development Mac; any other machine (the Quest above all) must match them exactly. Regenerate
/// with `UPDATE_BAKE_HASHES=1` when the baker or the kit changes on purpose.
#[test]
fn every_machine_bakes_the_same_bytes() {
    let mut now = std::collections::BTreeMap::new();
    for class in ["human_cs10_monotone", "human_cs20_layers", "large_cs10_layers"] {
        let kit = common::bake_kit(class);
        for a in &kit {
            now.insert(format!("{class}/{}", a.name), format!("{:016x}", a.content_hash()));
        }
        for l in common::layouts() {
            now.insert(format!("{class}/level{:02}", l.seed), format!("{:016x}", LevelDigest::of(&common::assemble(&kit, &l, 0)).hash()));
        }
    }
    let path = golden::test_data_dir().join("bake_hashes.json");
    if std::env::var_os("UPDATE_BAKE_HASHES").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
    }
    let want: std::collections::BTreeMap<String, String> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let differ: Vec<String> = want.iter().filter(|(k, v)| now.get(*k) != Some(v)).map(|(k, v)| format!("{k}: want {v}, got {:?}", now.get(k))).collect();
    assert!(differ.is_empty() && want.len() == now.len(), "{} of {} differ:\n{}", differ.len(), want.len(), differ.join("\n"));
}
