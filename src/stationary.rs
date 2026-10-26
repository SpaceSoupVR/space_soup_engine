use std::collections::HashSet;

use glam::Vec3;

use crate::brush::BrushDef;
use crate::brush_lightmap::BrushLightmapLayout;
use crate::scene::GameObject;
use crate::scene_light::{LightKind, LightMode};

pub const MAX_STATIONARY_CHANNELS: usize = 8;

pub const STATIONARY_LAMPS_PER_LAYER: usize = 2;

pub const MAX_STATIONARY_LAYERS: usize = MAX_STATIONARY_CHANNELS / STATIONARY_LAMPS_PER_LAYER;

pub const STATIONARY_BULB_RADIUS: f32 = 0.03;

pub const REACH_SLACK: f32 = 0.05;

#[derive(Clone, Debug, PartialEq)]
pub struct StationaryLamp {
    pub object_id: String,

    pub light_index: usize,
    pub position: Vec3,
    pub direction: Vec3,
    pub kind: LightKind,
    pub range: f32,

    pub cos_outer: f32,
}

impl StationaryLamp {

    #[must_use]
    pub fn id(&self) -> String {
        format!("{}#{}", self.object_id, self.light_index)
    }

    #[must_use]
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

#[must_use]
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

pub fn assign_channels(lamps: &[StationaryLamp], layout: &BrushLightmapLayout) -> Vec<Option<u8>> {

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

#[must_use]
pub fn scene_stationary_channels(objects: &[GameObject]) -> Vec<(StationaryLamp, Option<u8>)> {
    let lamps = stationary_lamps(objects);
    if lamps.is_empty() {
        return Vec::new();
    }

    let brushes: Vec<&BrushDef> = objects.iter().filter_map(|o| o.brush.as_ref()).collect();
    let layout = crate::brush_lightmap::scene_brush_lightmap_layout(&brushes);
    let channels = assign_channels(&lamps, &layout);
    lamps.into_iter().zip(channels).collect()
}

#[must_use]
pub fn mask_layers(channels: &[Option<u8>]) -> usize {
    channels.iter().flatten().map(|&c| c as usize / STATIONARY_LAMPS_PER_LAYER + 1).max().unwrap_or(0)
}

#[must_use]
pub fn channel_slot(c: u8) -> (usize, usize) {
    let c = c as usize;
    (c / STATIONARY_LAMPS_PER_LAYER, (c % STATIONARY_LAMPS_PER_LAYER) * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(json: &str) -> GameObject {
        serde_json::from_str(json).expect("the test object parses")
    }

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

    #[test]
    fn the_assignment_is_deterministic() {
        let objects = [floor(), lamp("b", -2.0, 4.0), lamp("a", -6.0, 4.0), lamp("c", 7.0, 3.0)];
        let one = scene_stationary_channels(&objects);
        let two = scene_stationary_channels(&objects);
        assert_eq!(one, two);
    }

    #[test]
    fn past_eight_at_one_spot_a_lamp_goes_unshadowed() {
        let mut objects = vec![floor()];
        for i in 0..9 {
            objects.push(lamp(&format!("l{i}"), -1.0 + i as f32 * 0.2, 6.0));
        }
        let got = scene_stationary_channels(&objects);
        assert_eq!(got.iter().filter(|(_, c)| c.is_some()).count(), MAX_STATIONARY_CHANNELS);
        assert_eq!(mask_layers(&got.iter().map(|(_, c)| *c).collect::<Vec<_>>()), 4);
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
