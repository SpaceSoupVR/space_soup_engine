use rhai::Engine;

use crate::script::{parse_hand, SharedContext};

/// Polled controller inputs.
///
/// Button edges arrive as events (`on_button_down` / `on_button_up`); anything
/// continuous is read here instead. An axis changes every frame, so delivering
/// it as an event would wake a script sixty times a second to tell it something
/// it can simply ask for.
///
/// This is what makes held-button behaviour writable. Button presses are
/// edge-triggered, so a script could previously see the trigger go down and had
/// nothing to tell it the trigger was still down -- which is exactly what a
/// cycling bolt or a two-stage trigger needs to know.
///
/// `hand` is "left" or "right"; anything else parses as right, matching every
/// other hand-taking script function.
pub(crate) fn register_input_fns(engine: &mut Engine, context: &SharedContext) {
    {
        let ctx = context.clone();
        engine.register_fn("get_trigger", move |hand: &str| {
            ctx.lock().unwrap().input_axes.trigger(parse_hand(hand)) as f64
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_grip", move |hand: &str| {
            ctx.lock().unwrap().input_axes.grip(parse_hand(hand)) as f64
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_stick_x", move |hand: &str| {
            ctx.lock().unwrap().input_axes.stick(parse_hand(hand))[0] as f64
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_stick_y", move |hand: &str| {
            ctx.lock().unwrap().input_axes.stick(parse_hand(hand))[1] as f64
        });
    }
}
