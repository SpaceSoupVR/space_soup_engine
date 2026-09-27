//! STATIONARY LAMPS' SHADOW CHANNELS: which channel of the brush atlas's
//! stationary masks holds each stationary lamp's baked shadows.
//!
//! A stationary lamp (`LightMode::Stationary`) is shaded every frame -- its
//! real cone, falloff and highlight -- while its shadows come from the bake: a
//! signed distance to the shadow's edge, one byte per lamp, in one channel of
//! an RGBA mask on the brush charts. Four lamps per mask layer.
//!
//! # Computed, not stored
//!
//! The baker writes each lamp's mask into its channel; the runtime tells each
//! lamp which channel to read. Both call [`scene_stationary_channels`] on the
//! same scene, so they cannot disagree, and no file has to carry the mapping --
//! the editor's own bake path included. A scene edited after its bake is stale
//! anyway, and `lightmaps` already warns about that.
//!
//! # Sharing channels
//!
//! Two lamps may share a channel when no brush texel is reached by both --
//! within range, inside the cone -- because each then reads its channel only
//! where the other's mask is not. Measured at the lightmap's own texels, with
//! [`REACH_SLACK`] to spare. Greedy graph colouring, most-constrained lamp
//! first, ties broken by id so the answer is the same on every machine. A lamp
//! that finds no free channel is left unshadowed and reported: the level has
//! more than [`MAX_STATIONARY_CHANNELS`] lamps lighting one spot.
//!
//! This is Unreal's stationary-light channel assignment, which colours with
//! four; eight here, two RGBA layers.

use std::collections::HashSet;

use glam::Vec3;

use crate::brush::BrushDef;
use crate::brush_lightmap::BrushLightmapLayout;
use crate::scene::GameObject;
use crate::scene_light::{LightKind, LightMode};

/// How many stationary lamps may light any one texel: two RGBA mask layers.
pub const MAX_STATIONARY_CHANNELS: usize = 8;

/// How far past a lamp's range, as a fraction of it, and past its cone, in
/// cosine, a texel still counts as reached when deciding which lamps may share
/// a channel. The shader's falloff ends exactly at the range, so a margin here
/// only ever keeps two lamps apart that could have shared; never the reverse.
pub const REACH_SLACK: f32 = 0.05;

/// One stationary lamp, resolved to where it is and how it points.
#[derive(Clone, Debug, PartialEq)]
pub struct StationaryLamp {
    pub object_id: String,
    /// Index into the object's FULL light list -- the same numbering as the
    /// render light's id, `object#index`.
    pub light_index: usize,
    pub position: Vec3,
    pub direction: Vec3,
    pub kind: LightKind,
    pub range: f32,
    /// Cosine of the outer cone's half-angle; ignored for a point lamp.
    pub cos_outer: f32,
}

impl StationaryLamp {
    /// The render light's id: `object#index`.
    pub fn id(&self) -> String {
        format!("{}#{}", self.object_id, self.light_index)
    }

    /// Whether this lamp's light can reach `p` at all: in range and, for a
    /// spot, inside the cone -- each widened by `slack`.
    pub fn reaches(&self, p: Vec3, slack: f32) -> bool {
        let to = p - self.position;
        let d2 = to.length_squared();
        let r = self.range * (1.0 + slack);
        if d2 > r * r {
            return false;
        }
        if self.kind == LightKind::Spot {
            let d = d2.sqrt().max(1e-6);
            if to.dot(self.direction) / d < self.cos_outer - slack {
                return false;
            }
        }
        true
    }
}

/// Every enabled stationary point or spot lamp in `objects`, posed through its
/// socket exactly as the baker and the renderer pose it.
pub fn stationary_lamps(objects: &[GameObject]) -> Vec<StationaryLamp> {
    let mut out = Vec::new();
    for o in objects {
        for (i, l) in o.lights.iter().enumerate() {
            if l.mode != LightMode::Stationary || !l.enabled || l.kind == LightKind::Directional {
                continue;
            }
            let pose = crate::scene_light::resolve_light_pose(
                l,
                o.cuboid.position,
                o.cuboid.rotation,
                l.socket.as_deref().and_then(|n| o.socket(n)),
            );
            out.push(StationaryLamp {
                object_id: o.id.clone(),
                light_index: i,
                position: pose.position,
                direction: pose.direction(),
                kind: l.kind,
                range: l.range,
                cos_outer: (l.cone_angle_deg.to_radians() * 0.5).cos(),
            });
        }
    }
    out
}

/// A mask channel for each of `lamps`, in order: `None` where none was free.
/// See the module note.
pub fn assign_channels(lamps: &[StationaryLamp], layout: &BrushLightmapLayout) -> Vec<Option<u8>> {
    // Lamps are named by bit, so at most 64 take part; past that they all
    // lose the colouring anyway.
    let n = lamps.len().min(64);
    let mut adjacency = vec![0u64; n];
    let mut seen: HashSet<u64> = HashSet::new();
    for chart in &layout.charts {
        for ty in 0..chart.h {
            for tx in 0..chart.w {
                let w = chart.texel_world(tx, ty);
                let p = Vec3::new(w[0] as f32, w[1] as f32, w[2] as f32);
                let mut mask = 0u64;
                for (i, lamp) in lamps.iter().take(n).enumerate() {
                    if lamp.reaches(p, REACH_SLACK) {
                        mask |= 1 << i;
                    }
                }
                if mask.count_ones() > 1 && seen.insert(mask) {
                    for i in 0..n {
                        if mask & (1 << i) != 0 {
                            adjacency[i] |= mask & !(1 << i);
                        }
                    }
                }
            }
        }
    }
    let ids: Vec<String> = lamps.iter().map(StationaryLamp::id).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        adjacency[b].count_ones().cmp(&adjacency[a].count_ones()).then_with(|| ids[a].cmp(&ids[b]))
    });
    let mut channel: Vec<Option<u8>> = vec![None; lamps.len()];
    for &i in &order {
        let used = (0..n)
            .filter(|&j| adjacency[i] & (1 << j) != 0)
            .filter_map(|j| channel[j])
            .fold(0u32, |acc, c| acc | (1 << c));
        channel[i] = (0..MAX_STATIONARY_CHANNELS as u8).find(|c| used & (1 << c) == 0);
    }
    channel
}

/// The whole assignment for a scene: every stationary lamp and its channel.
pub fn scene_stationary_channels(objects: &[GameObject]) -> Vec<(StationaryLamp, Option<u8>)> {
    let lamps = stationary_lamps(objects);
    if lamps.is_empty() {
        return Vec::new();
    }
    // The same brushes in the same order as the baker's atlas: every object
    // with a brush, in scene order.
    let brushes: Vec<&BrushDef> = objects.iter().filter_map(|o| o.brush.as_ref()).collect();
    let layout = crate::brush_lightmap::scene_brush_lightmap_layout(&brushes);
    let channels = assign_channels(&lamps, &layout);
    lamps.into_iter().zip(channels).collect()
}

/// How many RGBA mask layers `channels` need: one per four, at least one when
/// any lamp has a channel.
pub fn mask_layers(channels: &[Option<u8>]) -> usize {
    channels.iter().flatten().map(|&c| c as usize / 4 + 1).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(json: &str) -> GameObject {
        serde_json::from_str(json).expect("the test object parses")
    }

    /// A floor 20 m long, as one brush.
    fn floor() -> GameObject {
        object(
            r#"{"id":"floor","brush":{"solids":[{"faces":[
                {"plane":[1,0,0,10],"material":"m"},{"plane":[-1,0,0,10],"material":"m"},
                {"plane":[0,1,0,0],"material":"m"},{"plane":[0,-1,0,0.3],"material":"m"},
                {"plane":[0,0,1,2],"material":"m"},{"plane":[0,0,-1,2],"material":"m"}]}]}}"#,
        )
    }

    fn lamp(id: &str, x: f32, range: f32) -> GameObject {
        object(&format!(
            r#"{{"id":"{id}","cuboid":{{"position":[{x},2.5,0]}},
               "lights":[{{"kind":"Point","mode":"Stationary","range":{range}}}]}}"#
        ))
    }

    /// Two lamps whose light meets on the floor need different channels; a
    /// third far enough from both can share with either.
    #[test]
    fn lamps_that_meet_get_different_channels_and_far_ones_share() {
        let objects = [floor(), lamp("a", -6.0, 4.0), lamp("b", -2.0, 4.0), lamp("c", 7.0, 3.0)];
        let got = scene_stationary_channels(&objects);
        assert_eq!(got.len(), 3);
        let ch = |id: &str| got.iter().find(|(l, _)| l.object_id == id).and_then(|(_, c)| *c).expect("a channel");
        assert_ne!(ch("a"), ch("b"), "overlapping lamps shared a channel");
        assert!(ch("c") == ch("a") || ch("c") == ch("b"), "a lamp far from both did not reuse a channel");
        assert_eq!(mask_layers(&got.iter().map(|(_, c)| *c).collect::<Vec<_>>()), 1);
    }

    /// The same scene always colours the same way -- the baker and the runtime
    /// each compute it and must agree.
    #[test]
    fn the_assignment_is_deterministic() {
        let objects = [floor(), lamp("b", -2.0, 4.0), lamp("a", -6.0, 4.0), lamp("c", 7.0, 3.0)];
        let one = scene_stationary_channels(&objects);
        let two = scene_stationary_channels(&objects);
        assert_eq!(one, two);
    }

    /// Nine lamps all lighting one spot: eight get channels, one does not.
    #[test]
    fn past_eight_at_one_spot_a_lamp_goes_unshadowed() {
        let mut objects = vec![floor()];
        for i in 0..9 {
            objects.push(lamp(&format!("l{i}"), -1.0 + i as f32 * 0.2, 6.0));
        }
        let got = scene_stationary_channels(&objects);
        assert_eq!(got.iter().filter(|(_, c)| c.is_some()).count(), MAX_STATIONARY_CHANNELS);
        assert_eq!(mask_layers(&got.iter().map(|(_, c)| *c).collect::<Vec<_>>()), 2);
    }

    #[test]
    fn a_spot_reaches_only_inside_its_cone() {
        let l = StationaryLamp {
            object_id: "s".into(),
            light_index: 0,
            position: Vec3::new(0.0, 3.0, 0.0),
            direction: Vec3::NEG_Y,
            kind: LightKind::Spot,
            range: 10.0,
            cos_outer: (30f32.to_radians()).cos(),
        };
        assert!(l.reaches(Vec3::new(0.0, 0.0, 0.0), 0.0));
        assert!(!l.reaches(Vec3::new(3.0, 0.0, 0.0), 0.0), "45 degrees off a 30-degree cone");
        assert!(!l.reaches(Vec3::new(0.0, -8.0, 0.0), 0.0), "past the range");
    }
}
