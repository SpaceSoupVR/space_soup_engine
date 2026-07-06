use crate::scene::{Animation, Color3};
use glam::{Quat, Vec3};

#[derive(Debug, Clone)]
pub struct AnimationPlayer {
    pub anim_name: String,
    pub elapsed: f32,
    pub looping: bool,
    pub finished: bool,
}

impl AnimationPlayer {
    pub fn new(anim: &Animation) -> Self {
        Self {
            anim_name: anim.name.clone(),
            elapsed: 0.0,
            looping: anim.looping,
            finished: false,
        }
    }

    pub fn tick(&mut self, dt: f32, duration: f32) {
        if self.finished {
            return;
        }
        self.elapsed += dt;
        if self.elapsed >= duration {
            if self.looping {
                self.elapsed %= duration.max(0.0001);
            } else {
                self.elapsed = duration;
                self.finished = true;
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
    pub color: Option<Color3>,
}

pub fn sample(anim: &Animation, t: f32) -> Sample {
    if anim.keyframes.is_empty() {
        return Sample::default();
    }
    if anim.keyframes.len() == 1 {
        let k = &anim.keyframes[0];
        return Sample {
            position: k.position,
            rotation: k.rotation,
            scale: k.scale,
            color: k.color,
        };
    }

    let mut prev = &anim.keyframes[0];
    let mut next = &anim.keyframes[anim.keyframes.len() - 1];

    for i in 0..anim.keyframes.len() - 1 {
        let a = &anim.keyframes[i];
        let b = &anim.keyframes[i + 1];
        if t >= a.t && t <= b.t {
            prev = a;
            next = b;
            break;
        }
    }

    let span = (next.t - prev.t).max(0.0001);
    let raw_alpha = ((t - prev.t) / span).clamp(0.0, 1.0);
    let alpha = anim.easing.apply(raw_alpha);

    Sample {
        position: lerp_opt_vec3(prev.position, next.position, alpha),
        rotation: lerp_opt_quat(prev.rotation, next.rotation, alpha),
        scale: lerp_opt_vec3(prev.scale, next.scale, alpha),
        color: lerp_opt_color(prev.color, next.color, alpha),
    }
}

fn lerp_opt_vec3(a: Option<Vec3>, b: Option<Vec3>, t: f32) -> Option<Vec3> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.lerp(b, t)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn lerp_opt_quat(a: Option<Quat>, b: Option<Quat>, t: f32) -> Option<Quat> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.slerp(b, t)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn lerp_opt_color(a: Option<Color3>, b: Option<Color3>, t: f32) -> Option<Color3> {
    match (a, b) {
        (Some(a), Some(b)) => Some(Color3(
            lerp_u8(a.0, b.0, t),
            lerp_u8(a.1, b.1, t),
            lerp_u8(a.2, b.2, t),
            lerp_u8(a.3, b.3, t),
        )),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}
