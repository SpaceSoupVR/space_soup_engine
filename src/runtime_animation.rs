use log::warn;

use crate::animation::{sample, AnimationPlayer};
use crate::runtime::GameRuntime;

impl GameRuntime {
    pub(crate) fn update_animations(&mut self, dt: f32) {
        let mut finished: Vec<String> = Vec::new();

        let Self { scene, players, .. } = self;
        for (game_object_id, player) in players.iter_mut() {
            let Some(game_object) = scene.find_object(game_object_id) else {
                continue;
            };
            let Some(anim) = game_object.find_animation(&player.anim_name) else {
                continue;
            };
            let duration = anim.duration();
            player.tick(dt, duration);
            if player.finished {
                finished.push(game_object_id.clone());
            }
        }

        let samples: Vec<(String, crate::animation::Sample)> = self
            .players
            .iter()
            .filter_map(|(game_object_id, player)| {
                let game_object = self.scene.find_object(game_object_id)?;
                let anim = game_object.find_animation(&player.anim_name)?;
                Some((game_object_id.clone(), sample(anim, player.elapsed)))
            })
            .collect();

        for (game_object_id, s) in samples {
            if let Some(game_object_mut) = self.scene.find_object_mut(&game_object_id) {
                if let Some(p) = s.position {
                    game_object_mut.cuboid.position = p;
                }
                if let Some(r) = s.rotation {
                    game_object_mut.cuboid.rotation = r;
                }
                if let Some(sc) = s.scale {
                    game_object_mut.cuboid.half_size = sc;
                }
                if let Some(c) = s.color {
                    game_object_mut.cuboid.color = c;
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

    pub(crate) fn play_animation(&mut self, game_object_id: &str, anim_name: &str) {
        let Some(game_object) = self.scene.find_object(game_object_id) else {
            warn!("play_animation: unknown object '{game_object_id}'");
            return;
        };
        let Some(anim) = game_object.find_animation(anim_name) else {
            warn!("play_animation: object '{game_object_id}' has no animation '{anim_name}'");
            return;
        };
        self.players
            .insert(game_object_id.to_string(), AnimationPlayer::new(anim));
    }

    pub(crate) fn stop_animation(&mut self, game_object_id: &str) {
        self.players.remove(game_object_id);
        self.anim_queues.remove(game_object_id);
    }
}
