use log::warn;

use crate::animation::{sample, AnimationPlayer};
use crate::runtime::GameRuntime;

impl GameRuntime {
    pub(crate) fn update_animations(&mut self, dt: f32) {
        let mut finished: Vec<String> = Vec::new();

        let Self { scene, players, .. } = self;
        for (entity_id, player) in players.iter_mut() {
            let Some(entity) = scene.find_object(entity_id) else {
                continue;
            };
            let Some(anim) = entity.find_animation(&player.anim_name) else {
                continue;
            };
            let duration = anim.duration();
            player.tick(dt, duration);
            if player.finished {
                finished.push(entity_id.clone());
            }
        }

        let samples: Vec<(String, crate::animation::Sample)> = self
            .players
            .iter()
            .filter_map(|(entity_id, player)| {
                let entity = self.scene.find_object(entity_id)?;
                let anim = entity.find_animation(&player.anim_name)?;
                Some((entity_id.clone(), sample(anim, player.elapsed)))
            })
            .collect();

        for (entity_id, s) in samples {
            if let Some(entity_mut) = self.scene.find_object_mut(&entity_id) {
                if let Some(p) = s.position {
                    entity_mut.cuboid.position = p;
                }
                if let Some(r) = s.rotation {
                    entity_mut.cuboid.rotation = r;
                }
                if let Some(sc) = s.scale {
                    entity_mut.cuboid.half_size = sc;
                }
                if let Some(c) = s.color {
                    entity_mut.cuboid.color = c;
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

    pub(crate) fn play_animation(&mut self, entity_id: &str, anim_name: &str) {
        let Some(entity) = self.scene.find_object(entity_id) else {
            warn!("play_animation: unknown object '{entity_id}'");
            return;
        };
        let Some(anim) = entity.find_animation(anim_name) else {
            warn!("play_animation: object '{entity_id}' has no animation '{anim_name}'");
            return;
        };
        self.players
            .insert(entity_id.to_string(), AnimationPlayer::new(anim));
    }

    pub(crate) fn stop_animation(&mut self, entity_id: &str) {
        self.players.remove(entity_id);
        self.anim_queues.remove(entity_id);
    }
}
