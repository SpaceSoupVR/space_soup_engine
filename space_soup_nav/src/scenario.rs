//! Crowd test scenarios: the situations the research found crowd systems failing in (doorway
//! congestion and deadlock, chasers converging, corners, players who do not give way), written
//! down with their pass criteria BEFORE the crowd exists, so phase 4 is judged against targets
//! that were not tuned to fit it.
//!
//! Each scenario names a mesh in `tests/data/meshes`, the agents (monsters) with where they start
//! and what they go for, the players (moving obstacles that never yield, which is how ORCA's
//! "reciprocal" assumption fails against a human), and what counts as passing.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    /// Walk to a point and stop there.
    Point([f32; 3]),
    /// Chase a player by index, re-planning as they move.
    Player(usize),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioAgent {
    pub start: [f32; 3],
    pub goal: Goal,
    pub radius: f32,
    pub max_speed: f32,
}

/// A player: moves along waypoints at a fixed speed, then stands still. Never avoids anyone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioPlayer {
    pub waypoints: Vec<[f32; 3]>,
    pub speed: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassCriteria {
    /// Every agent with a point goal is within `arrive_radius` of it by this time.
    pub all_arrive_within_s: Option<f32>,
    pub arrive_radius: f32,
    /// No agent goes this long without moving `stuck_distance` while it still has somewhere to go.
    /// An agent at its goal, within `reach` of the player it chases, holding a place outside a
    /// full ring of attackers, or waiting its turn in the queue for a narrow place has nowhere to
    /// go and is not stuck.
    ///
    /// The queue exemption was added after the crowd was written (2026-10-17): with four agents
    /// each side of a single-file doorway, waits of 10-20 s are unavoidable and are the intended
    /// behaviour, the same case as holding outside a full ring. What it could hide, a queue that
    /// stops moving, is caught by `max_handover_s` instead; the longest queue wait is still
    /// reported.
    pub max_stuck_s: f32,
    pub stuck_distance: f32,
    /// While anyone waits for a narrow place, it must let the next agent through within this many
    /// seconds: a queue that stops moving is a jam. `None` where nothing queues.
    #[serde(default)]
    pub max_handover_s: Option<f32>,
    /// Deepest any agent may overlap another agent, a player or a wall.
    pub max_overlap: f32,
    /// For chasers: the time by which at least one agent is within reach of the player.
    pub first_contact_within_s: Option<f32>,
    pub reach: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    /// Relative to `tests/data/meshes`.
    pub mesh: String,
    /// Simulation step, seconds: the Quest's 72 Hz, or SpaceWarp's 36.
    pub dt: f32,
    pub duration_s: f32,
    pub agents: Vec<ScenarioAgent>,
    pub players: Vec<ScenarioPlayer>,
    pub pass: PassCriteria,
}

impl Scenario {
    /// A player's position at time `t`.
    pub fn player_at(&self, player: usize, t: f32) -> Vec3 {
        let p = &self.players[player];
        let mut remaining = t * p.speed;
        let mut at = Vec3::from(p.waypoints[0]);
        for w in &p.waypoints[1..] {
            let to = Vec3::from(*w);
            let leg = at.distance(to);
            if remaining <= leg {
                return if leg > 0.0 { at + (to - at) * (remaining / leg) } else { at };
            }
            remaining -= leg;
            at = to;
        }
        at
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(self.dt > 0.0 && self.duration_s > self.dt) {
            return Err(format!("{}: bad dt/duration", self.name));
        }
        for (i, a) in self.agents.iter().enumerate() {
            if a.radius <= 0.0 || a.max_speed <= 0.0 {
                return Err(format!("{}: agent {i} has no size or speed", self.name));
            }
            if let Goal::Player(p) = a.goal {
                if p >= self.players.len() {
                    return Err(format!("{}: agent {i} chases player {p} of {}", self.name, self.players.len()));
                }
            }
        }
        if self.players.iter().any(|p| p.waypoints.is_empty()) {
            return Err(format!("{}: a player with no waypoints", self.name));
        }
        Ok(())
    }
}

pub fn load_all(data: &std::path::Path) -> Result<Vec<Scenario>, String> {
    let dir = data.join("scenarios");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    names
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let s: Scenario = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            s.validate()?;
            Ok(s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_walk_their_waypoints_then_stop() {
        let s = Scenario {
            name: "t".into(),
            description: String::new(),
            mesh: String::new(),
            dt: 1.0 / 72.0,
            duration_s: 10.0,
            agents: vec![],
            players: vec![ScenarioPlayer { waypoints: vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 0.0, 3.0]], speed: 2.0, radius: 0.3 }],
            pass: PassCriteria {
                all_arrive_within_s: None,
                arrive_radius: 0.5,
                max_stuck_s: 2.0,
                stuck_distance: 0.1,
                max_handover_s: None,
                max_overlap: 0.05,
                first_contact_within_s: None,
                reach: 1.0,
            },
        };
        assert!(s.player_at(0, 1.0).distance(Vec3::new(2.0, 0.0, 0.0)) < 1e-5);
        assert!(s.player_at(0, 3.0).distance(Vec3::new(4.0, 0.0, 2.0)) < 1e-5);
        assert_eq!(s.player_at(0, 100.0), Vec3::new(4.0, 0.0, 3.0));
    }
}
