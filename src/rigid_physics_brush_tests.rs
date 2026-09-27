//! Brushes as physical barriers. A room whose walls can be walked through is
//! not a room.

use std::path::Path;

use glam::Vec3;

use crate::brush::{block_solid, BrushDef};
use crate::locomotion::{Locomotion, LocomotionInput, LocomotionMode, TeleportTarget};
use crate::rig::PlayerRig;
use crate::rigid_physics::PhysicsWorld;
use crate::rigid_physics_spawn::brush_collision_triangles;
use crate::runtime_test_support::PHYSX_TEST_LOCK;
use crate::scene::{GameObject, Scene};

/// A closed 6 m box with a 5.6 m room carved out of it: walls 0.3 m thick,
/// floor at y = 0.
fn sealed_room() -> BrushDef {
    BrushDef {
        solids: vec![block_solid([-3.0, -0.3, -3.0], [3.0, 3.2, 3.0], "m")],
        subtract: vec![block_solid([-2.7, 0.0, -2.7], [2.7, 2.8, 2.7], "m")],
    }
}

fn room_object(solid: Option<bool>) -> GameObject {
    GameObject {
        id: "room".into(),
        brush: Some(sealed_room()),
        solid,
        ..Default::default()
    }
}

fn world_with(objects: Vec<GameObject>) -> PhysicsWorld {
    let mut scene: Scene = serde_json::from_str(r#"{"name":"brush_test","objects":[]}"#).unwrap();
    scene.objects = objects;
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, Path::new("."));
    physics
}

/// Walk a player in a straight line in 5 cm steps, through the same collision
/// the headset and the server both run every frame.
fn walk(physics: &PhysicsWorld, from: Vec3, to: Vec3) -> Vec3 {
    walk_with_head(physics, from, to, Vec3::ZERO)
}

/// Where a headset reports a head standing `head` from the rig origin: in world
/// space, built from this locomotion's own offset.
fn head_at(loco: &Locomotion, head: Vec3) -> Option<Vec3> {
    Some(loco.player_offset + head + Vec3::Y * 1.7)
}

/// The same walk, by a player standing `head` away from their play area's
/// centre. Moves the RIG ORIGIN from `from` to `to` and returns where it ended.
fn walk_with_head(physics: &PhysicsWorld, from: Vec3, to: Vec3, head: Vec3) -> Vec3 {
    let mut loco = Locomotion::new(LocomotionMode::Smooth);
    loco.player_offset = from;
    let steps = ((to - from).length() / 0.05).ceil() as usize;
    for i in 1..=steps {
        let start = loco.collision_start(head_at(&loco, head));
        let target = from + (to - from) * (i as f32 / steps as f32);
        loco.player_offset.x = target.x;
        loco.player_offset.z = target.z;
        loco.apply_collision(physics, start);
    }
    loco.player_offset
}

#[test]
fn collision_triangles_face_out_of_the_solid() {
    let def = BrushDef {
        solids: vec![block_solid([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0], "m")],
        subtract: vec![],
    };
    let (p, idx) = brush_collision_triangles(&def);
    assert_eq!(idx.len() / 3, 12, "a box is six quads");
    for t in idx.chunks_exact(3) {
        let [a, b, c] = [p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]].map(Vec3::from);
        let n = (b - a).cross(c - a);
        let centroid = (a + b + c) / 3.0;
        assert!(n.dot(centroid) > 0.0, "triangle {t:?} faces into the solid");
    }
}

#[test]
fn a_brush_room_stops_a_player_walking_at_its_wall() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = world_with(vec![room_object(None)]);
    assert!(physics.has_collider("room"), "a brush is solid by default");
    let end = walk(&physics, Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
    assert!(end.x < 2.7, "walked through the wall, to x = {}", end.x);
    assert!(end.x > 2.0, "stopped at x = {}, well short of the wall", end.x);
}

#[test]
fn a_brush_marked_not_solid_is_walked_through() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = world_with(vec![room_object(Some(false))]);
    assert!(!physics.has_collider("room"));
    let end = walk(&physics, Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
    assert!(end.x > 4.9, "stopped at x = {} by a brush marked solid: false", end.x);
}

/// A one-sided mesh wound the wrong way is solid from exactly one side, so both
/// sides are asked.
#[test]
fn the_wall_is_solid_from_inside_the_room_and_from_outside() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = world_with(vec![room_object(None)]);
    let (inside, n_in) = physics
        .raycast(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0)
        .expect("a ray from inside the room passed through the wall");
    assert!((inside.x - 2.7).abs() < 1e-3, "inside hit at {inside:?}");
    assert!(n_in.x < -0.99, "inside hit normal {n_in:?} should face into the room");
    let (outside, _) = physics
        .raycast(Vec3::new(6.0, 1.0, 0.0), -Vec3::X, 10.0)
        .expect("a ray from outside passed through the wall");
    assert!((outside.x - 3.0).abs() < 1e-3, "outside hit at {outside:?}");
    let (floor, n_floor) = physics
        .raycast_down(Vec3::new(0.0, 2.0, 0.0), 10.0)
        .expect("nothing to stand on inside the room");
    assert!(floor.y.abs() < 1e-3 && n_floor.y > 0.99, "floor hit {floor:?} {n_floor:?}");
}

#[test]
fn a_trigger_is_never_a_wall_unless_someone_says_so() {
    let mut zone = room_object(None);
    assert!(zone.is_solid_brush());
    zone.is_trigger = true;
    assert!(!zone.is_solid_brush(), "an is_trigger brush became a wall");
    zone.solid = Some(true);
    assert!(zone.is_solid_brush(), "an explicit solid: true was ignored");

    let mut volume = room_object(None);
    volume.trigger_volume = Some(serde_json::from_str("{}").expect("an empty trigger volume parses"));
    assert!(!volume.is_solid_brush(), "a trigger_volume brush became a wall");

    let not_a_brush = GameObject { id: "crate".into(), solid: Some(true), ..Default::default() };
    assert!(!not_a_brush.is_solid_brush(), "only brushes build a collider from `solid`");
}

#[test]
fn an_authored_rigid_body_on_a_brush_keeps_its_own_shape() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut obj = room_object(None);
    obj.cuboid.position = Vec3::ZERO;
    obj.cuboid.half_size = Vec3::splat(0.5);
    obj.rigid_body = Some(serde_json::from_str(r#"{"mode":"Static","shape":"Box"}"#).unwrap());
    let physics = world_with(vec![obj]);
    assert!(physics.has_collider("room"));
    assert!(
        physics.raycast(Vec3::new(1.0, 1.0, 0.0), Vec3::X, 10.0).is_none(),
        "the brush's own wall collides too: two actors under one id"
    );
}

#[test]
fn breaching_a_solid_brush_removes_its_collider() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut physics = world_with(vec![room_object(None)]);
    let ray = || physics_ray(&physics);
    assert!(ray(), "the intact wall should stop a ray");
    assert!(physics.despawn_static("room"), "the brush collider was not in `statics`");
    assert!(!physics.has_collider("room"));
    assert!(!physics_ray(&physics), "the breached wall still stops a ray");
}

fn physics_ray(physics: &PhysicsWorld) -> bool {
    physics.raycast(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0).is_some()
}

/// THE REAL LEVEL: you get into the hall by its door, and nowhere else.
#[test]
fn the_shipped_hall_is_entered_by_its_door_and_not_through_its_walls() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let game = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
    let scene = Scene::load(&game.join("scenes/test_room.json")).expect("test_room.json loads");
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, game);
    for id in ["hall_shell", "hall_pillar", "brick_hall"] {
        assert!(physics.has_collider(id), "'{id}' is not solid");
    }

    // The doorway spans x -0.8..0.8 at floor level through the z = 3.7..4.0 wall.
    let entered = walk(&physics, Vec3::new(0.0, 0.0, 8.0), Vec3::new(0.0, 0.0, -3.0));
    assert!(entered.z < -2.9, "the doorway did not let the player in: stopped at {entered:?}");
    assert!(entered.y.abs() < 0.01, "not standing on the hall floor: {entered:?}");

    let beside = walk(&physics, Vec3::new(2.0, 0.0, 8.0), Vec3::new(2.0, 0.0, -3.0));
    assert!(beside.z > 4.0, "walked through the hall's front wall: {beside:?}");

    let side = walk(&physics, Vec3::new(0.0, 0.0, -10.0), Vec3::new(6.0, 0.0, -10.0));
    assert!(side.x < 2.7 && side.x > 2.0, "the side wall did not stop the player: {side:?}");
}

/// On a headset the rig origin is the centre of the play area, and the player
/// is usually standing somewhere else in it. The camera is where the HEAD is, so
/// that is what a wall has to stop.
#[test]
fn a_player_standing_away_from_their_play_area_centre_is_stopped_at_the_wall() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = world_with(vec![room_object(None)]);
    let head = Vec3::new(1.5, 0.0, 0.0);
    let origin = walk_with_head(&physics, Vec3::new(-1.5, 0.0, 0.0), Vec3::new(3.5, 0.0, 0.0), head);
    let body = origin.x + head.x;
    assert!(body < 2.7, "the head went through the wall, to x = {body} (origin stopped at {})", origin.x);
    assert!(body > 2.0, "the head stopped at x = {body}, well short of the wall");
}

#[test]
fn walking_into_a_wall_in_the_real_room_pushes_the_rig_back() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = world_with(vec![room_object(None)]);
    let mut loco = Locomotion::new(LocomotionMode::Smooth);
    let mut head_x = 0.0;
    // Four metres of real walking toward +x, with the stick untouched.
    for _ in 0..80 {
        head_x += 0.05;
        let start = loco.collision_start(head_at(&loco, Vec3::new(head_x, 0.0, 0.0)));
        loco.apply_collision(&physics, start);
    }
    let body = loco.player_offset.x + head_x;
    assert!(body < 2.7, "walked through the wall in the real room, head at x = {body}");
    assert!(body > 2.0, "stopped at x = {body}, well short of the wall");
    assert!(loco.player_offset.x < -1.0, "the rig was not pushed back: origin at {}", loco.player_offset.x);
}

#[test]
fn a_teleport_is_not_stopped_by_the_walls_it_crosses() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let physics = world_with(vec![room_object(None)]);
    let mut loco = Locomotion::new(LocomotionMode::Teleport);
    let rig = PlayerRig::new();
    let press = LocomotionInput { teleport_pressed: true, ..LocomotionInput::default() };
    let release = LocomotionInput { teleport_released: true, ..LocomotionInput::default() };
    let target = TeleportTarget { position: Vec3::new(5.0, 0.0, 0.0), valid: true };
    loco.update(0.016, &press, &rig, None);
    let start = loco.collision_start(None);
    loco.update(0.016, &release, &rig, Some(target));
    loco.apply_collision(&physics, start);
    assert!((loco.player_offset.x - 5.0).abs() < 1e-4, "the teleport was blocked at x = {}", loco.player_offset.x);
}

/// A LOW ROOM KEEPS YOU ON ITS FLOOR. The ground probe used to start 3 m above
/// the feet, which is above the roof of any room lower than that: walking into
/// test_room's 2.6 m hallway stood the player on its roof (headset, 2026-09-24).
#[test]
fn a_room_with_a_low_ceiling_keeps_the_player_on_its_floor() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let low = GameObject {
        id: "low".into(),
        brush: Some(BrushDef {
            solids: vec![block_solid([-3.0, -0.3, -3.0], [3.0, 2.9, 3.0], "m")],
            subtract: vec![block_solid([-2.7, 0.0, -2.7], [2.7, 2.6, 2.7], "m")],
        }),
        ..Default::default()
    };
    let physics = world_with(vec![low]);
    let end = walk(&physics, Vec3::ZERO, Vec3::new(1.5, 0.0, 0.0));
    assert!(end.y.abs() < 0.01, "stood on the roof, at y = {}", end.y);
}

/// THE REAL LEVEL: from the hall, through its side door, along the hallway and
/// into the brick room -- on the floor all the way.
#[test]
fn the_shipped_hallway_is_walked_through_on_its_floor() {
    let _guard = PHYSX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let game = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../game"));
    let scene = Scene::load(&game.join("scenes/test_room.json")).expect("test_room.json loads");
    let mut physics = PhysicsWorld::new();
    physics.rebuild(&scene, game);
    assert!(physics.has_collider("hallway"), "the hallway is not solid");

    let mut loco = Locomotion::new(LocomotionMode::Smooth);
    loco.player_offset = Vec3::new(0.0, 0.0, -3.0);
    let to = Vec3::new(14.0, 0.0, -3.0);
    let from = loco.player_offset;
    let mut highest = 0.0f32;
    for i in 1..=280 {
        let start = loco.collision_start(head_at(&loco, Vec3::ZERO));
        let target = from + (to - from) * (i as f32 / 280.0);
        loco.player_offset.x = target.x;
        loco.player_offset.z = target.z;
        loco.apply_collision(&physics, start);
        highest = highest.max(loco.player_offset.y);
    }
    assert!(loco.player_offset.x > 13.5, "stopped on the way, at {:?}", loco.player_offset);
    assert!(highest < 0.01, "left the floor on the way through, reaching y = {highest}");
}
