//! Rides the blacksite start room's elevator: a player stands in the cab, a hand presses the
//! up button, then the down button; prints the cab's height, the landing leaves and the player's
//! feet as it goes.
//!
//!   cargo run --release --example elevator_probe -- <game dir> [scene]
use std::collections::HashMap;
use glam::{Quat, Vec3};
use space_soup_engine::events::Hand;
use space_soup_engine::rig::{JointId, PlayerRig, Transform};
use space_soup_engine::runtime::{GameRuntime, PlayerFrameInput};
use space_soup_protocol::PlayerId;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scene = args.get(1).map_or("blacksite_1", String::as_str);
    let mut rt = GameRuntime::load(std::path::Path::new(&args[0])).expect("game");
    rt.load_scene(scene).expect("scene");
    let at = |rt: &GameRuntime, id: &str| rt.scene().find_object(id).map(|o| o.cuboid.position).unwrap_or_else(|| panic!("no object {id}"));
    let (up, down) = (at(&rt, "elev_btn_cab_up"), at(&rt, "elev_btn_cab_down"));
    let player = PlayerId::new();
    let dt = 1.0 / 60.0;
    let mut stood = false;
    let mut press: Option<(f32, Vec3)> = None;
    for frame in 0..60 * 24 {
        let t = frame as f32 * dt;
        let cab = rt.physics().door_angle("elev_cab").unwrap_or(f32::NAN);
        // a press: 0.3 s with a hand at the button, the button riding with the cab
        press = match t {
            t if (3.0..3.3).contains(&t) => Some((t, up)),
            t if (14.0..14.3).contains(&t) => Some((t, down)),
            _ => None,
        };
        let feet = rt.locomotions.get(&player).map_or(Vec3::ZERO, |l| l.player_offset);
        let mut rig = PlayerRig::new();
        let head = Vec3::new(-3.95, feet.y + 1.7, 7.3);
        rig.joints.insert(JointId::Head, Transform { position: head, rotation: Quat::from_rotation_y(std::f32::consts::PI) });
        if let Some((_, button)) = press {
            rig.joints.insert(JointId::HandGrip(Hand::Right), Transform { position: Vec3::new(button.x, button.y + cab, button.z), rotation: Quat::IDENTITY });
            rig.joints.insert(JointId::HandGrip(Hand::Left), Transform { position: Vec3::new(0.0, 50.0, 0.0), rotation: Quat::IDENTITY });
        }
        let mut inputs = HashMap::new();
        inputs.insert(player, PlayerFrameInput { rig, ..Default::default() });
        rt.update(dt, &inputs);
        if !stood {
            rt.locomotions.get_mut(&player).unwrap().player_offset = Vec3::new(-3.95, 0.0, 7.3);
            stood = true;
        }
        if frame % 30 == 0 {
            let ang = |id: &str| rt.physics().door_angle(id).map_or(f32::NAN, |a| (a * 100.0).round() / 100.0);
            println!(
                "t {t:5.1}s cab {cab:5.2}  floor0 leaves {:5.2} {:5.2}  floor1 leaves {:5.2} {:5.2}  feet y {:5.2}",
                ang("elev_d0_l"), ang("elev_d0_r"), ang("elev_d1_l"), ang("elev_d1_r"), feet.y
            );
        }
    }
    let _ = press;
}
