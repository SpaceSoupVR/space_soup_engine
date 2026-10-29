//! Steps a scene with one player standing by a door and prints what the door does: a check of
//! the automatic doors and locks the logic drives (`gameplay`).
//!
//!   cargo run --release --example door_probe -- <game dir> <scene> <door id> [x y z of the head]
use std::collections::HashMap;
use glam::{Quat, Vec3};
use space_soup_engine::rig::{JointId, PlayerRig, Transform};
use space_soup_engine::runtime::{GameRuntime, PlayerFrameInput};
use space_soup_protocol::PlayerId;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (game, scene, door) = (&args[0], &args[1], &args[2]);
    let mut rt = GameRuntime::load(std::path::Path::new(game)).expect("game");
    rt.load_scene(scene).expect("scene");
    let geom = rt.physics().doors().find(|(id, _)| *id == door.as_str()).map(|(_, g)| g.closed_pos).expect("door id");
    let head = if args.len() >= 6 {
        Vec3::new(args[3].parse().unwrap(), args[4].parse().unwrap(), args[5].parse().unwrap())
    } else {
        geom + Vec3::new(0.0, 0.3, 1.0)
    };
    println!("door {door} closed at {geom:?}; head at {head:?}; def {:?}", rt.scene().find_object(door).and_then(|o| o.door.as_ref()).map(|d| (d.auto.clone(), d.locked.clone())));
    let player = PlayerId::new();
    for frame in 0..180 {
        let mut rig = PlayerRig::new();
        rig.joints.insert(JointId::Head, Transform { position: head, rotation: Quat::IDENTITY });
        let mut inputs = HashMap::new();
        inputs.insert(player, PlayerFrameInput { rig, ..Default::default() });
        if frame == 90 {
            if let Ok(kv) = std::env::var("PROBE_SETVAR") {
                if let Some((k, v)) = kv.split_once('=') {
                    println!("set_var {k}={v}");
                    rt.set_var(k, v);
                }
            }
        }
        rt.update(1.0 / 60.0, &inputs);
        if frame % 20 == 0 {
            let angle = rt.physics().door_angle(door).map(f32::to_degrees);
            let pos = rt.scene().find_object(door).map(|o| o.cuboid.position);
            println!("t {:.2}s angle {:?} pos {:?} player hp {:?}", frame as f32 / 60.0, angle, pos, rt.gameplay().players.get(&player).map(|p| p.health));
        }
    }
}
