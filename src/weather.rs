//! WEATHER: rain and snow over authored areas, and what they leave behind --
//! wet ground, puddles in its hollows, snow on what faces the sky.
//!
//! WHAT AN AREA IS
//!
//! A box on the map (`bounds`, as a body of water's) where it rains or snows,
//! with the most it may leave there: how wet the ground gets, how deep its
//! puddles may stand, how deep its snow may lie. Its edges blend over `edge`
//! metres, so walking out of the rain is a gradual thing and the ground's
//! wetness does not stop on a line.
//!
//! WHAT IS SIMULATED, AND WHERE
//!
//! Only a few numbers move with time, one set an area ([`WeatherState`]): how
//! wet its open ground is, how deep the water stands in the bottom of its
//! hollows, and how deep its snow lies on open, level ground. They follow the
//! rain and snow actually falling (`intensity`, through `cycle`), the
//! authored rates, and drying and melting -- physics in millimetres an hour,
//! see the constants below.
//!
//! Everything that varies across the area is FIXED at load ([`WeatherGround`]):
//! what covers each point from the sky (the cover height, found by casting
//! straight down from above, so a roof covers the floor under it and the cave
//! the ground inside it), where water collects (each hollow's spill level, by
//! a priority flood over the ground plus a few centimetres of micro-relief,
//! which is what puddles really sit in on ground a heightfield calls flat),
//! and how well each slope holds snow. A map is then a cheap function of the
//! two: [`surface`], texel by texel, run a few times a second.
//!
//! The renderer reads the map (`space_soup::renderer::weather`): wetness
//! darkens porous ground and smooths its highlight, a puddle is a flat mirror
//! taking rain ripples, snow is white, rough and sparkling over what faces up,
//! and the ground rises by its depth. Every one of them is lit by the same
//! lights, shadows, masks and reflections as the dry surface.

use serde::{Deserialize, Serialize};

/// What falls from the sky.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrecipitationKind {
    #[default]
    Rain,
    Snow,
}

/// Where an area starts, before any time has passed: so a level can open on
/// ground already soaked or already white. Shares of the area's maxima.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct WeatherStart {
    /// How wet the open ground is, 0..1.
    #[serde(default)]
    pub wetness: f32,
    /// How full its hollows are, as a share of `max_puddle_depth`.
    #[serde(default)]
    pub puddles: f32,
    /// How deep the snow lies, as a share of `max_snow_depth`.
    #[serde(default)]
    pub snow: f32,
}

impl WeatherStart {
    fn is_dry(&self) -> bool {
        *self == Self::default()
    }
}

/// One area of weather. See the module comment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherDef {
    /// The author's name for it, for the editor's lists; nothing reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default)]
    pub kind: PrecipitationKind,

    /// Footprint as `[min_x, min_z, max_x, max_z]`, as a body of water's.
    pub bounds: [f32; 4],

    /// Metres inside `bounds` over which everything fades in: the falling
    /// rain or snow and what it leaves.
    #[serde(default = "default_edge")]
    pub edge: f32,

    /// How hard it falls: 1 is heavy rain (8 mm an hour) or a heavy snowfall
    /// (2.5 cm of snow an hour); 0 stops it.
    #[serde(default = "default_intensity")]
    pub intensity: f32,

    /// The wind, m/s, along x and z: what slants the rain and drifts the snow.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub wind: [f32; 2],

    /// The wettest the open ground gets, 0..1: 1 soaked dark, less for ground
    /// that drains or a drizzle that never soaks it.
    #[serde(default = "default_one")]
    pub max_wetness: f32,

    /// Metres: the deepest water stands in the area's hollows. 0 makes no
    /// puddles at all.
    #[serde(default = "default_puddle_depth")]
    pub max_puddle_depth: f32,

    /// Metres of micro-relief the ground has -- ruts, divots, tufts -- that a
    /// heightfield sampled every metre calls flat, and that puddles stand in.
    /// About 0.04 for a lawn, 0.08 for a rutted track, 0.01 for paving.
    #[serde(default = "default_relief")]
    pub ground_relief: f32,

    /// Metres: the deepest snow lies on open, level ground. 0 makes none.
    #[serde(default = "default_snow_depth")]
    pub max_snow_depth: f32,

    /// Multiplies how fast what falls accumulates: 1 is the real rate (a
    /// heavy rain fills puddles over the best part of an hour); a demo area
    /// takes 10-30 to show it in a few minutes.
    #[serde(default = "default_one")]
    pub accumulation_rate: f32,

    /// Multiplies how fast the ground dries and the snow melts once it stops.
    #[serde(default = "default_one")]
    pub dry_rate: f32,

    /// Air temperature, degrees C: above 0 snow melts. Defaults to 8 for rain
    /// and -2 for snow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Seconds of falling then seconds of clear, repeating, to show the
    /// ground drying and the snow melting between showers. `[0, 0]` falls
    /// all the time.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub cycle: [f32; 2],

    /// What has already fallen when the level opens.
    #[serde(default, skip_serializing_if = "WeatherStart::is_dry")]
    pub start: WeatherStart,
}

fn default_edge() -> f32 { 3.0 }
fn default_intensity() -> f32 { 1.0 }
fn default_one() -> f32 { 1.0 }
fn default_puddle_depth() -> f32 { 0.03 }
fn default_relief() -> f32 { 0.04 }
fn default_snow_depth() -> f32 { 0.15 }
fn is_zero2(v: &[f32; 2]) -> bool { v[0] == 0.0 && v[1] == 0.0 }

/// Heavy rain, mm an hour, at intensity 1.
pub const RAIN_MM_PER_HOUR: f32 = 8.0;
/// Water a surface holds before it is soaked and water runs off it, mm. A
/// film this deep darkens porous ground fully (Lagarde, "Water drop 2").
pub const WET_SATURATION_MM: f32 = 0.4;
/// Evaporation from wet ground under an overcast sky, mm an hour.
pub const EVAPORATION_MM_PER_HOUR: f32 = 0.25;
/// What soaks away into the ground under a puddle, mm an hour.
pub const INFILTRATION_MM_PER_HOUR: f32 = 0.5;
/// How many times the rain falling on a puddle reaches it: runoff from the
/// ground round a hollow drains into it.
pub const CATCHMENT: f32 = 6.0;
/// Snow on level ground at intensity 1, m an hour.
pub const SNOW_M_PER_HOUR: f32 = 0.025;
/// Snow depth melted per degree above zero, m an hour (a degree-day factor of
/// about 4 mm of water a degree a day, in fresh snow of density 0.1).
pub const MELT_M_PER_HOUR_PER_DEGREE: f32 = 0.0015;
/// Slope, degrees, past which snow starts to slide off, and the slope it no
/// longer lies on at all.
pub const SNOW_HOLD_DEG: (f32, f32) = (32.0, 50.0);
/// The finest a map's texel is, metres, and the most texels along a side.
pub const MAP_CELL: f32 = 0.25;
pub const MAP_MAX_CELLS: usize = 256;
/// The most areas a scene's weather draws. See `space_soup::renderer::weather`.
pub const MAX_AREAS: usize = 4;
/// Where nothing at all lies under a point, its cover height: open sky.
pub const NO_COVER: f32 = -1.0e4;

impl WeatherDef {
    /// Footprint `(min_x, min_z, max_x, max_z)`, normalised so min <= max.
    pub fn extent(&self) -> (f32, f32, f32, f32) {
        let [a, b, c, d] = self.bounds;
        (a.min(c), b.min(d), a.max(c), b.max(d))
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (x0, z0, x1, z1) = self.extent();
        x >= x0 && x <= x1 && z >= z0 && z <= z1
    }

    /// 1 well inside the area, 0 at and past its edge, smooth between.
    pub fn edge_fade(&self, x: f32, z: f32) -> f32 {
        let (x0, z0, x1, z1) = self.extent();
        let e = self.edge.max(1e-3);
        let s = |d: f32| {
            let t = (d / e).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        s((x - x0).min(x1 - x)) * s((z - z0).min(z1 - z))
    }

    pub fn temperature(&self) -> f32 {
        self.temperature.unwrap_or(match self.kind {
            PrecipitationKind::Rain => 8.0,
            PrecipitationKind::Snow => -2.0,
        })
    }

    /// How hard it is falling `t` seconds after the level opened, through
    /// `cycle`: each shower comes in and goes over 15 seconds.
    pub fn intensity_at(&self, t: f64) -> f32 {
        let i = self.intensity.max(0.0);
        let [on, off] = self.cycle;
        if on <= 0.0 || off <= 0.0 {
            return i;
        }
        let period = (on + off) as f64;
        let phase = t.rem_euclid(period) as f32;
        let ramp = 15.0f32.min(0.5 * on);
        let up = (phase / ramp).clamp(0.0, 1.0);
        let down = ((on - phase) / ramp).clamp(0.0, 1.0);
        i * up.min(down)
    }
}

/// What has accumulated over an area at one moment: the few numbers that move
/// with time. See the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeatherState {
    /// How wet its open ground is, 0..1, before `max_wetness`.
    pub wet: f32,
    /// Metres of water standing at the bottom of its hollows, up to
    /// `max_puddle_depth`.
    pub fill: f32,
    /// Metres of snow on open, level ground, up to `max_snow_depth`.
    pub snow: f32,
    /// How hard it is falling now, 0..`intensity`.
    pub falling: f32,
}

impl WeatherState {
    /// Where `def` starts.
    pub fn start(def: &WeatherDef) -> Self {
        let s = def.start;
        Self {
            wet: s.wetness.clamp(0.0, 1.0),
            fill: s.puddles.clamp(0.0, 1.0) * def.max_puddle_depth.max(0.0),
            snow: s.snow.clamp(0.0, 1.0) * def.max_snow_depth.max(0.0),
            falling: def.intensity_at(0.0),
        }
    }

    /// `dt` seconds on from `t`. Rain wets the ground until it is soaked, and
    /// then what keeps falling runs off into the hollows; snow piles up while
    /// it is cold enough; with nothing falling the ground dries, the puddles
    /// sink away and snow above freezing melts.
    pub fn step(&mut self, def: &WeatherDef, t: f64, dt: f32) {
        let falling = def.intensity_at(t);
        self.falling = falling;
        let a = def.accumulation_rate.max(0.0);
        let d = def.dry_rate.max(0.0);
        let temp = def.temperature();
        let rain_mm_s = match def.kind {
            PrecipitationKind::Rain => RAIN_MM_PER_HOUR * falling / 3600.0,
            PrecipitationKind::Snow => 0.0,
        };
        let snow_m_s = match def.kind {
            PrecipitationKind::Snow if temp <= 1.0 => SNOW_M_PER_HOUR * falling / 3600.0,
            _ => 0.0,
        };
        let melt_m_s = d * MELT_M_PER_HOUR_PER_DEGREE * temp.max(0.0) / 3600.0;
        let evap_mm_s = d * EVAPORATION_MM_PER_HOUR / 3600.0;

        // Wetness: soaked by rain, dried by the air; melting snow keeps what
        // is under it soaked.
        if rain_mm_s > 0.0 {
            self.wet += a * rain_mm_s * dt / WET_SATURATION_MM;
        } else {
            self.wet -= evap_mm_s * dt / WET_SATURATION_MM;
        }
        // Puddles: the runoff once the ground is soaked, less what soaks in and
        // evaporates.
        let runoff = if self.wet >= 1.0 { a * rain_mm_s * CATCHMENT } else { 0.0 };
        let loss = d * (EVAPORATION_MM_PER_HOUR + INFILTRATION_MM_PER_HOUR) / 3600.0;
        self.fill += (runoff - loss) * dt / 1000.0;
        // Snow.
        let melting = self.snow > 0.0 && melt_m_s > 0.0;
        self.snow += (a * snow_m_s - melt_m_s) * dt;
        if melting {
            self.wet = 1.0;
        }
        self.wet = self.wet.clamp(0.0, 1.0);
        self.fill = self.fill.clamp(0.0, def.max_puddle_depth.max(0.0));
        self.snow = self.snow.clamp(0.0, def.max_snow_depth.max(0.0));
    }

    /// Where `def` stands `t` seconds after the level opened: stepped a
    /// second at a time from its start, so the same `t` is always the same
    /// state -- what an offline render at a weather time needs.
    pub fn at(def: &WeatherDef, t: f64) -> Self {
        let mut s = Self::start(def);
        let dt = if t > 6.0 * 3600.0 { 10.0 } else { 1.0 };
        let mut now = 0.0f64;
        while now < t {
            let step = ((t - now) as f32).min(dt);
            s.step(def, now, step);
            now += step as f64;
        }
        s.falling = def.intensity_at(t);
        s
    }
}

/// An area's ground, as weather sees it: everything about each point that
/// does not change with time. Built once, at load. See the module comment.
#[derive(Debug, Clone)]
pub struct WeatherGround {
    /// The world x/z of texel (0, 0)'s corner, and the texel's size.
    pub min: [f32; 2],
    pub cell: f32,
    pub nx: usize,
    pub nz: usize,
    /// World y of the first thing straight down from the sky at each texel's
    /// centre: the ground, a roof, a cave's ridge.
    pub cover: Vec<f32>,
    /// The cover with its micro-relief: what water stands in.
    relief: Vec<f32>,
    /// The level water rises to in each texel's hollow before it spills.
    spill: Vec<f32>,
    /// The bottom of each texel's hollow; +inf where it is in none.
    floor: Vec<f32>,
    /// How well the slope here holds snow, 0..1, with its drift.
    snow_hold: Vec<f32>,
    /// `WeatherDef::edge_fade` at each texel.
    fade: Vec<f32>,
    /// How much of the area's wetness each texel does without, 0 to
    /// [`THIRST_MOST`]: see [`thirst`].
    thirst: Vec<f32>,
}

/// The most of an area's wetness a ridge or a slope goes without.
pub const THIRST_MOST: f32 = 0.5;
/// Slope, degrees, where ground starts to shed its water, and where it sheds
/// the most.
pub const THIRST_SLOPE_DEG: (f32, f32) = (8.0, 25.0);

/// WHERE AN AREA'S WETNESS SHOWS. Rain wets ground evenly, but water does not
/// stay evenly: it drains off slopes and off the crowns of the ground's own
/// relief into the hollows beside them, so as an area wets its hollows darken
/// first, and as it dries its ridges pale first and its hollows last, in
/// patches a metre or so across. One number a texel, fixed at load, taken from
/// the area's wetness before it is shown ([`surface`]): nothing in a hollow
/// (where water would stand) or on flat ground, up to [`THIRST_MOST`] on a
/// slope of `THIRST_SLOPE_DEG.1` or on a crown standing `relief` over the
/// ground round it. A soaked area is soaked everywhere.
pub fn thirst(slope_deg: f32, proud: f32, relief: f32, in_hollow: bool) -> f32 {
    if in_hollow {
        return 0.0;
    }
    let (s0, s1) = THIRST_SLOPE_DEG;
    let t = ((slope_deg - s0) / (s1 - s0)).clamp(0.0, 1.0);
    let steep = t * t * (3.0 - 2.0 * t);
    let crown = if relief > 0.0 { (proud / relief).clamp(0.0, 1.0) } else { 0.0 };
    THIRST_MOST * steep.max(crown)
}

fn hash2(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (z as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smooth value noise, -1..1, of wavelength `w` metres.
fn value_noise(x: f32, z: f32, w: f32, seed: u32) -> f32 {
    let (u, v) = (x / w, z / w);
    let (i, j) = (u.floor(), v.floor());
    let (fu, fv) = (u - i, v - j);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (su, sv) = (s(fu), s(fv));
    let (i, j) = (i as i32, j as i32);
    let a = hash2(i, j, seed);
    let b = hash2(i + 1, j, seed);
    let c = hash2(i, j + 1, seed);
    let d = hash2(i + 1, j + 1, seed);
    let top = a + (b - a) * su;
    let bottom = c + (d - c) * su;
    (top + (bottom - top) * sv) * 2.0 - 1.0
}

#[derive(PartialEq)]
struct Low(f32, usize);
impl PartialOrd for Low {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Eq for Low {}
impl Ord for Low {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reversed, so the heap pops the LOWEST first.
        other.0.partial_cmp(&self.0).unwrap_or(std::cmp::Ordering::Equal).then(other.1.cmp(&self.1))
    }
}

/// THE LEVEL EACH HOLLOW FILLS TO before it spills, by a priority flood from
/// the grid's edge (Barnes, Lehman & Mulla 2014): water leaves the area over
/// its edge, so every hollow inside it holds water up to its lowest rim.
pub fn spill_levels(h: &[f32], nx: usize, nz: usize) -> Vec<f32> {
    let mut spill = h.to_vec();
    let mut seen = vec![false; h.len()];
    let mut open = std::collections::BinaryHeap::new();
    for z in 0..nz {
        for x in 0..nx {
            if x == 0 || z == 0 || x + 1 == nx || z + 1 == nz {
                let i = z * nx + x;
                seen[i] = true;
                open.push(Low(h[i], i));
            }
        }
    }
    while let Some(Low(level, i)) = open.pop() {
        let (x, z) = (i % nx, i / nx);
        let mut visit = |j: usize| {
            if !seen[j] {
                seen[j] = true;
                spill[j] = h[j].max(level);
                open.push(Low(spill[j], j));
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < nx {
            visit(i + 1);
        }
        if z > 0 {
            visit(i - nx);
        }
        if z + 1 < nz {
            visit(i + nx);
        }
    }
    spill
}

/// The bottom of the hollow each texel lies in (+inf for none): the lowest
/// ground of each connected run of texels under water at their spill level.
pub fn hollow_floors(h: &[f32], spill: &[f32], nx: usize, nz: usize) -> Vec<f32> {
    let wet = |i: usize| spill[i] - h[i] > 1e-5;
    let mut floor = vec![f32::INFINITY; h.len()];
    let mut label = vec![usize::MAX; h.len()];
    let mut stack = Vec::new();
    for start in 0..h.len() {
        if !wet(start) || label[start] != usize::MAX {
            continue;
        }
        let mut members = Vec::new();
        let mut lowest = f32::INFINITY;
        label[start] = start;
        stack.push(start);
        while let Some(i) = stack.pop() {
            members.push(i);
            lowest = lowest.min(h[i]);
            let (x, z) = (i % nx, i / nx);
            let mut next = |j: usize| {
                if wet(j) && label[j] == usize::MAX {
                    label[j] = start;
                    stack.push(j);
                }
            };
            if x > 0 {
                next(i - 1);
            }
            if x + 1 < nx {
                next(i + 1);
            }
            if z > 0 {
                next(i - nx);
            }
            if z + 1 < nz {
                next(i + nx);
            }
        }
        for i in members {
            floor[i] = lowest;
        }
    }
    floor
}

impl WeatherGround {
    /// `def`'s ground, from `cover`: the height of the first thing a ray
    /// straight down from the sky meets at a world x/z, or `None` where it
    /// meets nothing (past the terrain's edge, through a hole).
    pub fn build(def: &WeatherDef, cover: impl Fn(f32, f32) -> Option<f32>) -> Self {
        let (x0, z0, x1, z1) = def.extent();
        let span = (x1 - x0).max(z1 - z0).max(MAP_CELL);
        let cell = MAP_CELL.max(span / MAP_MAX_CELLS as f32);
        let nx = (((x1 - x0) / cell).ceil() as usize).clamp(2, MAP_MAX_CELLS);
        let nz = (((z1 - z0) / cell).ceil() as usize).clamp(2, MAP_MAX_CELLS);
        let cx = (x1 - x0) / nx as f32;
        let cz = (z1 - z0) / nz as f32;
        let centre = |x: usize, z: usize| (x0 + (x as f32 + 0.5) * cx, z0 + (z as f32 + 0.5) * cz);
        let mut found = Vec::with_capacity(nx * nz);
        for z in 0..nz {
            for x in 0..nx {
                let (wx, wz) = centre(x, z);
                found.push(cover(wx, wz));
            }
        }
        // Where nothing is under a point it drains: below everything found.
        let lowest = found.iter().flatten().copied().fold(f32::INFINITY, f32::min);
        let drain = if lowest.is_finite() { lowest - 1.0 } else { 0.0 };
        let cover: Vec<f32> = found.iter().map(|c| c.unwrap_or(drain)).collect();
        let seed = (x0.to_bits() ^ z0.to_bits().rotate_left(11)) | 1;
        let mut relief = Vec::with_capacity(cover.len());
        let mut fade = Vec::with_capacity(cover.len());
        for z in 0..nz {
            for x in 0..nx {
                let (wx, wz) = centre(x, z);
                let n = 0.55 * value_noise(wx, wz, 2.3, seed)
                    + 0.3 * value_noise(wx, wz, 1.1, seed ^ 0x55)
                    + 0.15 * value_noise(wx, wz, 0.55, seed ^ 0xaa);
                relief.push(cover[z * nx + x] + def.ground_relief.max(0.0) * n);
                fade.push(def.edge_fade(wx, wz));
            }
        }
        let spill = spill_levels(&relief, nx, nz);
        let floor = hollow_floors(&relief, &spill, nx, nz);
        // Snow by slope, from the cover's own gradient; drifted a little.
        let at = |x: usize, z: usize| cover[z.min(nz - 1) * nx + x.min(nx - 1)];
        let mut snow_hold = Vec::with_capacity(cover.len());
        let mut dry_first = Vec::with_capacity(cover.len());
        let relief_at = |x: isize, z: isize| relief[z.clamp(0, nz as isize - 1) as usize * nx + x.clamp(0, nx as isize - 1) as usize];
        for z in 0..nz {
            for x in 0..nx {
                let gx = (at(x + 1, z) - at(x.saturating_sub(1), z)) / (cx * (1 + (x > 0) as usize) as f32);
                let gz = (at(x, z + 1) - at(x, z.saturating_sub(1))) / (cz * (1 + (z > 0) as usize) as f32);
                let slope = (gx * gx + gz * gz).sqrt().atan().to_degrees();
                // How far this texel stands over the ground a step round it
                // (the mean of a 5 x 5 block), and whether water stands on it.
                let i = z * nx + x;
                let mut round = 0.0;
                for dz in -2..=2isize {
                    for dx in -2..=2isize {
                        round += relief_at(x as isize + dx, z as isize + dz);
                    }
                }
                let in_hollow = floor[i].is_finite() && spill[i] - relief[i] > 1e-5;
                dry_first.push(thirst(slope, relief[i] - round / 25.0, 0.5 * def.ground_relief.max(0.0), in_hollow));
                let (s0, s1) = SNOW_HOLD_DEG;
                let t = ((slope - s0) / (s1 - s0)).clamp(0.0, 1.0);
                let hold = 1.0 - t * t * (3.0 - 2.0 * t);
                let (wx, wz) = centre(x, z);
                let drift = 1.0 + 0.18 * value_noise(wx, wz, 5.3, seed ^ 0x3c) + 0.07 * value_noise(wx, wz, 1.9, seed ^ 0xc3);
                snow_hold.push(hold * drift);
            }
        }
        Self { min: [x0, z0], cell: cx.max(cz), nx, nz, cover, relief, spill, floor, snow_hold, fade, thirst: dry_first }
    }

    /// Whether water can stand anywhere in the world rectangle `lo`..`hi`
    /// (x, z), a texel beyond it included for the map's filtering: some
    /// hollow there holds more than the least a puddle is drawn at. What
    /// decides whether ground there needs the standing water's shader at all
    /// (`space_soup::renderer::weather::WeatherKinds`).
    pub fn can_pool(&self, def: &WeatherDef, lo: [f32; 2], hi: [f32; 2]) -> bool {
        if def.max_puddle_depth <= 0.0 {
            return false;
        }
        let (cx, cz) = (self.extent_x() / self.nx as f32, self.extent_z() / self.nz as f32);
        let x0 = (((lo[0] - self.min[0]) / cx).floor() as isize - 1).max(0) as usize;
        let z0 = (((lo[1] - self.min[1]) / cz).floor() as isize - 1).max(0) as usize;
        let x1 = (((hi[0] - self.min[0]) / cx).floor() as isize + 1).min(self.nx as isize - 1);
        let z1 = (((hi[1] - self.min[1]) / cz).floor() as isize + 1).min(self.nz as isize - 1);
        if x1 < x0 as isize || z1 < z0 as isize {
            return false;
        }
        (z0..=z1 as usize).any(|z| {
            (x0..=x1 as usize).any(|x| {
                let i = z * self.nx + x;
                self.floor[i].is_finite() && self.fade[i] > 0.0 && (self.spill[i] - self.relief[i]).min(def.max_puddle_depth) * self.fade[i] > 3e-4
            })
        })
    }

    /// How much of its area's wetness the texel holding world x/z goes
    /// without ([`thirst`]); `None` outside.
    pub fn thirst_at(&self, x: f32, z: f32) -> Option<f32> {
        self.texel(x, z).map(|i| self.thirst[i])
    }

    /// The texel holding world x/z, if inside.
    fn texel(&self, x: f32, z: f32) -> Option<usize> {
        let cx = self.extent_x() / self.nx as f32;
        let cz = self.extent_z() / self.nz as f32;
        let i = ((x - self.min[0]) / cx).floor();
        let j = ((z - self.min[1]) / cz).floor();
        (i >= 0.0 && j >= 0.0 && (i as usize) < self.nx && (j as usize) < self.nz).then(|| j as usize * self.nx + i as usize)
    }

    fn extent_x(&self) -> f32 {
        self.cell_x() * self.nx as f32
    }
    fn extent_z(&self) -> f32 {
        self.cell_z() * self.nz as f32
    }
    fn cell_x(&self) -> f32 {
        self.cell
    }
    fn cell_z(&self) -> f32 {
        self.cell
    }

    /// The cover height at world x/z, if inside.
    pub fn cover_at(&self, x: f32, z: f32) -> Option<f32> {
        self.texel(x, z).map(|i| self.cover[i])
    }

    /// How far down the sky reaches a point: 1 where nothing above it stands
    /// higher than `slack` metres over it, 0 under a roof, as the shaders
    /// decide it.
    pub fn exposure(&self, p: [f32; 3]) -> f32 {
        match self.cover_at(p[0], p[2]) {
            Some(c) => ((p[1] - c + 0.5) / 0.4).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// Metres of water standing at each texel with `fill` at the hollows'
    /// floors, as [`surface`] has it.
    pub fn puddle_depth(&self, i: usize, fill: f32, max_depth: f32) -> f32 {
        let floor = self.floor[i];
        if !floor.is_finite() {
            return 0.0;
        }
        let level = self.spill[i].min(floor + fill);
        (level - self.relief[i]).clamp(0.0, max_depth)
    }
}

/// AN AREA'S MAP, texel by texel, rows along z: wetness (0..1), standing
/// water (m), snow (m) and the cover height (world y) -- what the renderer
/// uploads (`space_soup::renderer::weather::WeatherMaps`). Clamped to the
/// authored maxima and faded at the area's edge.
pub fn surface(def: &WeatherDef, ground: &WeatherGround, state: &WeatherState) -> Vec<[f32; 4]> {
    let max_wet = def.max_wetness.clamp(0.0, 1.0);
    let max_puddle = def.max_puddle_depth.max(0.0);
    let max_snow = def.max_snow_depth.max(0.0);
    (0..ground.cover.len())
        .map(|i| {
            let f = ground.fade[i];
            let puddle = ground.puddle_depth(i, state.fill, max_puddle) * f;
            // Ridges and slopes show less of it, and none until the area is
            // wetter than they are thirsty (`thirst`); soaked is soaked.
            let thirst = ground.thirst[i];
            let open = ((state.wet.min(max_wet) - thirst) / (1.0 - thirst)).clamp(0.0, 1.0);
            // A puddle's rim is soaked however dry the open ground has got.
            let wet = (open * f).max((puddle / 0.002).min(1.0) * f);
            let snow = (state.snow * ground.snow_hold[i]).clamp(0.0, max_snow) * f;
            [wet, puddle, snow, ground.cover[i]]
        })
        .collect()
}

/// Every area of a scene's weather, and where each stands. What the client
/// keeps and steps each frame.
#[derive(Debug, Clone, Default)]
pub struct WeatherField {
    pub areas: Vec<(WeatherDef, WeatherGround, WeatherState)>,
}

impl WeatherField {
    /// The scene's areas (the first [`MAX_AREAS`]), their ground from
    /// `cover`, at their start.
    pub fn new(defs: &[WeatherDef], cover: impl Fn(f32, f32) -> Option<f32>) -> Self {
        Self {
            areas: defs
                .iter()
                .take(MAX_AREAS)
                .map(|d| (d.clone(), WeatherGround::build(d, &cover), WeatherState::start(d)))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    /// Every area as it stands `t` seconds after the level opened.
    pub fn set_time(&mut self, t: f64) {
        for (def, _, state) in &mut self.areas {
            *state = WeatherState::at(def, t);
        }
    }

    /// Every area `dt` seconds on from `t`.
    pub fn step(&mut self, t: f64, dt: f32) {
        for (def, _, state) in &mut self.areas {
            state.step(def, t, dt);
        }
    }

    /// What is falling on a point now: (rain, snow), each its area's
    /// intensity where the point is open to the sky, faded at the edges.
    pub fn falling_on(&self, p: [f32; 3]) -> (f32, f32) {
        let (mut rain, mut snow) = (0.0f32, 0.0f32);
        for (def, ground, state) in &self.areas {
            if !def.contains(p[0], p[2]) {
                continue;
            }
            let k = state.falling * def.edge_fade(p[0], p[2]) * ground.exposure(p);
            match def.kind {
                PrecipitationKind::Rain => rain = rain.max(k),
                PrecipitationKind::Snow => snow = snow.max(k),
            }
        }
        (rain, snow)
    }
}

/// A body's wetness (hair, clothes, hands), `dt` seconds on: soaked in about a
/// minute of heavy rain, a little by snow melting on it, dried over ten
/// minutes out of it.
pub fn body_wetness_step(wet: f32, rain: f32, snow: f32, dt: f32) -> f32 {
    let gain = (rain + 0.15 * snow) / 60.0;
    let next = if gain > 0.0 { wet + gain * dt } else { wet - dt / 600.0 };
    next.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rain() -> WeatherDef {
        serde_json::from_str(r#"{"bounds": [0, 0, 16, 16]}"#).unwrap()
    }

    fn snow() -> WeatherDef {
        serde_json::from_str(r#"{"kind": "snow", "bounds": [0, 0, 16, 16]}"#).unwrap()
    }

    fn flat(_: f32, _: f32) -> Option<f32> {
        Some(0.0)
    }

    #[test]
    fn an_area_needs_only_its_bounds_and_writes_back_only_what_differs() {
        let d = rain();
        assert_eq!(d.kind, PrecipitationKind::Rain);
        assert_eq!((d.intensity, d.max_wetness, d.accumulation_rate), (1.0, 1.0, 1.0));
        assert!(d.max_puddle_depth > 0.0 && d.max_snow_depth > 0.0);
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("start") && !json.contains("cycle") && !json.contains("wind"), "{json}");
        let s = snow();
        assert_eq!(s.temperature(), -2.0);
        assert_eq!(rain().temperature(), 8.0);
    }

    #[test]
    fn rain_soaks_the_ground_then_fills_the_hollows_up_to_the_authored_depth() {
        let mut d = rain();
        d.max_puddle_depth = 0.02;
        let s = WeatherState::at(&d, 60.0);
        assert!(s.wet > 0.0 && s.wet < 1.0, "a minute of rain dampens: {s:?}");
        assert_eq!(s.fill, 0.0, "no runoff before the ground is soaked");
        let s = WeatherState::at(&d, 3.0 * 3600.0);
        assert_eq!(s.wet, 1.0);
        assert_eq!(s.fill, 0.02, "clamped to the authored maximum: {s:?}");
        // Faster with the demo's rate, and never past the maximum.
        d.accumulation_rate = 20.0;
        let fast = WeatherState::at(&d, 300.0);
        assert!(fast.fill > 0.0 && fast.fill <= 0.02, "{fast:?}");
    }

    #[test]
    fn with_nothing_falling_the_ground_dries_and_the_puddles_sink_away() {
        let mut d = rain();
        d.intensity = 0.0;
        d.start = WeatherStart { wetness: 1.0, puddles: 1.0, snow: 0.0 };
        let wet = WeatherState::at(&d, 0.0);
        let later = WeatherState::at(&d, 1800.0);
        assert!(later.wet < wet.wet && later.fill < wet.fill, "{wet:?} -> {later:?}");
        d.dry_rate = 50.0;
        let dry = WeatherState::at(&d, 3600.0);
        assert_eq!((dry.wet, dry.fill), (0.0, 0.0));
    }

    #[test]
    fn snow_piles_up_while_it_is_cold_and_melts_above_freezing() {
        let mut d = snow();
        d.max_snow_depth = 0.1;
        let an_hour = WeatherState::at(&d, 3600.0);
        assert!((an_hour.snow - SNOW_M_PER_HOUR).abs() < 1e-3, "{an_hour:?}");
        assert_eq!(WeatherState::at(&d, 8.0 * 3600.0).snow, 0.1, "clamped to the authored maximum");
        // It stops, and the air warms.
        d.intensity = 0.0;
        d.temperature = Some(5.0);
        d.start.snow = 1.0;
        let melting = WeatherState::at(&d, 3600.0);
        assert!(melting.snow < 0.1 && melting.snow > 0.0, "{melting:?}");
        assert_eq!(melting.wet, 1.0, "melting snow leaves the ground under it soaked");
        // And at -2 it lies.
        d.temperature = Some(-2.0);
        assert_eq!(WeatherState::at(&d, 3600.0).snow, 0.1);
    }

    #[test]
    fn a_cycle_falls_then_stops() {
        let mut d = rain();
        d.cycle = [60.0, 120.0];
        assert_eq!(d.intensity_at(0.0), 0.0, "a shower comes in");
        assert_eq!(d.intensity_at(30.0), 1.0);
        assert_eq!(d.intensity_at(100.0), 0.0);
        assert_eq!(d.intensity_at(210.0), 1.0, "and comes again");
    }

    #[test]
    fn water_stands_only_in_hollows_and_to_their_rim() {
        // A bowl 10 cm deep in the middle of flat ground.
        let bowl = |x: f32, z: f32| {
            let r = ((x - 8.0).powi(2) + (z - 8.0).powi(2)).sqrt();
            Some(if r < 2.0 { -0.1 * (1.0 - r / 2.0) } else { 0.0 })
        };
        let mut d = rain();
        d.max_puddle_depth = 0.5;
        let g = WeatherGround::build(&d, bowl);
        let full = WeatherState { wet: 1.0, fill: 0.5, snow: 0.0, falling: 1.0 };
        let map = surface(&d, &g, &full);
        let at = |x: f32, z: f32| map[g.texel(x, z).unwrap()];
        let middle = at(8.0, 8.0)[1];
        assert!(middle > 0.07 && middle < 0.13, "the bowl fills to its rim: {middle}");
        assert!(at(8.0, 3.0)[1] < 0.06, "flat ground holds only its micro-relief's puddles");
        // A little rain wets the bowl's bottom first.
        let little = WeatherState { fill: 0.01, ..full };
        let map = surface(&d, &g, &little);
        assert!(map[g.texel(8.0, 8.0).unwrap()][1] > 0.0);
        assert_eq!(map[g.texel(9.6, 8.0).unwrap()][1], 0.0, "not yet up its sides");
    }

    #[test]
    fn a_slope_sheds_its_water_and_its_snow() {
        // A 1-in-2 slope (27 degrees) and a cliff (63 degrees).
        let gentle = |x: f32, _: f32| Some(0.5 * x);
        let cliff = |x: f32, _: f32| Some(2.0 * x);
        let d = rain();
        let full = WeatherState { wet: 1.0, fill: 0.03, snow: 0.15, falling: 1.0 };
        let on = |cover: &dyn Fn(f32, f32) -> Option<f32>| {
            let g = WeatherGround::build(&d, cover);
            let map = surface(&d, &g, &full);
            map[g.texel(8.0, 8.0).unwrap()]
        };
        let g = on(&gentle);
        let c = on(&cliff);
        let flat_ground = on(&flat);
        assert_eq!(g[1], 0.0, "no puddle stands on a slope");
        assert!(g[2] > 0.1, "snow lies on a gentle slope: {g:?}");
        assert!(c[2] < 0.005, "and slides off a cliff: {c:?}");
        assert!(flat_ground[2] > 0.1);
        assert!(g[2] <= 0.15 * 1.3 && flat_ground[2] <= 0.15, "never past the authored maximum");
    }

    #[test]
    fn nothing_accumulates_under_cover_and_the_cover_is_what_the_sky_sees() {
        // A roof 3 m up over x 6..10.
        let roofed = |x: f32, _: f32| Some(if (6.0..10.0).contains(&x) { 3.0 } else { 0.0 });
        let d = rain();
        let g = WeatherGround::build(&d, roofed);
        assert_eq!(g.cover_at(8.0, 8.0), Some(3.0));
        assert_eq!(g.exposure([8.0, 0.0, 8.0]), 0.0, "the floor under the roof is sheltered");
        assert_eq!(g.exposure([8.0, 3.0, 8.0]), 1.0, "the roof itself is not");
        assert_eq!(g.exposure([2.0, 0.0, 8.0]), 1.0);
        let mut field = WeatherField { areas: vec![(d.clone(), g, WeatherState::start(&d))] };
        field.set_time(10.0);
        assert_eq!(field.falling_on([8.0, 1.6, 8.0]).0, 0.0, "no rain on a head under the roof");
        assert!(field.falling_on([3.0, 1.6, 8.0]).0 > 0.9);
    }

    /// A crown of the ground's relief and a slope show less of the area's
    /// wetness than a hollow or a flat, wet later and dry first; soaked ground
    /// is soaked everywhere, and dry is dry.
    #[test]
    fn ridges_and_slopes_wet_last_and_dry_first() {
        assert_eq!(thirst(0.0, -0.01, 0.02, false), 0.0);
        assert_eq!(thirst(40.0, 0.05, 0.02, true), 0.0, "water stands in a hollow");
        assert!((thirst(25.0, 0.0, 0.02, false) - THIRST_MOST).abs() < 1e-6);
        assert!((thirst(0.0, 0.02, 0.02, false) - THIRST_MOST).abs() < 1e-6);
        assert!(thirst(15.0, 0.0, 0.02, false) > 0.0 && thirst(15.0, 0.0, 0.02, false) < THIRST_MOST);
        // An area on a hillside with a level foot: the slope is drier than
        // the foot while the area is half wet, and no drier once it is soaked.
        let def = WeatherDef { ground_relief: 0.0, edge: 0.01, ..rain() };
        let hill = |x: f32, _z: f32| Some(if x > 10.0 { (x - 10.0) * 0.5 } else { 0.0 });
        let ground = WeatherGround::build(&def, hill);
        let (foot, slope) = (ground.texel(5.0, 10.0).unwrap(), ground.texel(15.0, 10.0).unwrap());
        assert_eq!(ground.thirst_at(5.0, 10.0), Some(0.0));
        assert!((ground.thirst_at(15.0, 10.0).unwrap() - THIRST_MOST).abs() < 1e-3, "{:?}", ground.thirst_at(15.0, 10.0));
        let at = |wet: f32| {
            let m = surface(&def, &ground, &WeatherState { wet, ..Default::default() });
            (m[foot][0], m[slope][0])
        };
        let (f, s) = at(0.75);
        assert!((f - 0.75).abs() < 1e-3 && (s - 0.5).abs() < 1e-3, "foot {f}, slope {s}");
        assert_eq!(at(0.4).1, 0.0, "a slope shows nothing until the area is wetter than it is thirsty");
        let (f, s) = at(1.0);
        assert!((f - 1.0).abs() < 1e-3 && (s - 1.0).abs() < 1e-3, "soaked: foot {f}, slope {s}");
        assert_eq!(at(0.0), (0.0, 0.0));
    }

    /// Whether a piece of an area needs the standing water's shader: only
    /// where a hollow there can hold a puddle.
    #[test]
    fn only_ground_with_a_hollow_can_pool() {
        let def = WeatherDef { ground_relief: 0.0, edge: 0.01, ..rain() };
        // Level but for a bowl a metre across at (6, 6), 5 cm deep.
        let bowl = |x: f32, z: f32| Some(if (x - 6.0).hypot(z - 6.0) < 0.6 { -0.05 } else { 0.0 });
        let ground = WeatherGround::build(&def, bowl);
        assert!(ground.can_pool(&def, [4.0, 4.0], [8.0, 8.0]));
        assert!(!ground.can_pool(&def, [12.0, 12.0], [18.0, 18.0]));
        // A texel beyond the rectangle counts: the map is filtered.
        assert!(ground.can_pool(&def, [6.7, 5.5], [9.0, 6.5]));
        assert!(!ground.can_pool(&WeatherDef { max_puddle_depth: 0.0, ..def.clone() }, [4.0, 4.0], [8.0, 8.0]));
    }

    #[test]
    fn everything_fades_out_at_the_areas_edge() {
        let d = rain();
        let g = WeatherGround::build(&d, flat);
        let full = WeatherState { wet: 1.0, fill: 0.03, snow: 0.1, falling: 1.0 };
        let map = surface(&d, &g, &full);
        let edge = map[g.texel(0.1, 8.0).unwrap()];
        let inside = map[g.texel(8.0, 8.0).unwrap()];
        assert!(edge[0] < 0.01 && edge[1] < 1e-3 && edge[2] < 1e-3, "{edge:?}");
        assert!(inside[0] > 0.99);
        assert!((d.edge_fade(1.5, 8.0) - 0.5).abs() < 1e-3, "half way across the edge band");
        assert_eq!(d.edge_fade(-1.0, 8.0), 0.0);
    }

    #[test]
    fn the_wettest_and_the_deepest_are_the_authored_ones() {
        let mut d = rain();
        d.max_wetness = 0.6;
        d.max_puddle_depth = 0.01;
        let g = WeatherGround::build(&d, flat);
        let s = WeatherState::at(&WeatherDef { accumulation_rate: 100.0, ..d.clone() }, 3600.0);
        let map = surface(&d, &g, &s);
        for t in &map {
            assert!(t[1] <= 0.01 + 1e-6);
            assert!(t[0] <= 0.6 + 1e-6 || t[1] > 0.0, "only a puddle's rim is wetter than the maximum: {t:?}");
        }
    }

    #[test]
    fn a_body_soaks_in_the_rain_and_dries_out_of_it() {
        let mut w = 0.0;
        for _ in 0..60 {
            w = body_wetness_step(w, 1.0, 0.0, 1.0);
        }
        assert!(w > 0.99);
        for _ in 0..300 {
            w = body_wetness_step(w, 0.0, 0.0, 1.0);
        }
        assert!(w > 0.4 && w < 0.6, "half dry after five minutes: {w}");
    }

    /// MEASUREMENT: what building an area's ground and working out its map
    /// cost on this machine, for an area the size of test_room's snow demo.
    #[test]
    #[ignore]
    fn time_an_areas_map() {
        let d: WeatherDef = serde_json::from_str(r#"{"bounds": [-10, -45, 18, -19]}"#).unwrap();
        let t = std::time::Instant::now();
        let g = WeatherGround::build(&d, |x, z| Some(0.3 * (x * 0.2).sin() + 0.2 * (z * 0.3).cos()));
        let built = t.elapsed();
        let s = WeatherState { wet: 1.0, fill: 0.02, snow: 0.1, falling: 1.0 };
        let t = std::time::Instant::now();
        let n = 50;
        for _ in 0..n {
            std::hint::black_box(surface(&d, &g, &s));
        }
        eprintln!("{}x{} texels: ground {:?}, map {:?} each", g.nx, g.nz, built, t.elapsed() / n);
    }

    #[test]
    fn the_flood_fills_a_closed_hollow_and_leaves_an_open_one() {
        // 5x5: a pit in the middle, walled; a notch to the edge on the right.
        #[rustfmt::skip]
        let h = vec![
            1.0, 1.0, 1.0, 1.0, 1.0,
            1.0, 0.5, 0.5, 0.5, 1.0,
            1.0, 0.5, 0.0, 0.5, 0.2,
            1.0, 0.5, 0.5, 0.5, 1.0,
            1.0, 1.0, 1.0, 1.0, 1.0,
        ];
        let spill = spill_levels(&h, 5, 5);
        assert_eq!(spill[12], 0.5, "the pit fills to the notch's rim");
        assert_eq!(spill[0], 1.0);
        let floor = hollow_floors(&h, &spill, 5, 5);
        assert_eq!(floor[12], 0.0);
        assert!(floor[0].is_infinite());
    }
}
