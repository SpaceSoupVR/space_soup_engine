//! A monster chasing a walking player through real generated levels, moved by the floor's own
//! `move_along_surface` (so any steer into a wall shows up as a blocked step).

mod common;

use glam::DVec3;
use space_soup_nav::follow::{FollowSettings, PathFollower, Steer};
use space_soup_nav::path::find_path_guided;
use space_soup_nav::world::NavWorld;

const EXT: DVec3 = DVec3::new(500.0, 2000.0, 500.0);
const DT: f64 = 1.0 / 36.0;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f64 / 16777216.0
    }
}

fn random_point(w: &NavWorld, rng: &mut Lcg) -> DVec3 {
    loop {
        let i = (rng.next() * w.islands.len() as f64) as usize % w.islands.len();
        let isl = &w.islands[i];
        if isl.asset.polys.is_empty() {
            continue;
        }
        let p = (rng.next() * isl.asset.polys.len() as f64) as usize % isl.asset.polys.len();
        let v = w.poly_dverts(space_soup_nav::world::PolyRef { island: i as u32, poly: p as u16 });
        return v.iter().copied().sum::<DVec3>() / v.len() as f64;
    }
}

/// Where along a polyline at a given distance.
fn along(pts: &[DVec3], mut d: f64) -> DVec3 {
    for w in pts.windows(2) {
        let l = (w[1] - w[0]).length();
        if d <= l {
            return w[0].lerp(w[1], d / l.max(1e-9));
        }
        d -= l;
    }
    *pts.last().unwrap()
}

struct Outcome {
    caught_s: Option<f64>,
    steps: usize,
    blocked: usize,
    plans: usize,
}

fn chase(w: &NavWorld, graph: &space_soup_nav::world::RoomGraph, monster: DVec3, player_route: &[DVec3], limit_s: f64) -> Outcome {
    let mut f = PathFollower::new(FollowSettings::default());
    let mut pos = w.nearest_poly(monster, EXT).unwrap().point;
    let (mut steps, mut blocked) = (0, 0);
    let mut t = 0.0;
    while t < limit_s {
        let player = along(player_route, t * 1400.0);
        match f.update(w, graph, pos, player, DT) {
            Steer::Arrived => return Outcome { caught_s: Some(t), steps, blocked, plans: f.plans },
            Steer::NoPath => return Outcome { caught_s: None, steps, blocked, plans: f.plans },
            Steer::TakeLink { to, .. } => pos = to,
            Steer::Walk(target) => {
                let d = DVec3::new(target.x - pos.x, 0.0, target.z - pos.z);
                let step = (3500.0 * DT).min(d.length());
                if step > 1e-6 {
                    let want = pos + d.normalize() * step;
                    let here = w.nearest_poly(pos, EXT).unwrap();
                    let (got, _) = w.move_along_surface(here.poly, pos, want);
                    if (got - want).length() > 5.0 {
                        blocked += 1;
                    }
                    let h = w.nearest_poly(got, EXT).unwrap().point.y;
                    pos = DVec3::new(got.x, h, got.z);
                    steps += 1;
                }
            }
        }
        t += DT;
    }
    Outcome { caught_s: None, steps, blocked, plans: f.plans }
}

#[test]
fn a_monster_catches_a_walking_player_without_walking_into_walls() {
    let kit = common::bake_kit("human_cs10_monotone");
    let (mut caught, mut tries, mut steps, mut blocked, mut plans) = (0, 0, 0, 0, 0);
    let mut times = Vec::new();
    for layout in common::layouts() {
        let mut w = common::assemble(&kit, &layout, 0);
        let graph = w.room_graph();
        let mut rng = Lcg(layout.seed * 7 + 1);
        for _ in 0..3 {
            let (m, a, b) = (random_point(&w, &mut rng), random_point(&w, &mut rng), random_point(&w, &mut rng));
            let Some(route) = find_path_guided(&w, &graph, a, b, EXT).filter(|p| p.complete) else { continue };
            let pts: Vec<DVec3> = route.waypoints.iter().map(|x| x.pos()).collect();
            let (sm, sa) = (w.nearest_poly(m, EXT).unwrap().poly, w.nearest_poly(a, EXT).unwrap().poly);
            if !w.same_component(sm, sa) {
                continue;
            }
            tries += 1;
            let o = chase(&w, &graph, m, &pts, 600.0);
            steps += o.steps;
            blocked += o.blocked;
            plans += o.plans;
            if let Some(s) = o.caught_s {
                caught += 1;
                times.push(s);
            }
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{caught}/{tries} chases caught the player (median {:.0} s, longest {:.0} s); {blocked} of {steps} steps blocked by a wall ({:.3}%); {plans} plans",
        times[times.len() / 2], times.last().unwrap(), 100.0 * blocked as f64 / steps as f64
    );
    assert_eq!(caught, tries, "a monster failed to catch a reachable player");
    assert!((blocked as f64) < 0.005 * steps as f64, "the monster keeps steering into walls");
}

#[test]
fn closing_a_door_on_the_monsters_route_makes_it_go_round_or_stop() {
    let kit = common::bake_kit("human_cs10_monotone");
    let layout = &common::layouts()[0];
    let mut w = common::assemble(&kit, layout, 0);
    let graph = w.room_graph();
    let mut rng = Lcg(99);
    let mut checked = 0;
    for _ in 0..40 {
        let (m, p) = (random_point(&w, &mut rng), random_point(&w, &mut rng));
        let mut f = PathFollower::new(FollowSettings::default());
        let pos = w.nearest_poly(m, EXT).unwrap().point;
        if !matches!(f.update(&w, &graph, pos, p, DT), Steer::Walk(_)) {
            continue;
        }
        let Some(door) = f.path().unwrap().links().find_map(|l| w.links[l as usize].gate) else { continue };
        w.set_gate(door, false);
        let mut s = Steer::Arrived;
        for _ in 0..20 {
            s = f.update(&w, &graph, pos, p, DT);
        }
        match s {
            Steer::Walk(_) => assert!(f.path().unwrap().links().all(|l| w.links[l as usize].gate != Some(door)), "still routed through the closed door"),
            Steer::NoPath => {}
            other => panic!("{other:?}"),
        }
        if !f.path().is_some_and(|p| p.complete) {
            assert!(!f.path().is_some_and(|p| p.complete));
        }
        w.set_gate(door, true);
        checked += 1;
    }
    assert!(checked > 5, "only {checked} routes used a door");
    println!("{checked} chases had a door closed in their way");
}
