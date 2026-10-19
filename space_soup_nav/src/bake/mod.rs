//! The offline baker: room geometry in, a navmesh out. Runs at build time on a desktop, never on
//! the headset; what it writes is the integer [`crate::asset::NavPieceAsset`].
//!
//! The voxel pipeline is a port of Recast at commit `9f4ce64` and is checked stage by stage
//! against C++ Recast's output on the same inputs (`tests/parity.rs`, data in
//! `tests/data/golden`). Watershed partitioning is deliberately absent (see the design report).

#[allow(clippy::needless_range_loop)]
pub mod clearance;
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
pub mod compact;
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
pub mod contour;
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
pub mod detail;
pub mod export;
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
pub mod heightfield;
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
pub mod polymesh;
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
pub mod region;

pub use region::Partition;

use compact::CompactHeightfield;
use heightfield::Heightfield;

/// Recast's area ids: 0 is not walkable, 63 is plain walkable ground.
pub const NULL_AREA: u8 = 0;
pub const WALKABLE_AREA: u8 = 63;

/// Where the baker departs from Recast to get closer to the geometry. Each is measured by
/// `tests/truth.rs`; [`Fixes::recast`] turns them all off for exact parity with C++ Recast
/// (`tests/parity.rs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fixes {
    /// Erode by the real distance to walls instead of whole voxels (see [`clearance`]).
    pub exact_wall_clearance: bool,
    /// With exact clearance, how much more than the radius a cell's centre must clear a wall by,
    /// in cells. A kept cell reaches half a cell nearer the wall than its centre, so 0.5 means
    /// no point of the navmesh is closer to a wall than the radius (safe); 0 lets agents graze
    /// walls by up to half a cell but keeps a gap open with less to spare. A gap is guaranteed
    /// open when it leaves the agent `(1 + 2 * margin)` cells of clearance (`tests/truth.rs`).
    pub wall_margin_cells: f32,
    /// Let reachable neighbours differ by as much as a walkable slope does over two cells, not
    /// only by one climb, so stairs survive coarse cells.
    pub slope_aware_ledges: bool,
}

impl Fixes {
    pub const fn recast() -> Fixes {
        Fixes { exact_wall_clearance: false, wall_margin_cells: 0.0, slope_aware_ledges: false }
    }
}

impl Default for Fixes {
    fn default() -> Self {
        Fixes { exact_wall_clearance: true, wall_margin_cells: 0.5, slope_aware_ledges: true }
    }
}

/// Agent and voxel settings in the units people author them in (metres, degrees, and Recast
/// sample conventions for the rest; see [`crate::golden::GoldenConfig`] for each unit).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub cell_size: f32,
    pub cell_height: f32,
    pub agent_radius: f32,
    pub agent_height: f32,
    pub agent_max_climb: f32,
    pub agent_max_slope_deg: f32,
    pub partition: Partition,
    pub region_min_size: i32,
    pub region_merge_size: i32,
    pub edge_max_len: f32,
    pub edge_max_error: f32,
    pub verts_per_poly: i32,
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
    pub fixes: Fixes,
}

/// The settings in cells, over a grid covering `bmin..bmax`: Recast's `rcConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub cs: f32,
    pub ch: f32,
    pub width: i32,
    pub height: i32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub walkable_slope_angle: f32,
    pub walkable_height: i32,
    pub walkable_climb: i32,
    pub walkable_radius: i32,
    pub max_edge_len: i32,
    pub max_simplification_error: f32,
    pub min_region_area: i32,
    pub merge_region_area: i32,
    pub max_verts_per_poly: i32,
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
}

impl Settings {
    /// Derived exactly as RecastDemo's `Sample_SoloMesh` derives it.
    pub fn config(&self, bmin: [f32; 3], bmax: [f32; 3]) -> Config {
        let (cs, ch) = (self.cell_size, self.cell_height);
        Config {
            cs,
            ch,
            width: ((bmax[0] - bmin[0]) / cs + 0.5) as i32,
            height: ((bmax[2] - bmin[2]) / cs + 0.5) as i32,
            bmin,
            bmax,
            walkable_slope_angle: self.agent_max_slope_deg,
            walkable_height: (self.agent_height / ch).ceil() as i32,
            walkable_climb: (self.agent_max_climb / ch).floor() as i32,
            walkable_radius: (self.agent_radius / cs).ceil() as i32,
            max_edge_len: (self.edge_max_len / cs) as i32,
            max_simplification_error: self.edge_max_error,
            min_region_area: (self.region_min_size as f32 * self.region_min_size as f32) as i32,
            merge_region_area: (self.region_merge_size as f32 * self.region_merge_size as f32) as i32,
            max_verts_per_poly: self.verts_per_poly,
            detail_sample_dist: if self.detail_sample_dist < 0.9 { 0.0 } else { cs * self.detail_sample_dist },
            detail_sample_max_error: ch * self.detail_sample_max_error,
        }
    }
}

/// `rcCalcBounds`.
pub fn bounds(verts: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut lo = verts[0];
    let mut hi = verts[0];
    for v in &verts[1..] {
        for i in 0..3 {
            lo[i] = lo[i].min(v[i]);
            hi[i] = hi[i].max(v[i]);
        }
    }
    (lo, hi)
}

/// Counts at each stage, the same ones the golden files record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageCounts {
    pub heightfield_spans: usize,
    pub walkable_spans_after_filters: usize,
    pub compact_spans: usize,
    pub walkable_after_erode: usize,
    pub max_regions: u16,
}

/// Everything the build produced, kept for tests and debug views.
pub struct Build {
    pub config: Config,
    pub counts: StageCounts,
    pub chf: CompactHeightfield,
    pub contours: contour::ContourSet,
    pub polymesh: polymesh::PolyMesh,
    pub detail: detail::DetailMesh,
    /// Milliseconds per stage: rasterise+filter, compact+erode, regions, contours, polygons, detail.
    pub timings: [f64; 6],
}

/// Runs the pipeline, Recast's `Sample_SoloMesh::build` order, with all three span filters.
pub fn build(verts: &[[f32; 3]], tris: &[[u32; 3]], settings: &Settings) -> Build {
    let (bmin, bmax) = bounds(verts);
    let cfg = settings.config(bmin, bmax);
    let mut counts = StageCounts::default();
    let mut timings = [0.0f64; 6];
    let mut clock = std::time::Instant::now();
    let mut lap = |slot: usize| {
        timings[slot] = clock.elapsed().as_secs_f64() * 1000.0;
        clock = std::time::Instant::now();
    };

    let mut hf = Heightfield::new(&cfg);
    let areas = heightfield::mark_walkable_triangles(cfg.walkable_slope_angle, verts, tris);
    heightfield::rasterize_triangles(&mut hf, verts, tris, &areas, cfg.walkable_climb);
    counts.heightfield_spans = hf.span_count();
    heightfield::filter_low_hanging_obstacles(&mut hf, cfg.walkable_climb);
    let max_spread = if settings.fixes.slope_aware_ledges {
        let rise = 2.0 * cfg.cs as f64 * libm::tan((cfg.walkable_slope_angle as f64).to_radians()) / cfg.ch as f64;
        cfg.walkable_climb.max((rise + 1e-4).floor() as i32)
    } else {
        cfg.walkable_climb
    };
    heightfield::filter_ledge_spans(&mut hf, cfg.walkable_height, cfg.walkable_climb, max_spread);
    heightfield::filter_low_height_spans(&mut hf, cfg.walkable_height);
    counts.walkable_spans_after_filters = hf.walkable_span_count();
    lap(0);

    let mut chf = CompactHeightfield::build(cfg.walkable_height, cfg.walkable_climb, &hf);
    counts.compact_spans = chf.spans.len();
    if settings.fixes.exact_wall_clearance {
        clearance::erode_exact(&mut chf, &hf, verts, tris, &cfg, settings.agent_radius + settings.fixes.wall_margin_cells * cfg.cs);
    } else {
        compact::erode_walkable_area(&mut chf, cfg.walkable_radius);
    }
    drop(hf);
    counts.walkable_after_erode = chf.walkable_count();
    lap(1);
    region::build_regions(&mut chf, settings.partition, 0, cfg.min_region_area, cfg.merge_region_area);
    counts.max_regions = chf.max_regions;
    lap(2);
    let contours = contour::build_contours(&chf, cfg.max_simplification_error, cfg.max_edge_len, contour::TESS_WALL_EDGES);
    lap(3);
    let polymesh = polymesh::build_poly_mesh(&contours, cfg.max_verts_per_poly as usize);
    lap(4);
    let detail = detail::build_poly_mesh_detail(&polymesh, &chf, cfg.detail_sample_dist, cfg.detail_sample_max_error);
    lap(5);

    Build { config: cfg, counts, chf, contours, polymesh, detail, timings }
}
