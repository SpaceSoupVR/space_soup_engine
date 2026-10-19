//! Multiplayer: what every headset must agree on, and what a new host needs.
//!
//! The host runs the monsters and replicates where they are, so paths need not be bit-identical
//! across headsets (DETERMINISM.md). The navigation *data* must be: a headset with an older bake
//! of one room, or a level stitched differently, would put a monster through a wall the others
//! see. So:
//!
//! - **On joining**, each headset sends [`LevelDigest::hash`] (8 bytes). On a mismatch the two
//!   swap full digests and [`LevelDigest::differences`] names what differs (which room, which
//!   join), and the late joiner does not take part in simulating monsters until it has the same
//!   level. A wrong level is refused, never "mostly" used.
//! - **Doors** are runtime state, not part of the digest. The host replicates
//!   [`gate_states`]; a headset that receives them (late join, host change) applies them with
//!   [`apply_gate_states`].
//! - **On a host change**, the new host rebuilds the crowd from the replicated agents with
//!   [`crate::crowd::Crowd::from_snapshot`]: same ids, every agent planning afresh against the new
//!   host's own world.

use glam::IVec3;
use serde::{Deserialize, Serialize};

use crate::asset::fnv1a64;
use crate::world::{NavEvent, NavWorld};

/// One room as placed: which bake, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IslandDigest {
    pub name: String,
    pub content_hash: u64,
    pub turn: u8,
    pub offset: [i32; 3],
}

/// One join between two doorways, and whether it has a door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinDigest {
    pub a: (u32, u16),
    pub b: (u32, u16),
    pub door: bool,
}

/// Everything about an assembled level that must be identical on every headset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelDigest {
    pub islands: Vec<IslandDigest>,
    pub joins: Vec<JoinDigest>,
}

impl LevelDigest {
    pub fn of(world: &NavWorld) -> LevelDigest {
        LevelDigest {
            islands: world
                .islands
                .iter()
                .map(|i| IslandDigest { name: i.asset.name.clone(), content_hash: i.asset.content_hash(), turn: i.turn.0 % 4, offset: i.offset.to_array() })
                .collect(),
            joins: world
                .connections
                .iter()
                .map(|c| JoinDigest { a: (c.a.island, c.a.socket), b: (c.b.island, c.b.socket), door: c.gate.is_some() })
                .collect(),
        }
    }

    /// FNV-1a 64 over a fixed little-endian layout, the same on every platform.
    pub fn hash(&self) -> u64 {
        let mut b = Vec::with_capacity(8 + self.islands.len() * 29 + self.joins.len() * 13);
        b.extend_from_slice(&(self.islands.len() as u32).to_le_bytes());
        for i in &self.islands {
            // The name is covered by the content hash; it is in the digest only to report.
            b.extend_from_slice(&i.content_hash.to_le_bytes());
            b.push(i.turn);
            for c in i.offset {
                b.extend_from_slice(&c.to_le_bytes());
            }
        }
        b.extend_from_slice(&(self.joins.len() as u32).to_le_bytes());
        for j in &self.joins {
            b.extend_from_slice(&j.a.0.to_le_bytes());
            b.extend_from_slice(&j.a.1.to_le_bytes());
            b.extend_from_slice(&j.b.0.to_le_bytes());
            b.extend_from_slice(&j.b.1.to_le_bytes());
            b.push(j.door as u8);
        }
        fnv1a64(&b)
    }

    /// What differs from `other` (the host's), most useful first, at most `limit` lines.
    pub fn differences(&self, other: &LevelDigest, limit: usize) -> Vec<String> {
        let mut out = Vec::new();
        if self.islands.len() != other.islands.len() {
            out.push(format!("{} rooms here, {} on the host", self.islands.len(), other.islands.len()));
        }
        for (k, (a, b)) in self.islands.iter().zip(&other.islands).enumerate() {
            if a.name != b.name {
                out.push(format!("room {k} is {} here, {} on the host", a.name, b.name));
            } else if a.content_hash != b.content_hash {
                out.push(format!("room {k} ({}) is a different bake: {:016x} here, {:016x} on the host", a.name, a.content_hash, b.content_hash));
            } else if a.turn != b.turn || a.offset != b.offset {
                let o = |d: &IslandDigest| IVec3::from_array(d.offset).as_dvec3() / 1000.0;
                out.push(format!("room {k} ({}) is placed at {} turn {} here, {} turn {} on the host", a.name, o(a), a.turn, o(b), b.turn));
            }
        }
        if self.joins.len() != other.joins.len() {
            out.push(format!("{} joins here, {} on the host", self.joins.len(), other.joins.len()));
        }
        for (k, (a, b)) in self.joins.iter().zip(&other.joins).enumerate() {
            if a != b {
                out.push(format!("join {k} is {a:?} here, {b:?} on the host"));
            }
        }
        out.truncate(limit);
        out
    }
}

/// Every door's state, by gate id, for the host to replicate.
pub fn gate_states(world: &NavWorld) -> Vec<bool> {
    world.gates.iter().map(|g| g.open).collect()
}

/// Sets every door to the host's state. Returns the events for the doors that changed, as if
/// each had been opened or shut here. Refused if the host has a different number of doors, since
/// then the levels differ and the digest check should have caught it.
pub fn apply_gate_states(world: &mut NavWorld, states: &[bool]) -> Result<Vec<NavEvent>, String> {
    if states.len() != world.gates.len() {
        return Err(format!("host has {} doors, this level has {}", states.len(), world.gates.len()));
    }
    Ok(states.iter().enumerate().filter_map(|(g, &open)| world.set_gate(g as u32, open)).collect())
}
