use rhai::Engine;

use crate::script::{parse_hand, EngineCommand, SharedContext};

pub(crate) fn register_interaction_and_audio_fns(engine: &mut Engine, context: &SharedContext) {
    {
        let ctx = context.clone();
        engine.register_fn(
            "attach_to_joint",
            move |id: &str, joint: &str, ox: f64, oy: f64, oz: f64| {
                ctx.lock()
                    .unwrap()
                    .commands
                    .push(EngineCommand::AttachToJoint {
                        id: id.to_string(),
                        joint: joint.to_string(),
                        offset_x: ox as f32,
                        offset_y: oy as f32,
                        offset_z: oz as f32,
                    });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("grab_at_joint", move |id: &str, joint: &str| {
            let mut ctx = ctx.lock().unwrap();
            let player = ctx.current_player;
            ctx.commands.push(EngineCommand::GrabAtJoint {
                id: id.to_string(),
                joint: joint.to_string(),
                point: None,
                player,
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn(
            "grab_at_joint",
            move |id: &str, joint: &str, point: &str| {
                let mut ctx = ctx.lock().unwrap();
                let player = ctx.current_player;
                ctx.commands.push(EngineCommand::GrabAtJoint {
                    id: id.to_string(),
                    joint: joint.to_string(),
                    point: Some(point.to_string()),
                    player,
                });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("detach", move |id: &str| {
            let mut ctx = ctx.lock().unwrap();
            let player = ctx.current_player;
            ctx.commands.push(EngineCommand::Detach {
                id: id.to_string(),
                hand: None,
                player,
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("detach", move |id: &str, hand: &str| {
            let mut ctx = ctx.lock().unwrap();
            let player = ctx.current_player;
            ctx.commands.push(EngineCommand::Detach {
                id: id.to_string(),
                hand: Some(parse_hand(hand)),
                player,
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("grab_at_point", move |id: &str, point: &str, hand: &str| {
            let mut ctx = ctx.lock().unwrap();
            let player = ctx.current_player;
            ctx.commands.push(EngineCommand::GrabAtPoint {
                id: id.to_string(),
                point: point.to_string(),
                hand: parse_hand(hand),
                player,
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("release_grip", move |id: &str, hand: &str| {
            let mut ctx = ctx.lock().unwrap();
            let player = ctx.current_player;
            ctx.commands.push(EngineCommand::ReleaseGrip {
                id: id.to_string(),
                hand: parse_hand(hand),
                player,
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("play_sound", move |id: &str| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::PlaySound { id: id.to_string() });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("stop_sound", move |id: &str| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::StopSound { id: id.to_string() });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("set_light_intensity", move |id: &str, intensity: f64| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::SetLightIntensity {
                    id: id.to_string(),
                    intensity: intensity as f32,
                });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("spawn_particle_burst", move |id: &str, count: i64| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::SpawnParticleBurst {
                    id: id.to_string(),
                    count,
                });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn(
            "spawn_object",
            move |template_id: &str, new_id: &str, x: f64, y: f64, z: f64| {
                ctx.lock().unwrap().commands.push(EngineCommand::SpawnObject {
                    template_id: template_id.to_string(),
                    new_id: new_id.to_string(),
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("apply_impulse", move |id: &str, x: f64, y: f64, z: f64| {
            ctx.lock().unwrap().commands.push(EngineCommand::ApplyImpulse {
                id: id.to_string(),
                x: x as f32,
                y: y as f32,
                z: z as f32,
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn(
            "attach_to_socket",
            move |child_id: &str, parent_id: &str, socket: &str| {
                ctx.lock()
                    .unwrap()
                    .commands
                    .push(EngineCommand::AttachToSocket {
                        child_id: child_id.to_string(),
                        parent_id: parent_id.to_string(),
                        socket: socket.to_string(),
                    });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("detach_from_socket", move |child_id: &str| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::DetachFromSocket {
                    child_id: child_id.to_string(),
                });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("set_sound_pitch", move |id: &str, pitch: f64| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::SetSoundPitch {
                    id: id.to_string(),
                    pitch: pitch as f32,
                });
        });
    }

}
