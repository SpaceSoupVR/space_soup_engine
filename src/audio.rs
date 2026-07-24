use std::collections::{HashMap, HashSet};
use std::path::Path;

use glam::{Quat, Vec3};
use kira::listener::ListenerHandle;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::PlaybackState;
use kira::track::{SpatialTrackBuilder, SpatialTrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Easing, Tween};

use crate::scene::GameObject;

struct ActiveSound {
    track: SpatialTrackHandle,
    handle: StaticSoundHandle,
}

pub struct SoundEngine {
    manager: Option<AudioManager>,
    listener: Option<ListenerHandle>,
    clips: HashMap<String, StaticSoundData>,
    playing: HashMap<String, ActiveSound>,
    autostarted: HashSet<String>,
    active: HashSet<String>,
}

fn to_mint_vec3(v: Vec3) -> mint::Vector3<f32> {
    mint::Vector3 { x: v.x, y: v.y, z: v.z }
}
fn to_mint_quat(q: Quat) -> mint::Quaternion<f32> {
    mint::Quaternion {
        v: mint::Vector3 { x: q.x, y: q.y, z: q.z },
        s: q.w,
    }
}

fn linear_to_decibels(linear: f32) -> Decibels {
    let linear = linear.max(0.0);
    if linear <= 0.0001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * linear.log10())
    }
}

fn cone_attenuation(forward: Vec3, to_listener: Vec3, cone_angle_deg: f32) -> f32 {
    const OFF_AXIS_FLOOR: f32 = 0.15;
    const INNER_FRACTION: f32 = 0.7;
    let Some(to_listener) = to_listener.try_normalize() else {
        return 1.0;
    };
    let half_angle = cone_angle_deg.to_radians() * 0.5;
    let outer_cos = half_angle.cos();
    let inner_cos = (half_angle * INNER_FRACTION).cos();
    let cos_actual = forward.dot(to_listener);
    let t = ((cos_actual - outer_cos) / (inner_cos - outer_cos).max(1e-4)).clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t);
    OFF_AXIS_FLOOR + (1.0 - OFF_AXIS_FLOOR) * eased
}

impl SoundEngine {
    pub fn new() -> Self {
        let mut manager =
            match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
                Ok(m) => Some(m),
                Err(e) => {
                    log::warn!("SoundEngine: no audio output available ({e}); sounds will be silent");
                    None
                }
            };
        let listener = manager
            .as_mut()
            .and_then(|m| match m.add_listener(to_mint_vec3(Vec3::ZERO), to_mint_quat(Quat::IDENTITY)) {
                Ok(l) => Some(l),
                Err(e) => {
                    log::warn!("SoundEngine: failed to create audio listener: {e}");
                    None
                }
            });

        Self {
            manager,
            listener,
            clips: HashMap::new(),
            playing: HashMap::new(),
            autostarted: HashSet::new(),
            active: HashSet::new(),
        }
    }

    fn load_clip(&mut self, game_dir: &Path, clip: &str) -> Option<StaticSoundData> {
        if let Some(data) = self.clips.get(clip) {
            return Some(data.clone());
        }
        match StaticSoundData::from_file(game_dir.join(clip)) {
            Ok(data) => {
                self.clips.insert(clip.to_string(), data.clone());
                Some(data)
            }
            Err(e) => {
                log::warn!("SoundEngine: failed to load clip '{clip}': {e}");
                None
            }
        }
    }

    fn start(&mut self, game_dir: &Path, obj: &GameObject) {
        let Some(sound) = obj.sound.clone() else { return };

        let Some(clip) = self.load_clip(game_dir, &sound.clip) else {
            return;
        };
        let clip = clip
            .volume(linear_to_decibels(sound.volume))
            .playback_rate(sound.pitch as f64);
        let clip = if sound.looping { clip.loop_region(..) } else { clip };

        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let Some(listener) = &self.listener else {
            return;
        };

        let track_builder = SpatialTrackBuilder::new()
            .distances((sound.min_distance, sound.max_distance))
            .attenuation_function(Some(Easing::OutPowf(2.0)));

        let mut track = match manager.add_spatial_sub_track(
            listener.id(),
            to_mint_vec3(obj.cuboid.position),
            track_builder,
        ) {
            Ok(t) => t,
            Err(e) => {
                log::warn!(
                    "SoundEngine: could not allocate spatial track for '{}': {e}",
                    obj.id
                );
                return;
            }
        };
        let handle = match track.play(clip) {
            Ok(h) => h,
            Err(e) => {
                log::warn!("SoundEngine: could not play '{}': {e}", obj.id);
                return;
            }
        };

        self.playing.insert(obj.id.clone(), ActiveSound { track, handle });
    }

    pub fn update(
        &mut self,
        game_dir: &Path,
        objects: &[GameObject],
        requested_play: &HashSet<String>,
        requested_stop: &HashSet<String>,
        listener: Option<(Vec3, Quat)>,
    ) {
        if let Some((listener_pos, listener_rot)) = listener {
            if let Some(listener_handle) = self.listener.as_mut() {
                listener_handle.set_position(to_mint_vec3(listener_pos), Tween::default());
                listener_handle.set_orientation(to_mint_quat(listener_rot), Tween::default());
            }
        }

        for id in requested_stop {
            self.active.remove(id);
            if let Some(mut active) = self.playing.remove(id) {
                active.handle.stop(Tween::default());
            }
        }

        for obj in objects {
            let Some(sound) = &obj.sound else { continue };

            let should_start = requested_play.contains(&obj.id)
                || (sound.autoplay && !self.autostarted.contains(&obj.id));
            if should_start {
                if sound.autoplay {
                    self.autostarted.insert(obj.id.clone());
                }
                self.active.insert(obj.id.clone());
                self.playing.remove(&obj.id);
                self.start(game_dir, obj);
            }

            let Some(active) = self.playing.get_mut(&obj.id) else {
                continue;
            };

            if active.handle.state() == PlaybackState::Stopped {
                self.playing.remove(&obj.id);
                self.active.remove(&obj.id);
                continue;
            }

            let Some((listener_pos, _)) = listener else {
                continue;
            };

            let forward = obj.cuboid.rotation * Vec3::NEG_Z;
            let cone = if sound.directional {
                cone_attenuation(forward, listener_pos - obj.cuboid.position, sound.cone_angle_deg)
            } else {
                1.0
            };

            active
                .track
                .set_position(to_mint_vec3(obj.cuboid.position), Tween::default());
            active
                .track
                .set_volume(linear_to_decibels(cone), Tween::default());
            active
                .handle
                .set_volume(linear_to_decibels(sound.volume), Tween::default());
            active
                .handle
                .set_playback_rate(sound.pitch as f64, Tween::default());
        }
    }

    pub fn active_sounds(&self, objects: &[GameObject]) -> Vec<(String, String, Vec3, f32, f32, bool)> {
        self.active
            .iter()
            .filter_map(|id| {
                let obj = objects.iter().find(|o| &o.id == id)?;
                let sound = obj.sound.as_ref()?;
                Some((
                    id.clone(),
                    sound.clip.clone(),
                    obj.cuboid.position,
                    sound.volume,
                    sound.pitch,
                    sound.looping,
                ))
            })
            .collect()
    }

    pub fn preview(&mut self, game_dir: &Path, clip: &str, volume: f32, pitch: f32) {
        let Some(data) = self.load_clip(game_dir, clip) else {
            return;
        };
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let data = data
            .volume(linear_to_decibels(volume))
            .playback_rate(pitch as f64);
        if let Err(e) = manager.play(data) {
            log::warn!("SoundEngine: preview playback failed: {e}");
        }
    }
}

impl Default for SoundEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod falloff_test {
    use super::*;

    #[test]
    fn cone_attenuation_full_volume_straight_ahead() {
        let atten = cone_attenuation(Vec3::NEG_Z, Vec3::NEG_Z, 60.0);
        assert!((atten - 1.0).abs() < 0.01, "expected ~1.0, got {atten}");
    }

    #[test]
    fn cone_attenuation_floors_behind_source() {
        let atten = cone_attenuation(Vec3::NEG_Z, Vec3::Z, 60.0);
        assert!(
            (atten - 0.15).abs() < 0.01,
            "expected the off-axis floor (~0.15), got {atten}"
        );
    }

    #[test]
    fn cone_attenuation_eases_smoothly_across_the_edge() {
        let half_angle_deg: f32 = 30.0;
        let just_inside = Quat::from_rotation_y((half_angle_deg * 0.6).to_radians()) * Vec3::NEG_Z;
        let just_outside = Quat::from_rotation_y((half_angle_deg * 0.9).to_radians()) * Vec3::NEG_Z;
        let inside = cone_attenuation(Vec3::NEG_Z, just_inside, half_angle_deg * 2.0);
        let outside = cone_attenuation(Vec3::NEG_Z, just_outside, half_angle_deg * 2.0);
        assert!(inside > 0.9, "expected near-full volume inside the cone's core, got {inside}");
        assert!(outside < inside, "expected volume to drop past the outer edge");
        assert!(outside > 0.15, "expected a smooth ease, not an instant drop to the floor");
    }

    #[test]
    fn linear_to_decibels_identity_at_full_volume() {
        assert_eq!(linear_to_decibels(1.0), Decibels::IDENTITY);
    }

    #[test]
    fn linear_to_decibels_silence_at_zero() {
        assert_eq!(linear_to_decibels(0.0), Decibels::SILENCE);
    }

    #[test]
    fn linear_to_decibels_monotonic() {
        let quiet = linear_to_decibels(0.2);
        let loud = linear_to_decibels(0.8);
        assert!(loud.0 > quiet.0, "louder linear volume should be more decibels");
    }
}

