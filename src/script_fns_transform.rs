use rhai::{Dynamic, Engine};

use crate::script::{EngineCommand, SharedContext};

pub(crate) fn register_transform_and_scene_fns(engine: &mut Engine, context: &SharedContext) {
    {
        let ctx = context.clone();
        engine.register_fn("move_object", move |id: &str, x: f64, y: f64, z: f64| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::MoveObject {
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
            "rotate_object",
            move |id: &str, x: f64, y: f64, z: f64, w: f64| {
                ctx.lock()
                    .unwrap()
                    .commands
                    .push(EngineCommand::RotateObject {
                        id: id.to_string(),
                        x: x as f32,
                        y: y as f32,
                        z: z as f32,
                        w: w as f32,
                    });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("scale_object", move |id: &str, x: f64, y: f64, z: f64| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::ScaleObject {
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
            "set_color",
            move |id: &str, r: i64, g: i64, b: i64, a: i64| {
                ctx.lock().unwrap().commands.push(EngineCommand::SetColor {
                    id: id.to_string(),
                    r: r.clamp(0, 255) as u8,
                    g: g.clamp(0, 255) as u8,
                    b: b.clamp(0, 255) as u8,
                    a: a.clamp(0, 255) as u8,
                });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("play_animation", move |id: &str, anim: &str| {
            ctx.lock().unwrap().commands.push(EngineCommand::PlayAnim {
                id: id.to_string(),
                anim: anim.to_string(),
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("trigger", move |id: &str, anim: &str| {
            ctx.lock().unwrap().commands.push(EngineCommand::PlayAnim {
                id: id.to_string(),
                anim: anim.to_string(),
            });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn(
            "play_part_animation",
            move |id: &str, clip: &str, blend: f64| {
                ctx.lock().unwrap().commands.push(EngineCommand::SetPartBlend {
                    id: id.to_string(),
                    clip: clip.to_string(),
                    blend: blend as f32,
                });
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("stop_animation", move |id: &str| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::StopAnim { id: id.to_string() });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("change_scene", move |scene: &str| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::ChangeScene {
                    scene: scene.to_string(),
                });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("destroy_object", move |id: &str| {
            ctx.lock()
                .unwrap()
                .commands
                .push(EngineCommand::DestroyObject { id: id.to_string() });
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("set_var", move |key: &str, value: Dynamic| {
            ctx.lock().unwrap().vars.insert(key.to_string(), value);
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_var", move |key: &str| -> Dynamic {
            ctx.lock()
                .unwrap()
                .vars
                .get(key)
                .cloned()
                .unwrap_or(Dynamic::UNIT)
        });
    }

}
