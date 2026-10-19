//! The crowd against the scenarios written in phase 0, before it existed
//! (`tests/data/scenarios`): their pass criteria are the judge, unchanged.

mod common;

use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::Turn;
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput};
use space_soup_nav::crowd::{AgentParams, AgentState, Crowd, CrowdSettings, Goal, Player};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;
use space_soup_nav::scenario::{self, Goal as SGoal, Scenario};
use space_soup_nav::world::NavWorld;

const EXT: DVec3 = DVec3::new(500.0, 2000.0, 500.0);

fn mm(v: [f32; 3]) -> DVec3 {
    DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64) * 1000.0
}

fn world_for(s: &Scenario) -> NavWorld {
    let mesh = TriMesh::load(&golden::test_data_dir().join("meshes").join(&s.mesh)).unwrap();
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
    let mut settings = common::settings(&common::config("human_cs10_monotone"));
    settings.agent_radius = s.agents[0].radius;
    let input = PieceInput { name: &s.name, vertices: &verts, triangles: &mesh.triangles, sockets: &[], links: Vec::new() };
    let (asset, _) = bake_piece(&input, &settings, [1.0; 3], &ExportOptions::default()).unwrap();
    let mut w = NavWorld::new();
    w.add_island(Arc::new(asset), Turn(0), IVec3::ZERO);
    w
}

#[derive(Debug, Default)]
struct Result {
    all_arrived_s: Option<f64>,
    first_contact_s: Option<f64>,
    max_overlap_mm: f64,
    arrived_at_end: (usize, usize),
    worst_stuck_s: f64,
    /// The longest stall of an agent not queued for a narrow place.
    worst_stuck_moving_s: f64,
    /// The longest an agent waited in a queue for a narrow place.
    longest_queue_s: f64,
    /// The longest any narrow place went without letting the next agent in while some waited.
    longest_handover_s: f64,
    stuck_agent: Option<(usize, AgentState)>,
    ms_per_tick: f64,
}

fn run(s: &Scenario) -> Result {
    let w = world_for(s);
    let g = w.room_graph();
    let mut crowd = Crowd::new(&w, CrowdSettings::default());
    for a in &s.agents {
        let pos = w.nearest_poly(mm(a.start), EXT).expect("agent start off the navmesh").point;
        let id = crowd.add_agent(AgentParams { radius_mm: a.radius as f64 * 1000.0, max_speed_mm_s: a.max_speed as f64 * 1000.0, priority: 0 }, pos);
        crowd.set_goal(id, match a.goal {
            SGoal::Point(p) => Goal::Point(w.nearest_poly(mm(p), EXT).expect("goal off the navmesh").point),
            SGoal::Player(i) => Goal::Player(i),
        });
    }
    let dt = s.dt as f64;
    let reach = s.pass.reach as f64 * 1000.0;
    let arrive = s.pass.arrive_radius as f64 * 1000.0;
    let stuck_d = s.pass.stuck_distance as f64 * 1000.0;
    let mut r = Result::default();
    let mut anchor: Vec<(DVec3, f64)> = crowd.agents.iter().map(|a| (a.pos, 0.0)).collect();
    let mut queued_since: Vec<Option<f64>> = vec![None; crowd.agents.len()];
    let mut was_waiting = vec![false; crowd.agents.len()];
    // Per narrow place: (who was inside last tick, time without letting anyone new in while queued).
    let mut handover: Vec<(Vec<usize>, f64)> = (0..crowd.chokes.len()).map(|_| (Vec::new(), 0.0)).collect();
    let flat = |a: DVec3, b: DVec3| ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt();
    let mut t = 0.0;
    let mut cost = std::time::Duration::ZERO;
    let steps = (s.duration_s as f64 / dt) as usize;
    for _ in 0..steps {
        for (i, _) in s.players.iter().enumerate() {
            let (now, next) = (mm(s.player_at(i, t as f32).to_array()), mm(s.player_at(i, (t + dt) as f32).to_array()));
            crowd.set_player(i, Player { pos: now, vel: (next - now) / dt, radius_mm: s.players[i].radius as f64 * 1000.0 });
        }
        let t0 = std::time::Instant::now();
        crowd.update(&w, &g, dt);
        cost += t0.elapsed();
        for i in 0..crowd.agents.len() {
            let a = &crowd.agents[i];
            let here = w.nearest_poly(a.pos, EXT).unwrap();
            let (pos, _) = w.slide(here.poly, a.pos, a.pos + a.desired * dt);
            let vel = (pos - a.pos) / dt;
            crowd.set_agent(i, pos, vel);
        }
        t += dt;
        if std::env::var("CROWD_TRACE").ok().as_deref() == Some(s.name.as_str()) && ((t / dt).round() as usize).is_multiple_of((1.0 / dt).round() as usize) {
            let line: Vec<String> = crowd.agents.iter().enumerate().map(|(i, a)| format!("{i}:{:?}@({:.3},{:.3})v({:.2},{:.2})->{}{}", a.state, a.pos.x / 1000.0, a.pos.z / 1000.0, a.desired.x / 1000.0, a.desired.z / 1000.0, a.target.map_or("-".into(), |t| format!("({:.3},{:.3})", t.x / 1000.0, t.z / 1000.0)), crowd.token_of(i).map_or(String::new(), |c| format!("T{c}")))).collect();
            let chokes: Vec<String> = (0..crowd.chokes.len()).map(|c| format!("{:?}", crowd.choke_state(c))).collect();
            println!("t {t:4.1} {} | chokes {}", line.join(" "), chokes.join(" "));
        }

        // Handovers: someone new inside resets the clock; only queued time counts.
        for (c, h) in handover.iter_mut().enumerate() {
            let (_, inside, queue) = crowd.choke_state(c);
            if inside.iter().any(|i| !h.0.contains(i)) || queue.is_empty() {
                h.1 = 0.0;
            } else {
                h.1 += dt;
                r.longest_handover_s = r.longest_handover_s.max(h.1);
            }
            h.0 = inside;
        }

        // Overlaps.
        let n = crowd.agents.len();
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&crowd.agents[i], &crowd.agents[j]);
                r.max_overlap_mm = r.max_overlap_mm.max(a.params.radius_mm + b.params.radius_mm - flat(a.pos, b.pos));
            }
            for p in &crowd.players {
                r.max_overlap_mm = r.max_overlap_mm.max(crowd.agents[i].params.radius_mm + p.radius_mm - flat(crowd.agents[i].pos, p.pos));
            }
        }
        // Arrival and contact.
        let point_goals: Vec<(usize, DVec3)> = crowd.agents.iter().enumerate().filter_map(|(i, a)| if let Goal::Point(p) = a.goal { Some((i, p)) } else { None }).collect();
        if r.all_arrived_s.is_none() && !point_goals.is_empty() && point_goals.iter().all(|&(i, p)| flat(crowd.agents[i].pos, p) <= arrive) {
            r.all_arrived_s = Some(t);
        }
        if r.first_contact_s.is_none() && crowd.players.iter().any(|p| crowd.agents.iter().any(|a| flat(a.pos, p.pos) <= reach)) {
            r.first_contact_s = Some(t);
        }
        // Stuck: no progress while it still has somewhere to go.
        for (i, a) in crowd.agents.iter().enumerate() {
            let done = match a.goal {
                Goal::Point(p) => flat(a.pos, p) <= arrive,
                Goal::Player(k) => crowd.players.get(k).is_some_and(|p| flat(a.pos, p.pos) <= reach),
                Goal::Idle => true,
            } || matches!(a.state, AgentState::Arrived | AgentState::Attacking | AgentState::Holding);
            let waiting = a.state == AgentState::Waiting;
            match (waiting, queued_since[i]) {
                (true, None) => queued_since[i] = Some(t),
                (false, Some(since)) => {
                    r.longest_queue_s = r.longest_queue_s.max(t - since);
                    queued_since[i] = None;
                }
                _ => {}
            }
            if waiting != was_waiting[i] {
                // A queue wait is measured on its own; the stall clock restarts either side of it.
                anchor[i] = (a.pos, t);
                was_waiting[i] = waiting;
            }
            if !waiting && !done && t - anchor[i].1 > r.worst_stuck_moving_s {
                r.worst_stuck_moving_s = t - anchor[i].1;
            }
            if done || flat(a.pos, anchor[i].0) > stuck_d {
                anchor[i] = (a.pos, t);
            } else if t - anchor[i].1 > r.worst_stuck_s {
                r.worst_stuck_s = t - anchor[i].1;
                r.stuck_agent = Some((i, a.state));
                if std::env::var("CROWD_STUCK").ok().as_deref() == Some(s.name.as_str()) && r.worst_stuck_s > 3.0 && r.worst_stuck_s - dt <= 3.0 {
                    println!("STUCK t {t:.1} agent {i} {:?} at ({:.2},{:.2}) target {:?} token {:?}", a.state, a.pos.x / 1000.0, a.pos.z / 1000.0, a.target.map(|p| (p.x / 1000.0, p.z / 1000.0)), crowd.token_of(i));
                }
            }
        }
    }
    for since in queued_since.into_iter().flatten() {
        r.longest_queue_s = r.longest_queue_s.max(t - since);
    }
    r.ms_per_tick = cost.as_secs_f64() * 1000.0 / steps as f64;
    let goals: Vec<(DVec3, DVec3)> = crowd.agents.iter().filter_map(|a| if let Goal::Point(p) = a.goal { Some((a.pos, p)) } else { None }).collect();
    r.arrived_at_end = (goals.iter().filter(|(a, p)| flat(*a, *p) <= arrive).count(), goals.len());
    r
}

#[test]
fn every_crowd_scenario_meets_its_criteria() {
    let mut failures = Vec::new();
    for s in scenario::load_all(&golden::test_data_dir()).unwrap() {
        let r = run(&s);
        let p = &s.pass;
        let mut fail = Vec::new();
        if let Some(limit) = p.all_arrive_within_s {
            match r.all_arrived_s {
                Some(t) if t <= limit as f64 => {}
                other => fail.push(format!("all arrive within {limit} s: {other:?}")),
            }
        }
        if let Some(limit) = p.first_contact_within_s {
            match r.first_contact_s {
                Some(t) if t <= limit as f64 => {}
                other => fail.push(format!("first contact within {limit} s: {other:?}")),
            }
        }
        if r.max_overlap_mm > p.max_overlap as f64 * 1000.0 {
            fail.push(format!("overlap {:.0} mm > {:.0}", r.max_overlap_mm, p.max_overlap * 1000.0));
        }
        // Queue waits are exempt (see `PassCriteria::max_stuck_s`); a queue that stops is not.
        if r.worst_stuck_moving_s > p.max_stuck_s as f64 {
            fail.push(format!("stuck {:.1} s > {} s", r.worst_stuck_moving_s, p.max_stuck_s));
        }
        if let Some(limit) = p.max_handover_s {
            if r.longest_handover_s > limit as f64 {
                fail.push(format!("a queue stood still {:.1} s > {limit} s", r.longest_handover_s));
            }
        }
        println!(
            "{:<24} arrived {:>6} ({}/{}) contact {:>6} overlap {:>4.0} mm stuck {:>4.1} s, longest queue {:.1} s, handover {:.1} s  {:.3} ms/tick {}",
            s.name,
            r.all_arrived_s.map_or("-".into(), |t| format!("{t:.1}s")),
            r.arrived_at_end.0,
            r.arrived_at_end.1,
            r.first_contact_s.map_or("-".into(), |t| format!("{t:.1}s")),
            r.max_overlap_mm.max(0.0),
            r.worst_stuck_moving_s,
            r.longest_queue_s,
            r.longest_handover_s,
            r.ms_per_tick,
            if fail.is_empty() { "PASS".to_string() } else { format!("FAIL: {}", fail.join("; ")) }
        );
        if !fail.is_empty() {
            failures.push(s.name.clone());
        }
    }
    assert!(failures.is_empty(), "failed: {failures:?}");
}
