use glam::Vec3;
use space_soup_protocol::PlayerId;

use crate::events::Hand;
use crate::rig::JointId;
use crate::runtime::{
    GameRuntime, RenderCuboid, RenderLaser, RenderLight, RenderMesh, RenderParticleBurst,
    RenderParticleEmitter, SoundState,
};
use crate::scene::{CuboidShape, GameObject, GripPointDef, MeshRef};

impl GameRuntime {
    pub fn held_grip_point(&self, player: PlayerId, hand: Hand) -> Option<(&GameObject, &GripPointDef)> {
        if let Some((id, point_name)) = self.rigid_physics.held_by(player, hand) {
            if let Some(point) = self
                .scene
                .find_object(id)
                .and_then(|obj| obj.grip_point(point_name).map(|p| (obj, p)))
            {
                return Some(point);
            }
        }

        let (id, point_name) = self
            .attachments
            .grip_point_at_joint(player, JointId::HandGrip(hand))?;
        let obj = self.scene.find_object(id)?;
        let point = obj.grip_point(point_name)?;
        Some((obj, point))
    }

    pub(crate) fn collect_render_cuboids(&self) -> Vec<RenderCuboid> {
        self.scene
            .objects
            .iter()
            // `hidden` is what the level author decided; `is_removed` is what
            // this match has done to it. A chunk shot out of a wall has to stop
            // being drawn, and it has no named parts for `hidden_parts` to
            // reach -- that field is only consulted for meshes below.
            //
            // Brushes are excluded because they are NOT boxes. Sending one here
            // drew a wall as its bounding cuboid -- a fractured wall came out as
            // twelve overlapping crates -- so the client meshes brushes itself
            // from scene data it already has, and this list would only double
            // -draw them. See `hidden_brushes` for how it is told which to skip.
            .filter(|o| {
                !o.hidden && !self.damage.is_removed(o) && o.mesh.is_none() && o.brush.is_none()
            })
            .map(|o| RenderCuboid {
                id: o.id.clone(),
                position: o.cuboid.position,
                half_size: o.cuboid.half_size,
                rotation: o.cuboid.rotation,
                color: o.cuboid.color,
                wire_color: o.cuboid.wire_color,
                style: o.cuboid.style,
                reflectivity: o.cuboid.reflectivity,
                shape: if o.teleportal.is_some() {
                    CuboidShape::Cylinder
                } else {
                    CuboidShape::Box
                },
            })
            .collect()
    }

    /// Brush objects the client must NOT draw this frame.
    ///
    /// Brush geometry is static scene data that ships with the game, so the
    /// client builds it once at scene load rather than receiving triangles over
    /// the wire -- the same trade terrain makes, and for the same reason: at 64
    /// players the snapshot budget is the binding constraint.
    ///
    /// What that cannot know is what has happened SINCE. A chunk shot out of a
    /// wall, or a brush a script hid, is a runtime fact, and it is the only part
    /// of a brush that has to cross per snapshot. Sending ids rather than
    /// geometry keeps it to a few bytes, and to nothing at all in the ordinary
    /// case where a level is intact.
    pub fn hidden_brushes(&self) -> Vec<String> {
        self.scene
            .objects
            .iter()
            .filter(|o| o.brush.is_some())
            .filter(|o| o.hidden || self.damage.is_removed(o))
            .map(|o| o.id.clone())
            .collect()
    }

    pub(crate) fn collect_render_meshes(&self) -> Vec<RenderMesh> {
        self.scene
            .objects
            .iter()
            .filter(|o| !o.hidden && !self.damage.is_removed(o))
            .filter_map(|o| {
                let mesh_ref: &MeshRef = o.mesh.as_ref()?;
                Some(RenderMesh {
                    id: o.id.clone(),
                    path: mesh_ref.path.clone(),
                    position: o.cuboid.position,
                    rotation: o.cuboid.rotation * mesh_ref.rotation_offset,
                    scale: mesh_ref.scale,
                    manual_part_blends: self.manual_part_blends.get(&o.id).cloned().unwrap_or_default(),
                    // The damage ledger decides this, not the authored list.
                    // Damage therefore reaches the headset through a field that
                    // already replicates -- breaching needed no new wire
                    // message, no new client code and no new decoder.
                    hidden_parts: self.damage.hidden_parts_for(o).to_vec(),
                    disabled_clips: o
                        .part_animations
                        .iter()
                        .filter(|pa| {
                            pa.enabled_when
                                .as_ref()
                                .is_some_and(|c| !c.holds(&|k| self.script_host.var_string(k)))
                        })
                        .map(|pa| pa.clip.clone())
                        .collect(),
                })
            })
            .collect()
    }

    /// The lights the GPU shades every frame.
    ///
    /// `Baked` lights are excluded: their contribution is already in the
    /// lightmap, and uploading them as well would add the same light twice --
    /// once occluded by the walls and once through them. The visible result is
    /// a room that is too bright AND still leaks, which looks like two
    /// unrelated bugs.
    ///
    /// Their index is still taken from the object's full light list, so a
    /// light's id does not change when a sibling is switched to baked.
    pub(crate) fn collect_render_lights(&self) -> Vec<RenderLight> {
        self.scene
            .objects
            .iter()
            .flat_map(|o| {
                o.lights
                    .iter()
                    .enumerate()
                    .filter(|(_, light)| light.mode == crate::LightMode::Realtime)
                    .map(move |(i, light)| RenderLight {
                    id: format!("{}#{i}", o.id),
                    position: o.cuboid.position,
                    direction: o.cuboid.rotation * Vec3::NEG_Z,
                    kind: light.kind,
                    color: light.color,
                    intensity: light.intensity,
                    range: light.range,
                    cone_angle_deg: light.cone_angle_deg,
                })
            })
            .collect()
    }

    pub(crate) fn collect_render_particle_emitters(&self) -> Vec<RenderParticleEmitter> {
        self.scene
            .objects
            .iter()
            .filter_map(|o| {
                let pe = o.particle_emitter.as_ref()?;
                Some(RenderParticleEmitter {
                    id: o.id.clone(),
                    position: o.cuboid.position,
                    direction: o.cuboid.rotation * Vec3::NEG_Z,
                    particle_size: pe.particle_size,
                    spawn_rate: pe.spawn_rate,
                    color: pe.color,
                    lifetime: pe.lifetime,
                    speed: pe.speed,
                    spread_deg: pe.spread_deg,
                    size_growth: pe.size_growth,
                })
            })
            .collect()
    }

    pub(crate) fn collect_render_particle_bursts(&self) -> Vec<RenderParticleBurst> {
        self.particle_bursts
            .iter()
            .map(|b| RenderParticleBurst {
                id: b.id.clone(),
                position: b.position,
                direction: b.direction,
                color: b.color,
                count: b.count,
                speed: b.speed,
                spread_deg: b.spread_deg,
                particle_size: b.particle_size,
                lifetime: b.lifetime,
                elapsed: b.elapsed,
            })
            .collect()
    }

    pub(crate) fn collect_render_lasers(&self) -> Vec<RenderLaser> {
        self.scene
            .objects
            .iter()
            .filter_map(|o| {
                let laser = o.laser.as_ref()?;
                let origin = o.cuboid.position;
                let direction = o.cuboid.rotation * Vec3::NEG_Z;
                let end = self
                    .rigid_physics
                    .raycast(origin, direction, laser.max_distance)
                    .map(|(hit_point, _normal)| hit_point)
                    .unwrap_or(origin + direction * laser.max_distance);
                Some(RenderLaser {
                    id: o.id.clone(),
                    origin,
                    direction,
                    end,
                    color: laser.color,
                    beam_width: laser.beam_width,
                })
            })
            .collect()
    }

    pub fn preview_sound(&mut self, clip: &str, volume: f32, pitch: f32) {
        self.sound_engine.preview(&self.game_dir, clip, volume, pitch);
    }

    pub fn active_sounds(&self) -> Vec<SoundState> {
        self.sound_engine
            .active_sounds(&self.scene.objects)
            .into_iter()
            .map(|(object_id, clip, position, volume, pitch, looping, min_distance, max_distance)| SoundState {
                object_id,
                clip,
                position,
                volume,
                pitch,
                looping,
                min_distance,
                max_distance,
            })
            .collect()
    }
}
