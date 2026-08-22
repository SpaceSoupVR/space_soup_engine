//! Regions of space that notice you without stopping you.
//!
//! WHAT A TRIGGER VOLUME IS
//!
//! A shape that reports who is inside it and does something about it, while
//! being neither drawn nor solid. Walking into one opens the door ahead;
//! leaving it closes it. It is the standard way level logic is wired to places
//! rather than to objects, and every level editor since Quake has had one --
//! Source calls them `trigger_multiple` and paints them with a tools texture so
//! they are visible while authoring and absent from the game.
//!
//! WHY IT IS A THIRD STATE AND NOT A SECOND FLAG
//!
//! A collider can be one of three things, and only three: it BLOCKS, it
//! REPORTS, or it does nothing. `rigid_body.enabled` already separates the
//! first from the third. This is the middle one, and it is genuinely different
//! from both -- a volume that blocked would be a wall, and one that did not
//! report would be nothing at all.
//!
//! HOW OCCUPANCY IS DECIDED, AND WHAT THAT COSTS
//!
//! One POINT per player -- roughly their chest -- tested against the volume's
//! own shape. Not the player's bounding box against the volume's bounding box,
//! which is what the teleportal pads do, because a box around a rotated or
//! L-shaped brush juts out at the corners and the door would open while you
//! were still outside it.
//!
//! The consequence to know: a point cannot half-overlap, so enter and exit are
//! unambiguous and the same rule can be drawn in the editor. The cost is that
//! occupancy is sampled per tick and not swept, so something moving faster than
//! its own width per frame can cross a thin volume without ever being inside
//! one. Make zones thicker than the fastest thing that must not miss them.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::brush::{self, BrushSolid};
use crate::physics::point_in_obb;
use crate::scene::GameObject;

fn default_true() -> bool {
    true
}

/// What a volume does when it becomes occupied or empty.
///
/// A deliberately small vocabulary, and every entry is something a LEVEL wants
/// rather than something a weapon wants. Anything beyond it is a script reading
/// `is_occupied`, which costs one line and cannot be outgrown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VolumeAction {
    /// Record a state. The same store scripts and part triggers use, so a state
    /// set by walking somewhere is indistinguishable from one set by a weapon.
    SetVar { name: String, value: String },
    AddVar { name: String, delta: f64 },
    /// Show or hide another object -- a hologram that appears as you approach.
    SetObjectVisible { id: String, visible: bool },
    /// Make another object solid or passable -- the door, the force field, the
    /// invisible wall that stops you leaving the arena once the fight starts.
    SetObjectSolid { id: String, solid: bool },
    PlaySound { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerVolumeDef {
    /// Whether the volume is watching.
    ///
    /// Same reasoning as `rigid_body.enabled`: an off state that keeps its
    /// configuration, so a zone can be armed by another trigger rather than
    /// having to exist or not exist.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// A state set to `"1"` while anyone is inside and `"0"` when empty.
    ///
    /// The cheapest useful thing a volume can do, and the one that composes:
    /// clip conditions already read vars, scripts already read vars, and other
    /// volumes can react to it. A zone with only this and no actions is a
    /// perfectly ordinary zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_enter: Vec<VolumeAction>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_exit: Vec<VolumeAction>,

    /// Fire `on_enter` at most once per scene load.
    ///
    /// The difference between a door that reopens every time you walk back and
    /// an ambush that happens once. Source needs two different entities for
    /// this; here it is a checkbox.
    #[serde(default)]
    pub once: bool,
}

impl Default for TriggerVolumeDef {
    fn default() -> Self {
        Self {
            enabled: true,
            var: None,
            on_enter: Vec::new(),
            on_exit: Vec::new(),
            once: false,
        }
    }
}

/// The shape a volume actually tests against, resolved once per scene load.
///
/// Brush solids are the output of a CSG evaluation, which is far too expensive
/// to redo every tick for every zone. Resolving at load also means a volume's
/// shape cannot drift from what the author saw, and there is no per-frame
/// allocation in the occupancy pass at all.
#[derive(Debug, Clone)]
pub enum VolumeShape {
    /// A rotated box: the object's own cuboid.
    Obb {
        center: Vec3,
        half_size: Vec3,
        rotation: glam::Quat,
    },
    /// One or more convex solids -- a brush, exactly as authored, including the
    /// angled and hollowed ones a bounding box would get wrong.
    Solids(Vec<BrushSolid>),
}

impl VolumeShape {
    /// The shape of an object, preferring its brush over its reported bounds.
    ///
    /// A brush object's `cuboid` is a REPORT of its bounds rather than its
    /// shape, so using it for a wedge or an L would claim ground the author can
    /// see is outside the zone.
    pub fn of(object: &GameObject) -> Self {
        if let Some(def) = &object.brush {
            let solids = def.evaluate();
            if !solids.is_empty() {
                return Self::Solids(solids);
            }
        }
        Self::Obb {
            center: object.cuboid.position,
            half_size: object.cuboid.half_size,
            rotation: object.cuboid.rotation,
        }
    }

    pub fn contains(&self, p: Vec3) -> bool {
        match self {
            Self::Obb {
                center,
                half_size,
                rotation,
            } => point_in_obb(p, *center, *half_size, *rotation),
            // Any solid, not all: subtracting a doorway from a brush leaves
            // several convex pieces that together are the authored shape.
            //
            // Brush geometry is f64 throughout -- it has to be, because the
            // editor computes the same planes in JavaScript and the two are
            // held to a shared checksum. Widening here rather than narrowing
            // there keeps that agreement.
            Self::Solids(solids) => {
                let at: brush::Vec3 = [p.x as f64, p.y as f64, p.z as f64];
                solids.iter().any(|s| brush::contains_point(s, at, 0.0))
            }
        }
    }
}
