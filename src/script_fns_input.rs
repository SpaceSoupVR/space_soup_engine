use rhai::Engine;

use crate::script::{parse_hand, SharedContext};

pub(crate) fn register_input_fns(engine: &mut Engine, context: &SharedContext) {
    {
        let ctx = context.clone();
        engine.register_fn("get_trigger", move |hand: &str| {
            f64::from(ctx.lock().unwrap().input_axes.trigger(parse_hand(hand)))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_grip", move |hand: &str| {
            f64::from(ctx.lock().unwrap().input_axes.grip(parse_hand(hand)))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_stick_x", move |hand: &str| {
            f64::from(ctx.lock().unwrap().input_axes.stick(parse_hand(hand))[0])
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_stick_y", move |hand: &str| {
            f64::from(ctx.lock().unwrap().input_axes.stick(parse_hand(hand))[1])
        });
    }
}
