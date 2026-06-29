//! Procedural cube hand for controller input.
//!
//! When only a controller is available (no finger tracking), this builds a
//! posable hand out of cuboids: a rigid welded palm plus five fingers, each a
//! short chain of segments that curl at every joint. Curl is driven by analog
//! inputs (e.g. trigger / grip pressure) in the 0.0..=1.0 range.
//!
//! Everything is expressed in a palm-local frame and then transformed into
//! world space through an anchor `Transform` (the controller grip pose). The
//! per-joint rotation used here is the same operation a skinned skeleton uses
//! to bend its bones, so the logic transfers directly to a real hand model:
//! swap each cube for a bone and drive the same joint rotations.
//!
//! Palm-local frame:
//!   -Z  = direction the fingers point (forward)
//!   +Y  = back of the hand
//!   -Y  = palm side (fingers curl toward here)
//!   +X  = thumb side for the right hand (mirrored for the left)

use glam::{Vec3, Quat};

use crate::events::Hand;
use crate::rig::Transform;
use crate::scene::Color3;

/// A single solid cuboid of the hand, already in world space.
#[derive(Debug, Clone, Copy)]
pub struct HandCuboid {
    pub position:  Vec3,
    pub half_size: Vec3,
    pub rotation:  Quat,
    pub color:     Color3,
}

/// Per-finger curl amount, 0.0 (straight) to 1.0 (fully folded).
#[derive(Debug, Clone, Copy, Default)]
pub struct FingerCurls {
    pub thumb:  f32,
    pub index:  f32,
    pub middle: f32,
    pub ring:   f32,
    pub little: f32,
}

const SKIN: Color3 = Color3(230, 190, 160, 255);
const PALM: Color3 = Color3(205, 170, 145, 255);

/// Maximum bend applied at each finger joint at full curl, in degrees.
const JOINT_BEND_DEG: f32 = 60.0;

/// Build the full hand for `hand`, anchored to the grip `Transform`.
pub fn build_hand(anchor: Transform, hand: Hand, curls: &FingerCurls) -> Vec<HandCuboid> {
    let mut out = Vec::with_capacity(20);

    // Mirror the layout across the palm's X axis. The thumb sits on the +X
    // side for the left hand and the -X side for the right hand.
    let s = match hand { Hand::Right => -1.0, Hand::Left => 1.0 };

    // ── Palm: a few welded cuboids, rigid relative to the grip anchor ──────
    push_box(&mut out, &anchor, Vec3::new(0.0, 0.0, 0.0),
             Vec3::new(0.046, 0.015, 0.045), Quat::IDENTITY, PALM);          // main palm
    push_box(&mut out, &anchor, Vec3::new(0.0, 0.0, 0.050),
             Vec3::new(0.032, 0.014, 0.018), Quat::IDENTITY, PALM);          // wrist / heel
    push_box(&mut out, &anchor, Vec3::new(0.034 * s, 0.0, 0.022),
             Vec3::new(0.013, 0.013, 0.020), Quat::from_rotation_y(-0.6 * s), PALM); // thumb mound

    // ── Four fingers: chains that curl at each joint ───────────────────────
    // Knuckles sit along the front edge of the palm (-Z), spread along X.
    let front = -0.045;
    let fingers: [(f32, [f32; 3], f32, f32); 4] = [
        // (x position,  segment lengths,            thickness, curl)
        ( 0.032 * s, [0.032, 0.024, 0.019], 0.009, curls.index),
        ( 0.011 * s, [0.035, 0.026, 0.021], 0.009, curls.middle),
        (-0.011 * s, [0.032, 0.024, 0.019], 0.009, curls.ring),
        (-0.032 * s, [0.026, 0.019, 0.016], 0.008, curls.little),
    ];
    for (x, lengths, thick, curl) in fingers {
        let knuckle = Vec3::new(x, 0.0, front);
        build_finger(&mut out, &anchor, knuckle, Quat::IDENTITY, &lengths, thick, curl, SKIN);
    }

    // ── Thumb: protrudes out to the side, angled forward ───────────────────
    // It sits low on the palm (toward the heel, +Z) and points strongly along
    // +X for the right hand — mirrored to -X for the left — so it sticks out
    // to the side instead of folding across the palm.
    let thumb_knuckle = Vec3::new(0.042 * s, 0.0, 0.018);
    let thumb_rest    = Quat::from_rotation_y(-1.1 * s) * Quat::from_rotation_x(-0.2);
    build_finger(&mut out, &anchor, thumb_knuckle, thumb_rest,
                 &[0.026, 0.020], 0.010, curls.thumb, SKIN);

    out
}

/// Walk a finger chain from `knuckle`, bending each joint by `curl`.
fn build_finger(
    out:     &mut Vec<HandCuboid>,
    anchor:  &Transform,
    knuckle: Vec3,
    rest:    Quat,
    lengths: &[f32],
    thick:   f32,
    curl:    f32,
    color:   Color3,
) {
    // Fingers fold toward the palm (-Y): bend around the local -X hinge axis.
    let bend_axis = Vec3::NEG_X;
    let forward   = Vec3::NEG_Z;
    let angle     = (JOINT_BEND_DEG * curl).to_radians();

    let mut rot = rest;
    let mut pos = knuckle;
    for &len in lengths {
        // Each joint adds another increment of curl down the chain.
        rot *= Quat::from_axis_angle(bend_axis, angle);

        // Centre the segment cuboid half a length ahead of the joint.
        let center = pos + rot * (forward * (len * 0.5));
        let (wp, wr) = place(anchor, center, rot);
        out.push(HandCuboid {
            position:  wp,
            half_size: Vec3::new(thick, thick, len * 0.5),
            rotation:  wr,
            color,
        });

        // Advance to the next joint at the far end of this segment.
        pos += rot * (forward * len);
    }
}

/// Emit one rigid box at a palm-local pose.
fn push_box(
    out:       &mut Vec<HandCuboid>,
    anchor:    &Transform,
    local_pos: Vec3,
    half_size: Vec3,
    local_rot: Quat,
    color:     Color3,
) {
    let (position, rotation) = place(anchor, local_pos, local_rot);
    out.push(HandCuboid { position, half_size, rotation, color });
}

/// Map a palm-local pose into world space through the anchor transform.
fn place(anchor: &Transform, local_pos: Vec3, local_rot: Quat) -> (Vec3, Quat) {
    (
        anchor.position + anchor.rotation * local_pos,
        anchor.rotation * local_rot,
    )
}
