//! Debug drawing: the navigation world, paths and crowds as coloured lines and triangles, for any
//! renderer to draw over the scene (each primitive carries its [`Layer`], so a view can toggle
//! them), or as a top-down SVG picture with [`DebugDraw::to_svg`].
//!
//! Millimetres, like the rest of the crate. Nothing here allocates per frame unless asked to: a
//! game builds the static world layers once and rebuilds paths and agents when it wants them.

use std::fmt::Write as _;

use glam::DVec3;

use crate::asset::EdgeLink;
use crate::crowd::{AgentState, Crowd};
use crate::path::{Path, Waypoint};
use crate::world::{LinkKind, NavWorld, PolyRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const FLOOR: Rgba = Rgba([58, 63, 72, 255]);
    pub const FLOOR_HIGH: Rgba = Rgba([90, 97, 112, 255]);
    pub const WALL: Rgba = Rgba([150, 156, 168, 255]);
    pub const DOORWAY: Rgba = Rgba([74, 163, 255, 255]);
    pub const DOOR_OPEN: Rgba = Rgba([224, 138, 44, 255]);
    pub const DOOR_SHUT: Rgba = Rgba([230, 60, 60, 255]);
    pub const LINK: Rgba = Rgba([190, 120, 255, 255]);
    pub const LINK_OFF: Rgba = Rgba([110, 110, 120, 255]);
    pub const CHOKE: Rgba = Rgba([255, 214, 64, 255]);
    pub const PATH: Rgba = Rgba([82, 210, 115, 255]);
    pub const PROBLEM: Rgba = Rgba([255, 40, 90, 255]);

    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }

    /// A distinct shade for the n-th connected area, so separate areas show at a glance.
    pub fn area(n: u32) -> Rgba {
        const SHADES: [[u8; 3]; 8] = [[58, 63, 72], [52, 74, 70], [74, 60, 82], [80, 70, 52], [52, 64, 90], [86, 56, 60], [60, 82, 56], [70, 70, 70]];
        let s = SHADES[n as usize % SHADES.len()];
        Rgba([s[0], s[1], s[2], 255])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Floor,
    Walls,
    Doorways,
    Links,
    Chokes,
    Paths,
    Agents,
    Problems,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DebugLine {
    pub a: DVec3,
    pub b: DVec3,
    pub colour: Rgba,
    pub layer: Layer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DebugTri {
    pub v: [DVec3; 3],
    pub colour: Rgba,
    pub layer: Layer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DebugLabel {
    pub at: DVec3,
    pub text: String,
    pub colour: Rgba,
    pub layer: Layer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorColour {
    /// Ground grey, raised floors (catwalks, upper storeys) lighter.
    ByHeight,
    /// One shade per connected area: anything in a different shade cannot be walked to.
    ByArea,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DebugDraw {
    pub lines: Vec<DebugLine>,
    pub tris: Vec<DebugTri>,
    pub labels: Vec<DebugLabel>,
}

impl DebugDraw {
    pub fn new() -> DebugDraw {
        DebugDraw::default()
    }

    pub fn line(&mut self, a: DVec3, b: DVec3, colour: Rgba, layer: Layer) {
        self.lines.push(DebugLine { a, b, colour, layer });
    }

    /// A horizontal circle as line segments.
    pub fn circle(&mut self, c: DVec3, r: f64, colour: Rgba, layer: Layer) {
        const N: usize = 16;
        // Points of a unit circle, every 22.5 degrees (written out: no trigonometry here).
        const H: f64 = std::f64::consts::FRAC_1_SQRT_2;
        const UNIT: [(f64, f64); N] = [
            (1.0, 0.0), (0.92388, 0.38268), (H, H), (0.38268, 0.92388), (0.0, 1.0), (-0.38268, 0.92388), (-H, H), (-0.92388, 0.38268),
            (-1.0, 0.0), (-0.92388, -0.38268), (-H, -H), (-0.38268, -0.92388), (0.0, -1.0), (0.38268, -0.92388), (H, -H), (0.92388, -0.38268),
        ];
        for k in 0..N {
            let (a, b) = (UNIT[k], UNIT[(k + 1) % N]);
            self.line(c + DVec3::new(a.0 * r, 0.0, a.1 * r), c + DVec3::new(b.0 * r, 0.0, b.1 * r), colour, layer);
        }
    }

    pub fn label(&mut self, at: DVec3, text: impl Into<String>, colour: Rgba, layer: Layer) {
        self.labels.push(DebugLabel { at, text: text.into(), colour, layer });
    }

    /// The world: floor polygons (with their real height detail), walls (the navmesh's outer
    /// edges), doorways (blue: open passage; orange: door open; red: door shut) and typed links.
    pub fn world(world: &mut NavWorld, colour: FloorColour) -> DebugDraw {
        let mut d = DebugDraw::new();
        // Area labels in a fixed order (first appearance), so colours do not depend on root ids.
        let mut seen: Vec<u32> = Vec::new();
        let refs: Vec<PolyRef> = world
            .islands
            .iter()
            .enumerate()
            .flat_map(|(ii, isl)| (0..isl.asset.polys.len()).map(move |p| PolyRef { island: ii as u32, poly: p as u16 }))
            .collect();
        let mut area_of = Vec::with_capacity(refs.len());
        for &p in &refs {
            let c = world.component(p);
            let k = seen.iter().position(|&x| x == c).unwrap_or_else(|| {
                seen.push(c);
                seen.len() - 1
            });
            area_of.push(k as u32);
        }
        for (k, &p) in refs.iter().enumerate() {
            let tris = world.floor_triangles(p);
            let raised = tris.iter().flatten().map(|v| v.y).fold(f64::MIN, f64::max) > world.islands[p.island as usize].offset.y as f64 + 1500.0;
            let c = match colour {
                FloorColour::ByHeight => {
                    if raised {
                        Rgba::FLOOR_HIGH
                    } else {
                        Rgba::FLOOR
                    }
                }
                FloorColour::ByArea => Rgba::area(area_of[k]),
            };
            for t in tris {
                d.tris.push(DebugTri { v: t, colour: c, layer: Layer::Floor });
            }
            let isl = &world.islands[p.island as usize];
            let v = world.poly_dverts(p);
            let n = v.len();
            for (e, link) in isl.asset.polys[p.poly as usize].edges.iter().enumerate() {
                let (a, b) = (v[e], v[(e + 1) % n]);
                match link {
                    EdgeLink::Border => d.line(a, b, Rgba::WALL, Layer::Walls),
                    EdgeLink::Socket(_) => {
                        let portals: Vec<_> = isl.links_from[p.poly as usize]
                            .iter()
                            .map(|&l| &world.links[l as usize])
                            .filter(|l| l.kind == LinkKind::Portal && l.from_edge == Some(e as u8))
                            .collect();
                        if portals.is_empty() {
                            // A doorway joined to nothing (or impassable for this size): a wall.
                            d.line(a, b, Rgba::WALL, Layer::Walls);
                        }
                        for l in portals {
                            let colour = match l.gate {
                                None => Rgba::DOORWAY,
                                Some(g) if world.gates[g as usize].open => Rgba::DOOR_OPEN,
                                Some(_) => Rgba::DOOR_SHUT,
                            };
                            d.line(l.from_segment[0].as_dvec3(), l.from_segment[1].as_dvec3(), colour, Layer::Doorways);
                        }
                    }
                    EdgeLink::Internal(_) => {}
                }
            }
        }
        for l in &world.links {
            if let LinkKind::Typed(kind) = &l.kind {
                let colour = if l.enabled { Rgba::LINK } else { Rgba::LINK_OFF };
                let (a, b) = ((l.from_segment[0] + l.from_segment[1]).as_dvec3() / 2.0, (l.to_segment[0] + l.to_segment[1]).as_dvec3() / 2.0);
                d.line(a, b, colour, Layer::Links);
                d.circle(a, 150.0, colour, Layer::Links);
                d.label((a + b) / 2.0, format!("{kind:?}"), colour, Layer::Links);
            }
        }
        d
    }

    /// The narrow places a crowd takes turns at.
    pub fn add_chokes(&mut self, crowd: &Crowd) {
        for c in &crowd.chokes {
            let along = DVec3::new(c.along.x, 0.0, c.along.y) * 500.0;
            let normal = DVec3::new(c.normal.x, 0.0, c.normal.y) * 500.0;
            self.line(c.centre - along, c.centre + along, Rgba::CHOKE, Layer::Chokes);
            self.line(c.centre - normal, c.centre + normal, Rgba::CHOKE, Layer::Chokes);
        }
    }

    /// A path: its straight legs, its corners, and its typed-link jumps.
    pub fn add_path(&mut self, path: &Path, colour: Rgba) {
        for w in path.waypoints.windows(2) {
            let style = if matches!(w[1], Waypoint::LinkEnd(..)) { Rgba::LINK } else { colour };
            self.line(w[0].pos(), w[1].pos(), style, Layer::Paths);
        }
        for w in &path.waypoints {
            self.circle(w.pos(), 80.0, colour, Layer::Paths);
        }
    }

    /// A crowd: each agent's body coloured by what it is doing, a line to what it steers at, and
    /// its velocity.
    pub fn add_crowd(&mut self, crowd: &Crowd) {
        for a in &crowd.agents {
            let colour = match a.state {
                AgentState::Moving => Rgba([255, 77, 94, 255]),
                AgentState::Attacking => Rgba([255, 20, 20, 255]),
                AgentState::Holding => Rgba([255, 160, 120, 255]),
                AgentState::Waiting => Rgba::CHOKE,
                AgentState::Arrived => Rgba([160, 160, 160, 255]),
                AgentState::Idle => Rgba([120, 120, 120, 255]),
                AgentState::NoPath => Rgba::PROBLEM,
            };
            self.circle(a.pos, a.params.radius_mm, colour, Layer::Agents);
            if let Some(t) = a.target {
                self.line(a.pos, t, Rgba([colour.0[0], colour.0[1], colour.0[2], 110]), Layer::Agents);
            }
            self.line(a.pos, a.pos + a.desired * 0.5, colour, Layer::Agents);
        }
        for p in &crowd.players {
            self.circle(p.pos, p.radius_mm, Rgba::PATH, Layer::Agents);
        }
    }

    /// A problem marker: a red cross and a label.
    pub fn add_problem(&mut self, at: DVec3, text: impl Into<String>) {
        let (dx, dz) = (DVec3::new(400.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 400.0));
        self.line(at - dx - dz, at + dx + dz, Rgba::PROBLEM, Layer::Problems);
        self.line(at - dx + dz, at + dx - dz, Rgba::PROBLEM, Layer::Problems);
        self.label(at, text, Rgba::PROBLEM, Layer::Problems);
    }

    /// A top-down picture: +X right, +Z down, `px_per_m` pixels to the metre, layers drawn in order.
    pub fn to_svg(&self, px_per_m: f64, caption: &str) -> String {
        let pts = self.tris.iter().flat_map(|t| t.v).chain(self.lines.iter().flat_map(|l| [l.a, l.b])).chain(self.labels.iter().map(|l| l.at));
        let (lo, hi) = pts.fold((DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        if lo.x > hi.x {
            return String::from("<svg xmlns=\"http://www.w3.org/2000/svg\"/>\n");
        }
        let s = px_per_m / 1000.0;
        let pad = 20.0;
        let (w, h) = ((hi.x - lo.x) * s + 2.0 * pad, (hi.z - lo.z) * s + 2.0 * pad + 28.0);
        let px = |v: DVec3| ((v.x - lo.x) * s + pad, (v.z - lo.z) * s + pad);
        let mut out = String::new();
        let _ = writeln!(out, r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}"><rect width="100%" height="100%" fill="#14161a"/>"##);
        let mut tris: Vec<&DebugTri> = self.tris.iter().collect();
        tris.sort_by_key(|t| t.layer);
        for t in tris {
            let p: Vec<String> = t.v.iter().map(|&q| {
                let (x, y) = px(q);
                format!("{x:.1},{y:.1}")
            }).collect();
            let _ = writeln!(out, r#"<polygon points="{}" fill="{}" stroke="{}" stroke-width="0.4"/>"#, p.join(" "), t.colour.hex(), t.colour.hex());
        }
        let mut lines: Vec<&DebugLine> = self.lines.iter().collect();
        lines.sort_by_key(|l| l.layer);
        for l in lines {
            let ((x1, y1), (x2, y2)) = (px(l.a), px(l.b));
            let width = match l.layer {
                Layer::Walls => 0.8,
                Layer::Doorways => 3.0,
                Layer::Problems => 2.5,
                _ => 1.4,
            };
            let opacity = l.colour.0[3] as f64 / 255.0;
            let _ = writeln!(out, r#"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{}" stroke-opacity="{opacity:.2}" stroke-width="{width}" stroke-linecap="round"/>"#, l.colour.hex());
        }
        for l in &self.labels {
            let (x, y) = px(l.at);
            let text = l.text.replace('&', "&amp;").replace('<', "&lt;");
            let _ = writeln!(out, r#"<text x="{x:.1}" y="{y:.1}" fill="{}" font-family="sans-serif" font-size="11">{text}</text>"#, l.colour.hex());
        }
        let caption = caption.replace('&', "&amp;").replace('<', "&lt;");
        let _ = writeln!(out, r##"<text x="{pad}" y="{:.0}" fill="#cfd3da" font-family="sans-serif" font-size="14">{caption}</text>"##, h - 10.0);
        out.push_str("</svg>\n");
        out
    }
}
