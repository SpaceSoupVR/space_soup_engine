use glam::{Quat, Vec3};
use rhai::Engine;

use crate::physics::ray_intersect_obb;
use crate::script::SharedContext;

pub(crate) fn register_position_query_fns(engine: &mut Engine, context: &SharedContext) {
    {
        let ctx = context.clone();

        engine.register_fn("is_occupied", move |id: &str| -> bool {
            ctx.lock().unwrap().occupied_volumes.contains(id)
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_x", move |id: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .object_positions
                .get(id)
                .map_or(0.0, |p| f64::from(p.0))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_y", move |id: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .object_positions
                .get(id)
                .map_or(0.0, |p| f64::from(p.1))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_z", move |id: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .object_positions
                .get(id)
                .map_or(0.0, |p| f64::from(p.2))
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("get_rig_x", move |joint: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .rig_positions
                .get(joint)
                .map_or(0.0, |p| f64::from(p.0))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_rig_y", move |joint: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .rig_positions
                .get(joint)
                .map_or(0.0, |p| f64::from(p.1))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_rig_z", move |joint: &str| -> f64 {
            ctx.lock()
                .unwrap()
                .rig_positions
                .get(joint)
                .map_or(0.0, |p| f64::from(p.2))
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn("get_object_rot_x", move |id: &str| -> f64 {
            ctx.lock().unwrap().object_rotations.get(id).map_or(0.0, |r| f64::from(r.0))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_rot_y", move |id: &str| -> f64 {
            ctx.lock().unwrap().object_rotations.get(id).map_or(0.0, |r| f64::from(r.1))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_rot_z", move |id: &str| -> f64 {
            ctx.lock().unwrap().object_rotations.get(id).map_or(0.0, |r| f64::from(r.2))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("get_object_rot_w", move |id: &str| -> f64 {
            ctx.lock().unwrap().object_rotations.get(id).map_or(1.0, |r| f64::from(r.3))
        });
    }

    {
        let ctx = context.clone();
        engine.register_fn(
            "raycast",
            move |ox: f64, oy: f64, oz: f64, dx: f64, dy: f64, dz: f64, max_dist: f64| -> bool {
                let mut ctx = ctx.lock().unwrap();
                let origin = Vec3::new(ox as f32, oy as f32, oz as f32);
                let dir = Vec3::new(dx as f32, dy as f32, dz as f32).normalize_or_zero();
                if dir == Vec3::ZERO {
                    ctx.last_raycast_hit = None;
                    return false;
                }

                let mut best: Option<(String, f32)> = None;
                for (id, &(px, py, pz)) in &ctx.object_positions {
                    let center = Vec3::new(px, py, pz);
                    let (rx, ry, rz, rw) =
                        ctx.object_rotations.get(id).copied().unwrap_or((0.0, 0.0, 0.0, 1.0));
                    let rotation = Quat::from_xyzw(rx, ry, rz, rw);
                    let (hx, hy, hz) =
                        ctx.object_half_sizes.get(id).copied().unwrap_or((0.5, 0.5, 0.5));
                    let half_size = Vec3::new(hx, hy, hz);
                    if let Some(dist) =
                        ray_intersect_obb(origin, dir, center, half_size, rotation, max_dist as f32)
                    {
                        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                            best = Some((id.clone(), dist));
                        }
                    }
                }

                if let Some((id, dist)) = best {
                    let hit = origin + dir * dist;
                    ctx.last_raycast_hit = Some((hit.x, hit.y, hit.z));
                    ctx.raycast_hit_object = id;
                    true
                } else {
                    ctx.last_raycast_hit = None;
                    ctx.raycast_hit_object.clear();
                    false
                }
            },
        );
    }

    {
        let ctx = context.clone();
        engine.register_fn("raycast_hit_object", move || -> String {
            ctx.lock().unwrap().raycast_hit_object.clone()
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("raycast_hit_x", move || -> f64 {
            ctx.lock().unwrap().last_raycast_hit.map_or(0.0, |p| f64::from(p.0))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("raycast_hit_y", move || -> f64 {
            ctx.lock().unwrap().last_raycast_hit.map_or(0.0, |p| f64::from(p.1))
        });
    }
    {
        let ctx = context.clone();
        engine.register_fn("raycast_hit_z", move || -> f64 {
            ctx.lock().unwrap().last_raycast_hit.map_or(0.0, |p| f64::from(p.2))
        });
    }
}
