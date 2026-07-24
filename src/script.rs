use anyhow::Result;
use rhai::{Dynamic, Engine, Scope, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use space_soup_protocol::PlayerId;

use crate::events::Hand;

fn parse_hand(s: &str) -> Hand {
    if s.eq_ignore_ascii_case("left") {
        Hand::Left
    } else {
        Hand::Right
    }
}

#[derive(Debug, Clone)]
pub enum EngineCommand {
    MoveObject {
        id: String,
        x: f32,
        y: f32,
        z: f32,
    },
    RotateObject {
        id: String,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    },
    ScaleObject {
        id: String,
        x: f32,
        y: f32,
        z: f32,
    },
    SetColor {
        id: String,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    },
    PlayAnim {
        id: String,
        anim: String,
    },
    StopAnim {
        id: String,
    },
    ChangeScene {
        scene: String,
    },
    DestroyObject {
        id: String,
    },
    AttachToJoint {
        id: String,
        joint: String,
        offset_x: f32,
        offset_y: f32,
        offset_z: f32,
    },
    GrabAtJoint {
        id: String,
        joint: String,
        point: Option<String>,
        player: PlayerId,
    },
    Detach {
        id: String,
        hand: Option<Hand>,
        player: PlayerId,
    },
    GrabAtPoint {
        id: String,
        point: String,
        hand: Hand,
        player: PlayerId,
    },
    ReleaseGrip {
        id: String,
        hand: Hand,
        player: PlayerId,
    },
    PlaySound {
        id: String,
    },
    StopSound {
        id: String,
    },
    SetLightIntensity {
        id: String,
        intensity: f32,
    },
    SetSoundPitch {
        id: String,
        pitch: f32,
    },
}

#[derive(Default)]
pub struct ScriptContext {
    pub commands: Vec<EngineCommand>,
    pub vars: HashMap<String, Dynamic>,
    pub object_positions: HashMap<String, (f32, f32, f32)>,
    pub rig_positions: HashMap<String, (f32, f32, f32)>,
    pub current_player: PlayerId,
}

pub type SharedContext = Arc<Mutex<ScriptContext>>;

pub struct ScriptHost {
    engine: Engine,
    asts: HashMap<String, AST>,
    context: SharedContext,
}

impl Default for ScriptHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptHost {
    pub fn new() -> Self {
        let context: SharedContext = Arc::new(Mutex::new(ScriptContext::default()));
        let engine = build_engine(context.clone());

        Self {
            engine,
            asts: HashMap::new(),
            context,
        }
    }

    pub fn context(&self) -> SharedContext {
        self.context.clone()
    }

    pub fn compile(&mut self, object_id: &str, source: &str) -> Result<()> {
        let ast = self
            .engine
            .compile(source)
            .map_err(|e| anyhow::anyhow!("script compile error in {object_id}: {e}"))?;
        self.asts.insert(object_id.to_string(), ast);
        Ok(())
    }

    pub fn has_script(&self, object_id: &str) -> bool {
        self.asts.contains_key(object_id)
    }

    pub fn call(&self, object_id: &str, fn_name: &str, args: impl rhai::FuncArgs) -> Result<()> {
        let Some(ast) = self.asts.get(object_id) else {
            return Ok(());
        };

        let mut scope = Scope::new();
        let result: Result<Dynamic, _> = self.engine.call_fn(&mut scope, ast, fn_name, args);

        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                if e.to_string().contains("Function not found") {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "script error in {object_id}::{fn_name}: {e}"
                    ))
                }
            }
        }
    }

    pub fn drain_commands(&self) -> Vec<EngineCommand> {
        let mut ctx = self.context.lock().unwrap();
        std::mem::take(&mut ctx.commands)
    }

    pub fn set_object_position(&self, id: &str, x: f32, y: f32, z: f32) {
        let mut ctx = self.context.lock().unwrap();
        ctx.object_positions.insert(id.to_string(), (x, y, z));
    }

    pub fn set_rig_position(&self, joint_name: &str, x: f32, y: f32, z: f32) {
        let mut ctx = self.context.lock().unwrap();
        ctx.rig_positions.insert(joint_name.to_string(), (x, y, z));
    }

    pub fn set_current_player(&self, player: PlayerId) {
        self.context.lock().unwrap().current_player = player;
    }
}

fn build_engine(context: SharedContext) -> Engine {
    let mut engine = Engine::new();

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

    {
        let ctx = context.clone();
        engine.register_fn("get_object_x", move |id: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .object_positions
                .get(id)
                .map(|p| p.0 as f64)
                .unwrap_or(0.0)
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_y", move |id: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .object_positions
                .get(id)
                .map(|p| p.1 as f64)
                .unwrap_or(0.0)
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_z", move |id: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .object_positions
                .get(id)
                .map(|p| p.2 as f64)
                .unwrap_or(0.0)
        });
    }

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

    {
        let ctx = context.clone();
        engine.register_fn("get_rig_x", move |joint: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .rig_positions
                .get(joint)
                .map(|p| p.0 as f64)
                .unwrap_or(0.0)
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_rig_y", move |joint: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .rig_positions
                .get(joint)
                .map(|p| p.1 as f64)
                .unwrap_or(0.0)
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_rig_z", move |joint: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .rig_positions
                .get(joint)
                .map(|p| p.2 as f64)
                .unwrap_or(0.0)
        });
    }

    engine
}

