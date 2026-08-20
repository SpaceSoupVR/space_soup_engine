//! Parent/child transform composition.
//!
//! The property under test is that a child's stored transform is
//! parent-relative and comes out in world space, because that is what makes
//! moving a compound prop one edit instead of N -- and because every other
//! module in the engine reads `cuboid.position` expecting world space.

use glam::{Quat, Vec3};

use crate::scene::{GameObject, Scene};

fn obj(id: &str, uuid: &str, parent: Option<&str>, pos: Vec3, rot: Quat) -> GameObject {
    let mut o = GameObject {
        id: id.into(),
        uuid: Some(uuid.into()),
        parent: parent.map(Into::into),
        ..Default::default()
    };
    o.cuboid.position = pos;
    o.cuboid.rotation = rot;
    o
}

fn scene(objects: Vec<GameObject>) -> Scene {
    Scene { name: "test".into(), objects, ..Default::default() }
}

#[test]
fn a_child_offset_is_relative_to_its_parent() {
    let mut s = scene(vec![
        obj("crate", "u-crate", None, Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY),
        obj("lid", "u-lid", Some("u-crate"), Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
    ]);
    s.resolve_world_transforms();

    assert_eq!(s.find_object("crate").unwrap().cuboid.position, Vec3::new(10.0, 0.0, 0.0));
    assert_eq!(s.find_object("lid").unwrap().cuboid.position, Vec3::new(10.0, 1.0, 0.0));
}

#[test]
fn a_parents_rotation_carries_its_children_around_it() {
    // 90 degrees about Y maps +X to -Z.
    let spin = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let mut s = scene(vec![
        obj("turret", "u-turret", None, Vec3::ZERO, spin),
        obj("barrel", "u-barrel", Some("u-turret"), Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
    ]);
    s.resolve_world_transforms();

    let barrel = s.find_object("barrel").unwrap().cuboid.position;
    assert!(
        (barrel - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5,
        "child should have swung with the parent, got {barrel:?}"
    );
}

#[test]
fn nesting_composes_through_every_level() {
    let mut s = scene(vec![
        obj("a", "u-a", None, Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
        obj("b", "u-b", Some("u-a"), Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
        obj("c", "u-c", Some("u-b"), Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
    ]);
    s.resolve_world_transforms();

    assert_eq!(s.find_object("c").unwrap().cuboid.position, Vec3::new(3.0, 0.0, 0.0));
}

#[test]
fn children_declared_before_their_parent_still_resolve() {
    // Document order is authoring order, not dependency order, and a migration
    // or a hand edit can easily put a child first.
    let mut s = scene(vec![
        obj("lid", "u-lid", Some("u-crate"), Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        obj("crate", "u-crate", None, Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY),
    ]);
    s.resolve_world_transforms();

    assert_eq!(s.find_object("lid").unwrap().cuboid.position, Vec3::new(10.0, 1.0, 0.0));
}

#[test]
fn a_flat_scene_is_left_exactly_alone() {
    // Every scene written before `parent` existed is this case.
    let before = scene(vec![
        obj("a", "u-a", None, Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY),
        obj("b", "u-b", None, Vec3::new(4.0, 5.0, 6.0), Quat::IDENTITY),
    ]);
    let mut after = before.clone();
    after.resolve_world_transforms();

    for (a, b) in before.objects.iter().zip(after.objects.iter()) {
        assert_eq!(a.cuboid.position, b.cuboid.position);
        assert_eq!(a.cuboid.rotation, b.cuboid.rotation);
    }
}

#[test]
fn an_object_with_no_uuid_cannot_be_a_parent_and_does_not_crash() {
    let mut s = scene(vec![
        GameObject { id: "old".into(), ..Default::default() },
        obj("orphan", "u-orphan", Some("u-missing"), Vec3::new(7.0, 0.0, 0.0), Quat::IDENTITY),
    ]);
    s.resolve_world_transforms();

    // A dangling parent is treated as a root: transform stays as authored, and
    // the scene still opens so it can be fixed in the editor.
    assert_eq!(s.find_object("orphan").unwrap().cuboid.position, Vec3::new(7.0, 0.0, 0.0));
}

#[test]
fn a_parent_cycle_is_survived_rather_than_hung() {
    let mut s = scene(vec![
        obj("a", "u-a", Some("u-b"), Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
        obj("b", "u-b", Some("u-a"), Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY),
    ]);
    s.resolve_world_transforms();

    assert_eq!(s.find_object("a").unwrap().cuboid.position, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(s.find_object("b").unwrap().cuboid.position, Vec3::new(2.0, 0.0, 0.0));
}

#[test]
fn an_object_parented_to_itself_is_survived() {
    let mut s = scene(vec![obj(
        "self", "u-self", Some("u-self"), Vec3::new(5.0, 0.0, 0.0), Quat::IDENTITY,
    )]);
    s.resolve_world_transforms();

    assert_eq!(s.find_object("self").unwrap().cuboid.position, Vec3::new(5.0, 0.0, 0.0));
}

#[test]
fn load_leaves_transforms_local_so_a_round_trip_is_lossless() {
    // Scene::load must NOT resolve: the runtime does that explicitly. If load
    // flattened, then load -> save would bake world coordinates into a file
    // whose parents are still declared, and every subsequent load would
    // compound the offset.
    let dir = std::env::temp_dir().join(format!("ss_hier_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.json");

    let authored = scene(vec![
        obj("crate", "u-crate", None, Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY),
        obj("lid", "u-lid", Some("u-crate"), Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
    ]);
    authored.save(&path).unwrap();

    let reloaded = Scene::load(&path).unwrap();
    assert_eq!(
        reloaded.find_object("lid").unwrap().cuboid.position,
        Vec3::new(0.0, 1.0, 0.0),
        "load must preserve the stored local offset"
    );

    std::fs::remove_dir_all(&dir).ok();
}
