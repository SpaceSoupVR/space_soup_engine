//! The phase-5 tools on the real kit and levels: the bake checker finds what is really there, the
//! level report agrees with the world, and debug drawing covers what it should.

mod common;

use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::{LinkKind, NavLink};
use space_soup_nav::crowd::{AgentParams, Crowd, CrowdSettings, Goal};
use space_soup_nav::debug::{DebugDraw, FloorColour, Layer};
use space_soup_nav::path::find_path;
use space_soup_nav::validate::{check_kit, check_level, check_piece, Severity};
use space_soup_nav::world::{LinkKind as WorldLinkKind, PolyRef};

fn piece<'a>(kit: &'a [Arc<space_soup_nav::asset::NavPieceAsset>], name: &str) -> &'a Arc<space_soup_nav::asset::NavPieceAsset> {
    kit.iter().find(|a| a.name == name).unwrap()
}

#[test]
fn the_piece_checker_finds_floor_no_doorway_reaches() {
    let kit = common::bake_kit("human_cs10_monotone");
    let vent = check_piece(piece(&kit, "VentLockRoom"));
    assert!(vent.iter().any(|i| i.severity == Severity::Warning && i.message.contains("cannot be reached") && i.at.is_some_and(|p| p.y > 3000.0)), "{vent:?}");
    let catwalk = check_piece(piece(&kit, "4WayTallWCatwalk_DoorType_A"));
    assert!(catwalk.iter().any(|i| i.message.contains("cannot be reached")), "{catwalk:?}");
    assert!(check_piece(piece(&kit, "HallwayShort_DoorType_A")).iter().all(|i| i.severity == Severity::Info));
    assert!(check_piece(piece(&kit, "DoorType_A_NO_PATH")).iter().all(|i| i.severity == Severity::Info));
}

#[test]
fn the_piece_checker_catches_a_link_that_lands_nowhere_and_one_that_reaches_the_floor_above() {
    let kit = common::bake_kit("human_cs10_monotone");
    let vent_room = piece(&kit, "VentLockRoom");
    let up_floor = check_piece(vent_room).iter().find(|i| i.message.contains("86") || i.at.is_some_and(|p| p.y > 3000.0)).and_then(|i| i.at).unwrap();
    let ground = vent_room.vertices.iter().find(|v| v.y < 500).copied().unwrap();
    let mut fixed = (**vent_room).clone();
    let rung = |p: IVec3| [p + IVec3::new(-200, 0, 0), p + IVec3::new(200, 0, 0)];
    let ground_spot = vent_room.polys.iter().find(|p| p.verts.iter().all(|&v| vent_room.vertices[v as usize].y < 500)).map(|p| {
        let s: IVec3 = p.verts.iter().map(|&v| vent_room.vertices[v as usize]).sum();
        s / p.verts.len() as i32
    }).unwrap_or(ground);
    fixed.links = vec![
        NavLink { kind: LinkKind::Ladder, start: rung(ground_spot), end: rung(up_floor.as_ivec3()), bidirectional: true, cost_mm: 2000, enabled: true },
        NavLink { kind: LinkKind::Vent, start: rung(ground_spot), end: rung(IVec3::new(90_000, 0, 90_000)), bidirectional: false, cost_mm: 0, enabled: true },
    ];
    let issues = check_piece(&Arc::new(fixed));
    assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("Vent link 1: its end is not on any floor")), "{issues:?}");
    assert!(!issues.iter().any(|i| i.message.contains("cannot be reached") && i.at.is_some_and(|p| (p - up_floor).length() < 2000.0)), "{issues:?}");
}

#[test]
fn the_kit_checker_names_the_narrowest_doorways() {
    let kit = common::bake_kit("human_cs10_monotone");
    let issues = check_kit(&kit);
    assert!(issues.iter().any(|i| i.message.contains("LIGHT_TEST_3WayTall_DoorType_A")), "{issues:?}");
}

#[test]
fn the_level_report_agrees_with_the_world() {
    for class in ["human_cs10_monotone", "large_cs10_layers"] {
        let kit = common::bake_kit(class);
        for layout in common::layouts().iter().take(3) {
            let mut w = common::assemble(&kit, layout, 0);
            let r = check_level(&mut w, PolyRef { island: 0, poly: 0 });
            assert_eq!(r.rooms, w.islands.iter().filter(|i| !i.asset.polys.is_empty()).count());
            assert_eq!(r.reachable_rooms, r.rooms, "{class} seed {}: {}", layout.seed, r.to_markdown(""));
            assert!(r.impassable_joins.is_empty());
            assert_eq!(r.missed_joins, 0);
            assert_eq!(r.open_doorways, layout.unsealed);
            assert!(!r.cutting_doors.is_empty() && r.cutting_doors.len() < r.doors);
            assert!(w.gates.iter().all(|g| g.open));
            assert!(w.drain_events().is_empty());
        }
    }
}

#[test]
fn debug_drawing_covers_the_world_paths_and_crowds() {
    let kit = common::bake_kit("human_cs10_monotone");
    let layout = &common::layouts()[0];
    let mut w = common::assemble(&kit, layout, 0);
    let d = DebugDraw::world(&mut w, FloorColour::ByArea);
    let floor_tris: usize = w.islands.iter().enumerate().map(|(ii, isl)| (0..isl.asset.polys.len()).map(|p| w.floor_triangles(PolyRef { island: ii as u32, poly: p as u16 }).len()).sum::<usize>()).sum();
    assert_eq!(d.tris.iter().filter(|t| t.layer == Layer::Floor).count(), floor_tris);
    let portals = w.links.iter().filter(|l| l.kind == WorldLinkKind::Portal).count();
    assert_eq!(d.lines.iter().filter(|l| l.layer == Layer::Doorways).count(), portals);
    assert!(d.lines.iter().any(|l| l.layer == Layer::Walls));
    let gate = w.gates.iter().position(|g| !g.links.is_empty()).unwrap() as u32;
    w.set_gate(gate, false);
    let shut = DebugDraw::world(&mut w, FloorColour::ByHeight);
    assert!(shut.lines.iter().any(|l| l.colour == space_soup_nav::debug::Rgba::DOOR_SHUT));

    let mut d = d;
    let ext = DVec3::new(500.0, 2000.0, 500.0);
    let a = w.poly_dverts(PolyRef { island: 0, poly: 0 })[0];
    let b = w.islands[30].vertices[0].as_dvec3();
    if let Some(p) = find_path(&w, a, b, ext) {
        d.add_path(&p, space_soup_nav::debug::Rgba::PATH);
        assert!(d.lines.iter().any(|l| l.layer == Layer::Paths));
    }
    let mut crowd = Crowd::new(&w, CrowdSettings::default());
    let id = crowd.add_agent(AgentParams { radius_mm: 400.0, max_speed_mm_s: 3000.0, priority: 0 }, w.nearest_poly(a, ext).unwrap().point);
    crowd.set_goal(id, Goal::Point(w.nearest_poly(b, ext).unwrap().point));
    crowd.update(&w, &w.room_graph(), 1.0 / 72.0);
    d.add_crowd(&crowd);
    d.add_chokes(&crowd);
    d.add_problem(a, "test");
    assert_eq!(d.lines.iter().filter(|l| l.layer == Layer::Agents).count(), 16 + 2, "one body circle, a target line, a velocity line");
    let svg = d.to_svg(10.0, "seed 1");
    assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
    assert!(svg.contains("<polygon") && svg.contains("<line") && svg.contains(">test</text>"));
}

#[test]
fn a_player_on_the_catwalk_draws_a_monster_to_the_floor_beneath_it() {
    for class in ["human_cs10_monotone", "large_cs10_layers"] {
        let kit = common::bake_kit(class);
        let room = piece(&kit, "4WayTallWCatwalk_DoorType_A");
        let up = check_piece(room).iter().filter(|i| i.message.contains("cannot be reached")).filter_map(|i| i.at).max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
        let mut w = space_soup_nav::world::NavWorld::new();
        w.add_island(room.clone(), space_soup_nav::asset::Turn(0), IVec3::ZERO);
        let door = room.sockets.iter().find(|s| !s.edges.is_empty()).unwrap();
        let start = (door.a + door.b).as_dvec3() / 2.0;
        let p = find_path(&w, start, up, DVec3::new(1500.0, 300.0, 1500.0)).expect("start and goal are on the mesh");
        assert!(!p.complete, "{class}: the catwalk is reachable after all");
        let end = p.waypoints.last().unwrap().pos();
        let below = w.nearest_poly(DVec3::new(up.x, 100.0, up.z), DVec3::new(4000.0, 500.0, 4000.0)).unwrap().point;
        assert!(end.y < up.y - 1500.0 && (end - below).length() < 1.0, "{class}: ends at {end:?}, nearest floor {below:?}");
    }
}
