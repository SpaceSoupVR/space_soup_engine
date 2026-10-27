//! The runtime world assembled from real generated levels and the real baked kit.

mod common;

use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::{LinkKind as TypedKind, NavLink, Turn};
use space_soup_nav::world::{JoinError, JoinIssue, LinkKind, NavEvent, NavWorld, PolyRef, SocketRef};

fn hallway_index(kit_names: &[String]) -> usize {
    kit_names.iter().position(|n| n == "HallwayShort_DoorType_A").unwrap()
}

fn kit_names() -> Vec<String> {
    common::kit_index().pieces.into_iter().map(|p| p.piece).collect()
}

#[test]
fn two_hallways_join_end_to_end_through_a_portal_the_width_of_the_doorway() {
    let kit = common::bake_kit("human_cs10_monotone");
    let h = kit[hallway_index(&kit_names())].clone();
    let mut w = NavWorld::new();
    let a = w.add_island(h.clone(), Turn(0), IVec3::ZERO);
    let b = w.add_island(h.clone(), Turn(0), IVec3::new(0, 0, 20_480));
    let c = w.connect(SocketRef { island: a, socket: 0 }, SocketRef { island: b, socket: 1 }, Some("door")).unwrap();
    let conn = &w.connections[c as usize];
    assert_eq!(conn.gap_mm, 0);
    assert!(!conn.links.is_empty() && conn.links.len().is_multiple_of(2), "portals come in pairs, one each way");
    let width: i32 = conn.links.iter().step_by(2).map(|&l| {
        let s = &w.links[l as usize].from_segment;
        (s[0] - s[1]).abs().max_element()
    }).sum();
    let span = |isl: u32, s: u16| { let k = w.socket(SocketRef { island: isl, socket: s }).unwrap(); (k.b - k.a).abs().max_element() };
    assert_eq!(width, span(a, 0).min(span(b, 1)));
    let s = &w.links[conn.links[0] as usize].from_segment;
    assert!(s[0].x > s[1].x);

    let mut w2 = NavWorld::new();
    let a2 = w2.add_island(h.clone(), Turn(0), IVec3::ZERO);
    let b2 = w2.add_island(h.clone(), Turn(0), IVec3::new(0, 0, 21_000));
    assert!(matches!(w2.connect(SocketRef { island: a2, socket: 0 }, SocketRef { island: b2, socket: 1 }, None), Err(JoinError::TooFar { .. })));
    assert!(matches!(w2.connect(SocketRef { island: a2, socket: 0 }, SocketRef { island: b2, socket: 0 }, None), Err(JoinError::NotFacing { .. })));
}

#[test]
fn a_closed_door_is_impassable_not_expensive_and_says_so() {
    let kit = common::bake_kit("human_cs10_monotone");
    let h = kit[hallway_index(&kit_names())].clone();
    let mut w = NavWorld::new();
    let a = w.add_island(h.clone(), Turn(0), IVec3::ZERO);
    let b = w.add_island(h.clone(), Turn(0), IVec3::new(0, 0, 20_480));
    let c = w.connect(SocketRef { island: a, socket: 0 }, SocketRef { island: b, socket: 1 }, Some("door")).unwrap();
    let pa = PolyRef { island: a, poly: 0 };
    let pb = PolyRef { island: b, poly: 0 };
    assert!(w.same_component(pa, pb));
    let gate = w.connections[c as usize].gate.unwrap();
    let v0 = w.version();
    assert_eq!(w.set_gate(gate, false), Some(NavEvent::GateChanged { gate, open: false, version: v0 + 1 }));
    assert_eq!(w.set_gate(gate, false), None, "no event when nothing changed");
    assert!(!w.same_component(pa, pb));
    assert!(w.room_graph().route(&w, a, b).is_none());
    for p in 0..h.polys.len() as u16 {
        assert!(w.neighbours(PolyRef { island: a, poly: p }).all(|(n, _)| n.island == a));
    }
    w.set_gate(gate, true);
    assert!(w.same_component(pa, pb));
    assert_eq!(w.room_graph().route(&w, a, b).map(|r| r.0), Some(vec![c]));
    assert_eq!(w.drain_events().len(), 2);
}

#[test]
fn typed_links_attach_to_the_polygons_at_their_ends_and_follow_the_piece() {
    let kit = common::bake_kit("human_cs10_monotone");
    let mut h = (*kit[hallway_index(&kit_names())]).clone();
    h.links = vec![NavLink {
        kind: TypedKind::Vent,
        start: [IVec3::new(-300, 0, -8000), IVec3::new(300, 0, -8000)],
        end: [IVec3::new(-300, 0, 8000), IVec3::new(300, 0, 8000)],
        bidirectional: false,
        cost_mm: 4000,
        enabled: true,
    }];
    let h = Arc::new(h);
    for turn in 0..4 {
        let mut w = NavWorld::new();
        let off = IVec3::new(50_000, 0, -7_000);
        let a = w.add_island(h.clone(), Turn(turn), off);
        assert_eq!(w.links.len(), 1, "one way: one link");
        let l = &w.links[0];
        assert_eq!(l.kind, LinkKind::Typed(TypedKind::Vent));
        assert_eq!(l.from, w.locate(Turn(turn).apply(IVec3::new(0, 0, -8000)) + off).unwrap());
        assert_eq!(l.to, w.locate(Turn(turn).apply(IVec3::new(0, 0, 8000)) + off).unwrap());
        assert_eq!(l.from.island, a);
        let from = l.from;
        assert!(w.set_link_enabled(0, false).is_some());
        assert_eq!(w.neighbours(from).filter(|(_, via)| via.is_some()).count(), 0);
    }
}

/// The whole level as one connected world, the way the game will load it.
#[test]
fn every_generated_level_assembles_with_every_doorway_joined_and_one_connected_floor() {
    let kit = common::bake_kit("human_cs10_monotone");
    for layout in common::layouts() {
        let t = std::time::Instant::now();
        let mut w = common::assemble(&kit, &layout, 0);
        let ms = t.elapsed().as_secs_f64() * 1000.0;

        let is_cap = |s: SocketRef| w.islands[s.island as usize].asset.polys.is_empty();
        let issues = w.check_joins();
        let missed: Vec<_> = issues.iter().filter(|i| matches!(i, JoinIssue::MissedJoin(..))).collect();
        let unjoined = issues.iter().filter(|i| matches!(i, JoinIssue::Unjoined(..))).count();
        let impassable: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i, JoinIssue::Impassable(c) if { let c = &w.connections[*c as usize]; !is_cap(c.a) && !is_cap(c.b) }))
            .collect();
        assert!(missed.is_empty(), "seed {}: doorways that meet but are not joined: {missed:?}", layout.seed);
        assert!(impassable.is_empty(), "seed {}: joins a human cannot pass: {impassable:?}", layout.seed);
        assert_eq!(unjoined, layout.unsealed, "seed {}", layout.seed);

        let start = w.connections.iter().filter_map(|c| c.links.first()).map(|&l| w.links[l as usize].from).next().unwrap();
        let portal_polys: Vec<PolyRef> = w.links.iter().map(|l| l.from).collect();
        let cut_off = portal_polys.iter().filter(|&&p| !w.same_component(start, p)).count();
        assert_eq!(cut_off, 0, "seed {}: {cut_off} doorway polygons are cut off from the rest", layout.seed);

        let gaps = w.connections.iter().filter(|c| c.gap_mm.abs() > 1).count();
        let max_gap = w.connections.iter().map(|c| c.gap_mm.abs()).max().unwrap();
        println!(
            "seed {:>2}: {} islands, {} polygons, {} joins ({} welded with a gap, widest {} mm), {} links, {} doors, assembled in {:.2} ms",
            layout.seed, w.islands.len(), w.poly_count(), w.connections.len(), gaps, max_gap, w.links.len(), w.gates.len(), ms
        );
    }
}

#[test]
fn turning_the_whole_level_changes_nothing_but_coordinates() {
    let kit = common::bake_kit("human_cs10_monotone");
    for layout in common::layouts().iter().take(3) {
        let base = common::assemble(&kit, layout, 0);
        let widths = |w: &NavWorld| -> Vec<i32> {
            w.links.iter().map(|l| (l.from_segment[0] - l.from_segment[1]).abs().max_element()).collect()
        };
        for turn in 1..4 {
            let w = common::assemble(&kit, layout, turn);
            assert_eq!(w.links.len(), base.links.len(), "seed {} turn {turn}", layout.seed);
            assert_eq!(widths(&w), widths(&base), "seed {} turn {turn}: portal widths changed", layout.seed);
            for (a, b) in base.islands.iter().zip(&w.islands) {
                assert!(a.vertices.iter().zip(&b.vertices).all(|(&p, &q)| Turn(turn).apply(p) == q));
            }
        }
    }
}

#[test]
fn closing_doors_agrees_between_the_floor_and_the_room_graph() {
    let kit = common::bake_kit("human_cs10_monotone");
    let layout = &common::layouts()[0];
    let mut w = common::assemble(&kit, layout, 0);
    let graph = w.room_graph();
    let gated: Vec<u32> = (0..w.connections.len() as u32).filter(|&c| w.connections[c as usize].gate.is_some()).collect();
    let total = gated.len();
    let mut splits = 0;
    for c in gated {
        let conn = w.connections[c as usize].clone();
        let (pa, pb) = (w.links[conn.links[0] as usize].from, w.links[conn.links[0] as usize].to);
        w.set_gate(conn.gate.unwrap(), false);
        let split = !w.same_component(pa, pb);
        let routed = graph.route(&w, conn.a.island, conn.b.island).is_some();
        assert_eq!(split, !routed, "join {c}: the polygons and the room graph disagree about the closed door");
        splits += usize::from(split);
        w.set_gate(conn.gate.unwrap(), true);
        assert!(w.same_component(pa, pb));
    }
    assert!(splits > 0 && splits < total, "{splits} of {total} doors split the level");
    println!("{splits} of {total} doors split seed {}'s level when closed", layout.seed);
}

#[test]
fn the_large_class_is_told_which_doorways_it_cannot_pass() {
    let kit = common::bake_kit("large_cs10_layers");
    let layout = &common::layouts()[0];
    let w = common::assemble(&kit, layout, 0);
    let blocked = w.check_joins().iter().filter(|i| matches!(i, JoinIssue::Impassable(_))).count();
    println!("large monster: {blocked} of {} joins impassable in seed {}", w.connections.len(), layout.seed);
    assert!(blocked > 0);
}

#[test]
fn agents_walk_across_every_welded_doorway_in_small_steps() {
    let kit = common::bake_kit("human_cs10_monotone");
    let (mut crossed, mut gaps) = (0, 0);
    for layout in common::layouts().iter().take(4) {
        let w = common::assemble(&kit, layout, 0);
        for c in &w.connections {
            if c.links.is_empty() {
                continue;
            }
            if c.gap_mm.abs() > 5 {
                gaps += 1;
            }
            for (from, to) in [(c.a, c.b), (c.b, c.a)] {
                let s = w.socket(from).unwrap();
                let out = s.outward.vec().as_dvec3();
                let mid = (s.a + s.b).as_dvec3() / 2.0;
                let (start, goal) = (mid - out * 400.0, mid + out * (400.0 + f64::from(c.gap_mm.max(0))));
                let ext = DVec3::new(100.0, 500.0, 100.0);
                let Some(n) = w.nearest_poly_in(from.island, start, ext) else { continue };
                let (mut pos, mut poly) = (n.point, n.poly);
                for _ in 0..200 {
                    let d = DVec3::new(goal.x - pos.x, 0.0, goal.z - pos.z);
                    if d.length() < 5.0 {
                        break;
                    }
                    (pos, poly) = w.slide(poly, pos, pos + d.normalize() * d.length().min(20.0));
                }
                assert!(
                    DVec3::new(goal.x - pos.x, 0.0, goal.z - pos.z).length() < 30.0 && poly.island == to.island,
                    "seed {} join {from:?} -> {to:?} (gap {} mm): stopped at {pos:?} on island {}",
                    layout.seed, c.gap_mm, poly.island
                );
                crossed += 1;
            }
        }
    }
    eprintln!("walked across {crossed} doorway crossings, {gaps} of the joins welded with a gap");
    assert!(gaps > 0, "no welded gaps to test");
}
