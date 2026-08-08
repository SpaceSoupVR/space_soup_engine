use anyhow::Result;
use rhai::{Dynamic, Engine, Scope, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use space_soup_protocol::PlayerId;

use crate::events::{Hand, InputAxes};

pub(crate) fn parse_hand(s: &str) -> Hand {
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
    SetPartBlend {
        id: String,
        clip: String,
        blend: f32,
    },
    SpawnParticleBurst {
        id: String,
        count: i64,
    },
    SpawnObject {
        template_id: String,
        new_id: String,
        x: f32,
        y: f32,
        z: f32,
    },
    ApplyImpulse {
        id: String,
        x: f32,
        y: f32,
        z: f32,
    },
    AttachToSocket {
        child_id: String,
        parent_id: String,
        socket: String,
    },
    DetachFromSocket {
        child_id: String,
    },
}

#[derive(Default)]
pub struct ScriptContext {
    pub commands: Vec<EngineCommand>,
    pub vars: HashMap<String, Dynamic>,
    pub object_positions: HashMap<String, (f32, f32, f32)>,
    pub object_rotations: HashMap<String, (f32, f32, f32, f32)>,
    pub object_half_sizes: HashMap<String, (f32, f32, f32)>,
    pub rig_positions: HashMap<String, (f32, f32, f32)>,
    pub current_player: PlayerId,
    pub last_raycast_hit: Option<(f32, f32, f32)>,
    pub raycast_hit_object: String,

    /// Continuous controller values, mirrored here each frame so scripts can
    /// poll them from on_update. Edges arrive as events; levels are polled.
    pub input_axes: InputAxes,
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

    pub fn set_object_rotation(&self, id: &str, x: f32, y: f32, z: f32, w: f32) {
        let mut ctx = self.context.lock().unwrap();
        ctx.object_rotations.insert(id.to_string(), (x, y, z, w));
    }

    pub fn set_object_half_size(&self, id: &str, x: f32, y: f32, z: f32) {
        let mut ctx = self.context.lock().unwrap();
        ctx.object_half_sizes.insert(id.to_string(), (x, y, z));
    }

    pub fn set_rig_position(&self, joint_name: &str, x: f32, y: f32, z: f32) {
        let mut ctx = self.context.lock().unwrap();
        ctx.rig_positions.insert(joint_name.to_string(), (x, y, z));
    }

    pub fn set_current_player(&self, player: PlayerId) {
        self.context.lock().unwrap().current_player = player;
    }

    pub fn set_input_axes(&self, axes: &InputAxes) {
        self.context.lock().unwrap().input_axes = *axes;
    }
}


fn build_engine(context: SharedContext) -> Engine {
    let mut engine = Engine::new();
    crate::script_fns_transform::register_transform_and_scene_fns(&mut engine, &context);
    crate::script_fns_query::register_position_query_fns(&mut engine, &context);
    crate::script_fns_interact::register_interaction_and_audio_fns(&mut engine, &context);
    crate::script_fns_input::register_input_fns(&mut engine, &context);
    engine
}
