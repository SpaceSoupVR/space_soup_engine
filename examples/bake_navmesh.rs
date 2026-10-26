//! Bakes scenes' navmeshes from their colliders (`navigation`), into the game folder beside them.
//!
//!   cargo run --release --example `bake_navmesh` -- <game dir> [scene ...]
//!
//! With no scenes named, every scene in the manifest. Each is written to
//! `<game dir>/navmesh/scenes/<scene>.human.ssnav` with its `.json`, and the runtime loads it with
//! the scene. Bake again after moving any collider: a stale navmesh says so when it loads.

use std::path::PathBuf;

use space_soup_engine::manifest::Manifest;
use space_soup_engine::navigation::SceneNav;
use space_soup_engine::rigid_physics::PhysicsWorld;
use space_soup_engine::scene::Scene;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(game) = args.next().map(PathBuf::from) else {
        eprintln!("usage: bake_navmesh <game dir> [scene ...]");
        std::process::exit(2);
    };
    let mut scenes: Vec<String> = args.collect();
    if scenes.is_empty() {
        match Manifest::load(&game) {
            Ok(m) => scenes = m.scenes,
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
    }
    let mut failed = 0;
    for name in &scenes {
        let started = std::time::Instant::now();
        let result = Scene::load(&Manifest::scene_path(&game, name)).map_err(|e| format!("{e:#}")).and_then(|mut scene| {
            scene.resolve_world_transforms();
            let mut physics = PhysicsWorld::new();
            physics.rebuild(&scene, &game);
            let geometry = physics.static_collision_triangles();
            let nav = SceneNav::bake(&physics, name)?;
            nav.save(&game)?;
            Ok((nav, geometry.shapes))
        });
        match result {
            Ok((nav, shapes)) => println!(
                "{name}: {} polygons from {} triangles ({shapes:?}) at {} m cells, in {:.1} s",
                nav.info.polygons,
                nav.info.triangles,
                nav.info.cell_size,
                started.elapsed().as_secs_f32()
            ),
            Err(e) => {
                failed += 1;
                println!("{name}: not baked: {e}");
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
