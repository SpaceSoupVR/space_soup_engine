use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaterDef {

    pub height: f32,

    pub bounds: [f32; 4],

    #[serde(default = "default_shallow")]
    pub shallow: [u8; 3],

    #[serde(default = "default_deep")]
    pub deep: [u8; 3],

    #[serde(default = "default_depth_scale")]
    pub depth_scale: f32,

    #[serde(default = "default_shore_fade")]
    pub shore_fade: f32,

    #[serde(default = "default_wind_speed")]
    pub wind_speed: f32,

    #[serde(default = "default_wind_direction")]
    pub wind_direction: f32,

    #[serde(default = "default_fetch")]
    pub fetch: f32,

    #[serde(default = "default_choppiness")]
    pub choppiness: f32,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub skirt: f32,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub swash: f32,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub swell: f32,
    #[serde(default = "default_gloss", skip_serializing_if = "is_one")]
    pub gloss: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mud: f32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wade_depth: Option<f32>,

    /// A MIRROR: a small calm body -- a pond under a bank -- whose
    /// reflection is the scene drawn mirrored in its surface when it is the
    /// largest such body on the screen (the renderer's `planar_water`), not
    /// the ground map's top-down picture, which smears a steep bank. Leave
    /// it off for the sea: its waves break a mirror up anyway.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
}

pub const DEFAULT_WADE_DEPTH: f32 = 0.6;

fn default_shallow() -> [u8; 3] { [90, 150, 140] }
fn default_deep() -> [u8; 3] { [10, 40, 70] }
fn default_depth_scale() -> f32 { 4.0 }
fn default_shore_fade() -> f32 { 0.35 }
fn default_wind_speed() -> f32 { 3.0 }
fn default_wind_direction() -> f32 { 30.0 }
fn default_fetch() -> f32 { 100.0 }
fn default_choppiness() -> f32 { 0.6 }
fn is_zero(v: &f32) -> bool { *v == 0.0 }
fn default_gloss() -> f32 { 1.0 }
fn is_one(v: &f32) -> bool { *v == 1.0 }

impl WaterDef {

    #[must_use]
    pub fn extent(&self) -> (f32, f32, f32, f32) {
        let [a, b, c, d] = self.bounds;
        (a.min(c), b.min(d), a.max(c), b.max(d))
    }

    #[must_use]
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (x0, z0, x1, z1) = self.extent();
        x >= x0 && x <= x1 && z >= z0 && z <= z1
    }

    #[must_use]
    pub fn depth_over(&self, ground_y: f32) -> f32 {
        (self.height - ground_y).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterVertex {
    pub position: [f32; 3],

    pub depth: f32,
}

pub const WATER_GRID_STEP: f32 = 0.5;

pub const WATER_GRID_MAX_STEPS: f32 = 200.0;

pub const OPEN_WATER_DEPTH: f32 = 100.0;

const SKIRT_RINGS: usize = 8;

struct Grid {
    step: f32,
    nx: usize,
    nz: usize,
    verts: Vec<WaterVertex>,
    wet: Vec<bool>,
}

fn grid(def: &WaterDef, ground: impl Fn(f32, f32) -> Option<f32>) -> Option<Grid> {
    let (x0, z0, x1, z1) = def.extent();
    let step = WATER_GRID_STEP.max((x1 - x0).max(z1 - z0) / WATER_GRID_MAX_STEPS);
    let nx = (((x1 - x0) / step).ceil() as usize).max(1) + 1;
    let nz = (((z1 - z0) / step).ceil() as usize).max(1) + 1;

    let reach = def.swash.max(0.0);

    let mut verts = Vec::with_capacity(nx * nz);
    let mut any_wet = def.skirt > 0.0;
    for iz in 0..nz {
        for ix in 0..nx {

            let x = if ix + 1 == nx { x1 } else { x0 + ix as f32 * step };
            let z = if iz + 1 == nz { z1 } else { z0 + iz as f32 * step };

            let depth = match ground(x, z) {
                Some(g) => def.height - g,
                None => OPEN_WATER_DEPTH,
            };
            if depth > 0.0 {
                any_wet = true;
            }
            verts.push(WaterVertex { position: [x, def.height, z], depth });
        }
    }
    if !any_wet {
        return None;
    }

    let wet: Vec<bool> = if def.skirt > 0.0 {
        let mut reached = vec![false; verts.len()];
        let mut frontier: Vec<usize> = (0..verts.len())
            .filter(|&i| {
                let (ix, iz) = (i % nx, i / nx);
                (ix == 0 || iz == 0 || ix + 1 == nx || iz + 1 == nz) && verts[i].depth > -reach
            })
            .collect();
        for &i in &frontier {
            reached[i] = true;
        }
        while let Some(i) = frontier.pop() {
            let (ix, iz) = (i % nx, i / nx);
            let mut visit = |j: usize| {
                if !reached[j] && verts[j].depth > -reach {
                    reached[j] = true;
                    frontier.push(j);
                }
            };
            if ix > 0 { visit(i - 1); }
            if ix + 1 < nx { visit(i + 1); }
            if iz > 0 { visit(i - nx); }
            if iz + 1 < nz { visit(i + nx); }
        }
        reached
    } else {
        verts.iter().map(|v| v.depth > -reach).collect()
    };
    Some(Grid { step, nx, nz, verts, wet })
}

pub fn build_surface(
    def: &WaterDef,
    ground: impl Fn(f32, f32) -> Option<f32>,
) -> Option<(Vec<WaterVertex>, Vec<u32>)> {
    let Grid { step, nx, nz, mut verts, wet } = grid(def, ground)?;
    let (x0, z0, x1, z1) = def.extent();

    let mut indices = Vec::with_capacity((nx - 1) * (nz - 1) * 6);
    for iz in 0..nz.saturating_sub(1) {
        for ix in 0..nx.saturating_sub(1) {
            let a = (iz * nx + ix) as u32;
            let b = a + 1;
            let c = a + nx as u32;
            let d = c + 1;

            for tri in [[a, c, b], [b, c, d]] {
                if tri.iter().any(|&i| wet[i as usize]) {
                    indices.extend_from_slice(&tri);
                }
            }
        }
    }

    if def.skirt > 0.0 {
        let mut ring: Vec<u32> = (0..nx).map(|ix| ix as u32).collect();
        ring.extend((1..nz).map(|iz| (iz * nx + nx - 1) as u32));
        ring.extend((0..nx - 1).rev().map(|ix| ((nz - 1) * nx + ix) as u32));
        ring.extend((1..nz - 1).rev().map(|iz| (iz * nx) as u32));
        let (cx, cz) = (f32::midpoint(x0, x1), f32::midpoint(z0, z1));
        let half = ((x1 - x0).min(z1 - z0) * 0.5).max(step);
        let far = 1.0 + def.skirt / half;
        let mut inner = ring.clone();
        for k in 1..=SKIRT_RINGS {
            let s = far.powf(k as f32 / SKIRT_RINGS as f32);
            let base = verts.len() as u32;
            for &i in &ring {
                let [x, _, z] = verts[i as usize].position;
                verts.push(WaterVertex {
                    position: [cx + (x - cx) * s, def.height, cz + (z - cz) * s],
                    depth: OPEN_WATER_DEPTH,
                });
            }
            for j in 0..ring.len() {
                let jn = (j + 1) % ring.len();
                let (a, b) = (inner[j], inner[jn]);
                let (c, d) = (base + j as u32, base + jn as u32);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
            inner = (base..base + ring.len() as u32).collect();
        }
    }
    if indices.is_empty() {
        return None;
    }
    Some((verts, indices))
}

#[derive(Debug, Clone)]
pub struct WaterFootprint {
    height: f32,
    wade: f32,
    extent: (f32, f32, f32, f32),
    step: f32,
    nx: usize,
    nz: usize,
    wet: Vec<bool>,
    skirt: f32,
}

impl WaterFootprint {

    pub fn new(def: &WaterDef, ground: impl Fn(f32, f32) -> Option<f32>) -> Option<Self> {
        let Grid { step, nx, nz, wet, .. } = grid(def, ground)?;
        Some(Self {
            height: def.height,
            wade: def.max_wade_depth.unwrap_or(DEFAULT_WADE_DEPTH),
            extent: def.extent(),
            step,
            nx,
            nz,
            wet,
            skirt: def.skirt.max(0.0),
        })
    }

    /// The still surface's height, where this water stands at all.
    #[must_use]
    pub fn surface_height(&self) -> Option<f32> {
        (self.wet.iter().any(|&w| w) || self.skirt > 0.0).then_some(self.height)
    }

    /// The deepest of this water a walk goes into, metres.
    #[must_use]
    pub fn wade_depth(&self) -> f32 {
        self.wade
    }

    #[must_use]
    pub fn surface_at(&self, x: f32, z: f32) -> Option<f32> {
        let (x0, z0, x1, z1) = self.extent;
        if x < x0 || z < z0 || x > x1 || z > z1 {
            let out = (x0 - x).max(x - x1).max(z0 - z).max(z - z1);
            return (out <= self.skirt).then_some(self.height);
        }

        let ix = (((x - x0) / self.step) as usize).min(self.nx - 2);
        let iz = (((z - z0) / self.step) as usize).min(self.nz - 2);
        let a = iz * self.nx + ix;
        [a, a + 1, a + self.nx, a + self.nx + 1].iter().any(|&i| self.wet[i]).then_some(self.height)
    }
}

/// SOMETHING BREAKING A WATER SURFACE from above: what a splash is made by.
/// Raised by [`WaterWatch::see`] for the bodies physics moves
/// (`PhysicsWorld::take_water_entries`), and by the headset for what it only
/// sees move; drawn by the renderer from these numbers alone
/// (`space_soup::renderer::effects::Splash`).
#[derive(Debug, Clone, PartialEq)]
pub struct WaterEntry {
    pub id: String,
    /// Where it met the still surface, world.
    pub position: glam::Vec3,
    /// How it was moving as it did, m/s.
    pub velocity: glam::Vec3,
    /// The radius of a disc as big as its cross-section there, metres.
    pub radius: f32,
    pub mass: f32,
}

/// The slowest thing that splashes going in, m/s down: slower, it is lowered.
pub const ENTRY_SPEED: f32 = 0.6;

/// AN ENTRY THE WORLD STILL TELLS OF: one a simulation saw in the last
/// [`WATER_ENTRY_KEPT`] seconds (`GameRuntime::water_entries`), with a number
/// that is its alone and how long ago it was. An entry is a moment, and a
/// snapshot of the world is a state: told once it would be lost with the one
/// snapshot that carried it, so it is told in every snapshot for a while, and
/// whoever draws it knows by its number that they have, and by its age when
/// it was.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentWaterEntry {
    pub serial: u64,
    /// Seconds since it went in, as of the simulation's last step.
    pub age: f32,
    pub entry: WaterEntry,
}

/// How long an entry is told of, seconds: thirty snapshots at the server's
/// sixty a second, and well under the time its splash is in the air.
pub const WATER_ENTRY_KEPT: f32 = 0.5;

impl WaterEntry {
    /// The energy it brought to the water, joules: half its mass by its
    /// speed squared. What a splash is as big as, what a going-in again is
    /// measured against ([`WaterState`]) and what decides which entries are
    /// told of when there are too many ([`keep_most_energetic`]).
    #[must_use]
    pub fn energy(&self) -> f32 {
        0.5 * self.mass.max(0.0) * self.velocity.length_squared()
    }
}

/// A body that goes in again soon after it went in splashes only if it comes
/// with at least this share of the energy it had then...
pub const REENTRY_SHARE: f32 = 0.5;
/// ...a share that halves with every this many seconds since: the water it
/// churned has settled, and a small plop shows on it again.
pub const REENTRY_HALF_LIFE: f32 = 0.4;

/// ONE BODY AND THE WATER: whether it is in, and what it last went in with.
///
/// A body is in from the moment its underside is at the surface until it has
/// come clear of it again -- by a quarter of its own width, and no less than
/// two centimetres. Without that, one floating at the waterline, bobbing a
/// millimetre either way, went in again every time it dipped: a splash a
/// tenth of a second, for as long as it floated.
///
/// And one that does come clear and goes in again -- a stone skipping, a
/// crate thrown up by its own buoyancy, a rock bouncing off a shallow bed --
/// splashes again only by what it brings: at least [`REENTRY_SHARE`] of the
/// energy of its last splash, less as that splash settles
/// ([`REENTRY_HALF_LIFE`]). A skip keeps most of its speed and splashes; a bob
/// keeps a tenth and does not. By the energy, not by a timer: two real
/// strikes a tenth of a second apart are both seen.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct WaterState {
    /// Its underside when last seen; none before it has been.
    under: Option<f32>,
    is_in: bool,
    /// When it last splashed, and with what energy.
    last: Option<(f64, f32)>,
}

impl WaterState {
    /// Whether it was in the water when last seen.
    #[must_use]
    pub fn is_in(&self) -> bool {
        self.is_in
    }

    /// The body as it is at `now` (seconds on any steady clock; none where
    /// there is no clock, and then every going-in counts): its `centre`, the
    /// half extents of the box round it along the world's axes, how it is
    /// moving and its mass; `surface` the still water's height where it is,
    /// none over dry ground. Returns its entry (with no id) on the step it
    /// goes in.
    pub fn see(&mut self, now: Option<f64>, centre: glam::Vec3, half: glam::Vec3, velocity: glam::Vec3, mass: f32, surface: Option<f32>) -> Option<WaterEntry> {
        let under = centre.y - half.y.abs();
        let before = self.under.map(|was| (was, self.is_in));
        self.under = Some(under);
        let Some(surface) = surface else {
            self.is_in = false;
            return None;
        };
        let radius = (4.0 * half.x.abs() * half.z.abs() / std::f32::consts::PI).sqrt();
        let clear = (0.5 * radius).max(0.02);
        let was_in = before.is_some_and(|(_, was_in)| was_in);
        self.is_in = if was_in { under <= surface + clear } else { under <= surface };
        let came_down = before.is_some_and(|(was, was_in)| !was_in && was > surface);
        if !(self.is_in && came_down && -velocity.y >= ENTRY_SPEED) {
            return None;
        }
        let entry = WaterEntry { id: String::new(), position: glam::Vec3::new(centre.x, surface, centre.z), velocity, radius, mass };
        if let (Some(now), Some((when, energy))) = (now, self.last) {
            let since = (now - when).max(0.0) as f32;
            if entry.energy() < REENTRY_SHARE * energy * 0.5f32.powf(since / REENTRY_HALF_LIFE) {
                return None;
            }
        }
        if let Some(now) = now {
            self.last = Some((now, entry.energy()));
        }
        Some(entry)
    }
}

/// THE MOST ENERGETIC `cap` OF `entries`, in the order they were in: what is
/// told of when more things strike the water at once than anyone can draw. The
/// least energetic go first -- a hundred pebbles and a boulder is the boulder
/// and the biggest pebbles -- and ties keep the earlier.
pub fn keep_most_energetic(entries: &mut Vec<WaterEntry>, cap: usize) {
    if entries.len() <= cap {
        return;
    }
    if cap == 0 {
        entries.clear();
        return;
    }
    let mut energies: Vec<f32> = entries.iter().map(WaterEntry::energy).collect();
    energies.sort_by(|a, b| b.total_cmp(a));
    let floor = energies[cap - 1];
    let mut ties = cap - energies.iter().filter(|e| **e > floor).count();
    entries.retain(|e| {
        let energy = e.energy();
        if energy > floor {
            true
        } else if energy == floor && ties > 0 {
            ties -= 1;
            true
        } else {
            false
        }
    });
}

/// The most entries one step of a simulation tells of
/// (`PhysicsWorld::step`), and the most a world tells of at once
/// (`GameRuntime::water_entries`: in every snapshot for half a second).
pub const MAX_ENTRIES_A_STEP: usize = 8;
pub const MAX_ENTRIES_TOLD: usize = 32;

/// WHICH BODIES ARE IN THE WATER, by name, so each going in is seen once: a
/// [`WaterState`] each, for someone who only sees things move (the headset,
/// for a server that does not tell). No clock, so every going-in counts.
#[derive(Debug, Default, Clone)]
pub struct WaterWatch {
    bodies: std::collections::HashMap<String, WaterState>,
}

impl WaterWatch {
    /// A body as it is now: its `centre`, the half extents of the box round
    /// it along the world's axes, how it is moving and its mass; `surface`
    /// the still water's height where it is, none over dry ground. Returns
    /// its entry, on the step it goes in.
    pub fn see(&mut self, id: &str, centre: glam::Vec3, half: glam::Vec3, velocity: glam::Vec3, mass: f32, surface: Option<f32>) -> Option<WaterEntry> {
        if !self.bodies.contains_key(id) {
            self.bodies.insert(id.to_string(), WaterState::default());
        }
        let state = self.bodies.get_mut(id)?;
        state.see(None, centre, half, velocity, mass, surface).map(|mut e| {
            e.id = id.to_string();
            e
        })
    }

    /// Forget every body not in `seen`.
    pub fn keep(&mut self, seen: impl Fn(&str) -> bool) {
        self.bodies.retain(|id, _| seen(id));
    }
}

/// The half extents, along the world's axes, of the box round a box of
/// `half_size` turned by `rotation`.
#[must_use]
pub fn world_half_extents(half_size: glam::Vec3, rotation: glam::Quat) -> glam::Vec3 {
    let m = glam::Mat3::from_quat(rotation);
    let h = half_size.abs();
    m.x_axis.abs() * h.x + m.y_axis.abs() * h.y + m.z_axis.abs() * h.z
}

#[cfg(test)]
mod tests {
    use super::*;

    fn water() -> WaterDef {
        WaterDef {
            height: 2.0,
            bounds: [-10.0, -10.0, 10.0, 10.0],
            shallow: default_shallow(),
            deep: default_deep(),
            depth_scale: default_depth_scale(),
            shore_fade: default_shore_fade(),
            wind_speed: default_wind_speed(),
            wind_direction: default_wind_direction(),
            fetch: default_fetch(),
            choppiness: default_choppiness(),
            skirt: 0.0,
            swash: 0.0,
        swell: 0.0,
            gloss: 1.0,
            mud: 0.0,
        name: None,
        max_wade_depth: None,
        mirror: false,
        }
    }

    #[test]
    fn a_rectangle_dragged_backwards_still_describes_the_same_box() {
        let mut w = water();
        w.bounds = [10.0, 10.0, -10.0, -10.0];
        assert_eq!(w.extent(), (-10.0, -10.0, 10.0, 10.0));
        assert!(w.contains(0.0, 0.0), "an inverted rectangle must still contain its middle");
    }

    #[test]
    fn ground_above_the_surface_has_no_depth() {
        let w = water();
        assert_eq!(w.depth_over(5.0), 0.0, "a hill standing out of the water is not -3m deep");
        assert_eq!(w.depth_over(2.0), 0.0, "ground exactly at the surface is the shoreline");
        assert!((w.depth_over(-1.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn every_optical_property_has_a_default() {

        let minimal: WaterDef = serde_json::from_str(
            r#"{"height": 1.5, "bounds": [0, 0, 4, 4]}"#,
        )
        .expect("height and bounds alone must deserialise");
        assert_eq!(minimal.height, 1.5);
        assert_eq!(minimal.wind_speed, default_wind_speed());
        assert_eq!((minimal.skirt, minimal.swash), (0.0, 0.0), "a lake has no skirt and no surf");
        assert_eq!(minimal.deep, default_deep());
    }

    #[test]
    fn a_flat_bed_below_the_surface_tessellates_and_carries_depth() {
        let w = water();
        let (verts, indices) = build_surface(&w, |_, _| Some(0.0)).expect("all of it is wet");
        assert!(!indices.is_empty() && indices.len() % 3 == 0);
        for v in &verts {
            assert!((v.depth - 2.0).abs() < 1e-6, "flat bed at y=0 under y=2 is 2m deep");
            assert!((v.position[1] - 2.0).abs() < 1e-6, "every vertex sits ON the surface");
        }
    }

    #[test]
    fn a_bed_entirely_above_the_surface_produces_nothing() {

        let w = water();
        assert!(build_surface(&w, |_, _| Some(9.0)).is_none());
    }

    #[test]
    fn dry_quads_are_dropped_but_the_wet_ones_survive() {

        let w = water();
        let bowl = |x: f32, z: f32| Some(if x.abs() < 4.0 && z.abs() < 4.0 { -1.0 } else { 6.0 });
        let (verts, indices) = build_surface(&w, bowl).expect("the middle is wet");

        let full = build_surface(&w, |_, _| Some(0.0)).unwrap().1.len();
        assert!(
            indices.len() < full / 2,
            "a small pond in a big box must drop most of its quads: {} vs {full}",
            indices.len(),
        );

        for tri in indices.as_chunks::<3>().0 {
            assert!(
                tri.iter().any(|&i| verts[i as usize].depth > 0.0),
                "a triangle was kept with all three corners on dry land",
            );
        }
    }

    #[test]
    fn water_running_off_the_heightfield_keeps_going() {

        let w = water();
        let (verts, _) = build_surface(&w, |x, _| if x < 0.0 { Some(0.0) } else { None })
            .expect("half of it is over known ground");
        assert!(
            verts.iter().filter(|v| v.position[0] > 0.0).all(|v| v.depth > 0.0),
            "water past the edge of the terrain must still be wet",
        );
    }

    #[test]
    fn the_grid_reaches_its_own_far_edge() {

        let mut w = water();
        w.bounds = [0.0, 0.0, 3.3, 3.3];
        let (verts, _) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let max_x = verts.iter().fold(f32::MIN, |m, v| m.max(v.position[0]));
        let max_z = verts.iter().fold(f32::MIN, |m, v| m.max(v.position[2]));
        assert!((max_x - 3.3).abs() < 1e-5, "grid stopped at {max_x}, not 3.3");
        assert!((max_z - 3.3).abs() < 1e-5, "grid stopped at {max_z}, not 3.3");
    }

    #[test]
    fn depth_below_the_surface_is_signed() {

        let w = water();
        let (verts, _) = build_surface(&w, |x, _| Some(if x < 0.0 { 0.0 } else { 2.5 })).unwrap();
        let dry = verts.iter().find(|v| v.position[0] > 1.0).unwrap();
        assert!((dry.depth + 0.5).abs() < 1e-6, "ground 0.5 m above the surface is -0.5 m deep, got {}", dry.depth);
    }

    #[test]
    fn a_beach_the_waves_reach_keeps_its_surface() {

        let mut w = water();
        let beach = |x: f32, _: f32| Some(2.0 + 0.05 * x);
        let still = build_surface(&w, beach).unwrap().1.len();
        w.swash = 0.3;
        let (verts, indices) = build_surface(&w, beach).unwrap();
        assert!(indices.len() > still, "swash must keep more surface: {} vs {still}", indices.len());
        for tri in indices.as_chunks::<3>().0 {
            assert!(
                tri.iter().any(|&i| verts[i as usize].depth > -0.3),
                "a triangle was kept above where the waves reach",
            );
        }
    }

    #[test]
    fn a_skirt_carries_the_surface_out_to_the_horizon() {
        let mut w = water();
        w.skirt = 5000.0;
        let (verts, indices) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let reach = verts.iter().map(|v| v.position[0].abs().max(v.position[2].abs())).fold(0.0f32, f32::max);
        assert!(reach >= 5000.0, "the skirt reaches {reach} m, not 5 km past the bounds");
        assert!(verts.iter().all(|v| v.position[1] == 2.0), "a skirt lies at the surface's own height");
        assert!(indices.iter().all(|&i| (i as usize) < verts.len()));

        let edge = verts.iter().filter(|v| v.position[0].abs() == 10.0 && v.position[2].abs() <= 10.0).count();
        let side = (20.0 / WATER_GRID_STEP) as usize + 1;
        assert_eq!(edge, 2 * side, "the grid's edge was duplicated for the skirt");

        assert!(verts.iter().filter(|v| v.position[0].abs() > 10.0).all(|v| v.depth == OPEN_WATER_DEPTH));
    }

    #[test]
    fn a_sea_does_not_fill_a_basin_ringed_by_land() {

        let mut w = water();
        w.bounds = [-20.0, -20.0, 20.0, 20.0];
        w.skirt = 100.0;
        let island = |x: f32, z: f32| {
            let r = (x * x + z * z).sqrt();
            Some(if r < 3.0 { 0.0 } else if r < 12.0 { 4.0 } else { 0.0 })
        };
        let (verts, indices) = build_surface(&w, island).unwrap();
        let in_basin = indices.as_chunks::<3>().0.iter().filter(|t| {
            t.iter().all(|&i| {
                let [x, _, z] = verts[i as usize].position;
                (x * x + z * z).sqrt() < 3.0
            })
        });
        assert_eq!(in_basin.count(), 0, "the sea flooded a basin it cannot reach");

        w.skirt = 0.0;
        let (verts, indices) = build_surface(&w, island).unwrap();
        assert!(indices.as_chunks::<3>().0.iter().any(|t| t.iter().all(|&i| {
            let [x, _, z] = verts[i as usize].position;
            (x * x + z * z).sqrt() < 3.0
        })));
    }

    #[test]
    fn a_walk_finds_the_water_the_surface_draws() {

        let mut w = water();
        w.bounds = [-20.0, -20.0, 20.0, 20.0];
        w.skirt = 100.0;
        let island = |x: f32, z: f32| {
            let r = (x * x + z * z).sqrt();
            Some(if r < 3.0 { 0.0 } else if r < 12.0 { 4.0 } else { 0.0 })
        };
        let sea = WaterFootprint::new(&w, island).unwrap();
        assert_eq!(sea.surface_at(16.0, 0.0), Some(2.0), "off the shore");
        assert_eq!(sea.surface_at(20.0, 20.0), Some(2.0), "on the grid's far corner");
        assert_eq!(sea.surface_at(60.0, 0.0), Some(2.0), "out on the skirt");
        assert_eq!(sea.surface_at(200.0, 0.0), None, "past the skirt's edge");
        assert_eq!(sea.surface_at(8.0, 0.0), None, "on the island");
        assert_eq!(sea.surface_at(0.5, 0.5), None, "in the basin the sea cannot reach");
        w.skirt = 0.0;
        let lake = WaterFootprint::new(&w, island).unwrap();
        assert_eq!(lake.surface_at(0.5, 0.5), Some(2.0), "a lake is the water in its box");
        assert_eq!(lake.surface_at(60.0, 0.0), None, "and ends at its edge");
    }

    #[test]
    fn a_big_body_gets_a_coarser_grid() {
        let mut w = water();
        w.bounds = [-200.0, -200.0, 200.0, 200.0];
        let (verts, _) = build_surface(&w, |_, _| Some(0.0)).unwrap();
        let side = WATER_GRID_MAX_STEPS as usize + 1;
        assert_eq!(verts.len(), side * side, "a 400 m sea must not be tessellated at half a metre");
    }

    #[test]
    fn a_water_def_survives_a_round_trip() {
        let w = water();
        let text = serde_json::to_string(&w).unwrap();
        assert_eq!(serde_json::from_str::<WaterDef>(&text).unwrap(), w);
    }

    #[test]
    fn a_body_going_in_is_seen_once_and_one_bobbing_at_the_surface_is_not_seen_again() {
        use glam::Vec3;
        let mut w = WaterWatch::default();
        let half = Vec3::new(0.2, 0.1, 0.3);
        let fall = Vec3::new(0.5, -4.0, 0.0);
        let mut entries = Vec::new();
        for k in 0..20 {
            let centre = Vec3::new(1.0, 1.0 - 0.1 * k as f32, 2.0);
            entries.extend(w.see("box", centre, half, fall, 12.0, Some(0.0)));
        }
        assert_eq!(entries.len(), 1, "{entries:?}");
        let e = &entries[0];
        assert!(e.position == Vec3::new(1.0, 0.0, 2.0) && e.velocity == fall && e.mass == 12.0);
        assert!((std::f32::consts::PI * e.radius * e.radius - 0.24).abs() < 1e-4);
        for k in 0..200 {
            let centre = Vec3::new(1.0, 0.1 + 0.01 * (k as f32 * 0.7).sin(), 2.0);
            assert!(w.see("box", centre, half, Vec3::new(0.0, -1.0, 0.0), 12.0, Some(0.0)).is_none());
        }
        assert!(w.see("box", Vec3::new(1.0, 1.0, 2.0), half, Vec3::ZERO, 12.0, Some(0.0)).is_none());
        assert!(w.see("box", Vec3::new(1.0, 0.05, 2.0), half, fall, 12.0, Some(0.0)).is_some());
        let mut w = WaterWatch::default();
        assert!(w.see("a", Vec3::new(0.0, 1.0, 0.0), half, Vec3::ZERO, 1.0, Some(0.0)).is_none());
        assert!(w.see("a", Vec3::new(0.0, 0.0, 0.0), half, Vec3::new(0.0, -0.2, 0.0), 1.0, Some(0.0)).is_none());
        assert!(w.see("b", Vec3::new(0.0, -1.0, 0.0), half, fall, 1.0, Some(0.0)).is_none());
        assert!(w.see("c", Vec3::new(0.0, 1.0, 0.0), half, fall, 1.0, None).is_none());
        assert!(w.see("c", Vec3::new(0.0, -1.0, 0.0), half, fall, 1.0, None).is_none());
        w.keep(|id| id == "a");
        assert_eq!(w.bodies.len(), 1);
    }

    #[test]
    fn a_turned_box_is_wider_along_the_worlds_axes() {
        use glam::{Quat, Vec3};
        let h = world_half_extents(Vec3::new(1.0, 0.1, 0.2), Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!((h - Vec3::new(0.1, 1.0, 0.2)).length() < 1e-5, "{h:?}");
    }
}
