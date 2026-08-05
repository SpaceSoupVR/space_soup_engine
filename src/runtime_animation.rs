use log::warn;

use crate::animation::{sample, AnimationPlayer};
use crate::runtime::GameRuntime;

impl GameRuntime {
    pub(crate) fn update_animations(&mut self, dt: f32) {
        let mut finished: Vec<String> = Vec::new();

        let Self { scene, players, .. } = self;
        for (obj_id, player) in players.iter_mut() {
            let Some(obj) = scene.find_object(obj_id) else {
                continue;
            };
            let Some(anim) = obj.find_animation(&player.anim_name) else {
                continue;
            };
            let duration = anim.duration();
            player.tick(dt, duration);
            if player.finished {
                finished.push(obj_id.clone());
            }
        }

        let samples: Vec<(String, crate::animation::Sample)> = self
            .players
            .iter()
            .filter_map(|(obj_id, player)| {
                let obj = self.scene.find_object(obj_id)?;
                let anim = obj.find_animation(&player.anim_name)?;
                Some((obj_id.clone(), sample(anim, player.elapsed)))
            })
            .collect();

        for (obj_id, s) in samples {
            if let Some(obj_mut) = self.scene.find_object_mut(&obj_id) {
                if let Some(p) = s.position {
                    obj_mut.cuboid.position = p;
                }
                if let Some(r) = s.rotation {
                    obj_mut.cuboid.rotation = r;
                }
                if let Some(sc) = s.scale {
                    obj_mut.cuboid.half_size = sc;
                }
                if let Some(c) = s.color {
                    obj_mut.cuboid.color = c;
                }
            }
        }

        for id in finished {
            self.players.remove(&id);
            let next = self
                .anim_queues
                .get_mut(&id)
                .and_then(|q| (!q.is_empty()).then(|| q.remove(0)));
            if let Some(anim_name) = next {
                self.play_animation(&id, &anim_name);
            }
        }
    }

    pub(crate) fn play_animation(&mut self, obj_id: &str, anim_name: &str) {
        let Some(obj) = self.scene.find_object(obj_id) else {
            warn!("play_animation: unknown object '{obj_id}'");
            return;
        };
        let Some(anim) = obj.find_animation(anim_name) else {
            warn!("play_animation: object '{obj_id}' has no animation '{anim_name}'");
            return;
        };
        self.players
            .insert(obj_id.to_string(), AnimationPlayer::new(anim));
    }

    pub(crate) fn stop_animation(&mut self, obj_id: &str) {
        self.players.remove(obj_id);
        self.anim_queues.remove(obj_id);
    }
}
