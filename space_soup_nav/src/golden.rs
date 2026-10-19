//! Reference ("golden") results from C++ Recast/Detour, read by the tests of the later phases.
//!
//! The files are produced once by a throwaway tool outside this repository
//! (`nav_golden_tool/`, see `tests/data/golden/README.md`) that runs Recast/Detour at a pinned
//! commit on every mesh in `tests/data/meshes` under every config in
//! `tests/data/golden_configs.json`. No C++ is built or shipped with this crate: the reference is
//! data.
//!
//! Only what the research said is worth matching is recorded: the build stages we keep
//! (rasterise, filters, compact heightfield, erosion, LAYERS and MONOTONE regions, contours,
//! polygon mesh, detail mesh; watershed is deferred) and Detour's GEOMETRIC queries (nearest
//! polygon, raycast, move along surface). Path search is not here on purpose: Detour's paths are
//! not optimal, so ours are tested against the true shortest path instead.
//!
//! Recast units are kept as Recast writes them (cell coordinates for the polygon mesh, metres for
//! detail vertices and queries), so a comparison never depends on a conversion we wrote.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One agent and voxel setting to run Recast with. The same file drives the C++ tool and the
/// Rust tests, so they cannot disagree about what was configured.
///
/// Units are those of Recast's `Sample_SoloMesh` settings panel, and are turned into
/// [`RecastConfig`] the way that sample does it, so the reference is Recast as people use it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenConfig {
    pub name: String,
    /// Voxel size across (m) and up (m).
    pub cell_size: f32,
    pub cell_height: f32,
    /// Metres; becomes cells by `ceil(r / cs)`.
    pub agent_radius: f32,
    /// Metres; becomes cells by `ceil(h / ch)`.
    pub agent_height: f32,
    /// Metres; becomes cells by `floor(c / ch)`.
    pub agent_max_climb: f32,
    pub agent_max_slope_deg: f32,
    /// "layers" or "monotone".
    pub partition: String,
    /// Square root of the area in cells (squared to get `minRegionArea`).
    pub region_min_size: i32,
    /// Square root of the area in cells (squared to get `mergeRegionArea`).
    pub region_merge_size: i32,
    /// Metres; becomes cells by `trunc(len / cs)`.
    pub edge_max_len: f32,
    /// Cells (Recast's `maxSimplificationError` is already in cells).
    pub edge_max_error: f32,
    pub verts_per_poly: i32,
    /// In cells: multiplied by `cell_size`, or 0 (no sampling) when below 0.9.
    pub detail_sample_dist: f32,
    /// In cell heights: multiplied by `cell_height`.
    pub detail_sample_max_error: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenConfigs {
    pub configs: Vec<GoldenConfig>,
}

/// Recast's own config, as the C++ tool derived it from a [`GoldenConfig`] (cells, rounded the way
/// Recast's sample rounds them). The Rust build must derive the same integers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecastConfig {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenContour {
    pub region: u16,
    pub area: u8,
    /// Simplified contour vertices: x, y, z in cells, then Recast's region/flags word.
    pub verts: Vec<[i32; 4]>,
    pub raw_vertex_count: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenPoly {
    /// Vertex indices; Recast's RC_MESH_NULL_IDX padding removed.
    pub verts: Vec<u16>,
    /// Per edge: Recast's neighbour word (0x8000 bit = border/portal flag, low bits = polygon or direction).
    pub neis: Vec<u16>,
    pub region: u16,
    pub area: u8,
    pub flags: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenPolyMesh {
    pub bmin: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    pub nvp: i32,
    pub border_size: i32,
    /// Cell coordinates.
    pub verts: Vec<[u16; 3]>,
    pub polys: Vec<GoldenPoly>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenDetail {
    /// Per polygon: vertex base, vertex count, triangle base, triangle count.
    pub meshes: Vec<[u32; 4]>,
    /// Metres.
    pub verts: Vec<[f32; 3]>,
    /// Three indices and Recast's edge flags.
    pub tris: Vec<[u8; 4]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenStages {
    /// Solid spans after rasterising.
    pub heightfield_spans: i64,
    /// Spans still walkable after the three filters.
    pub walkable_spans_after_filters: i64,
    pub compact_spans: i64,
    /// Compact spans still walkable after eroding by the agent radius.
    pub walkable_after_erode: i64,
    /// Regions found (the highest region id + 1, as Recast reports `maxRegions`).
    pub max_regions: i32,
    pub contours: Vec<GoldenContour>,
    pub polymesh: GoldenPolyMesh,
    pub detail: GoldenDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NearestQuery {
    pub point: [f32; 3],
    pub half_extents: [f32; 3],
    /// Polygon index in the single tile, or -1 when none was found.
    pub poly: i32,
    pub nearest: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaycastQuery {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub start_poly: i32,
    /// Fraction of the way the ray got; Detour returns f32::MAX when it reached the end.
    pub t: f32,
    pub hit_normal: [f32; 3],
    pub path: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveQuery {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub start_poly: i32,
    pub result: [f32; 3],
    pub visited: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenQueries {
    pub seed: u32,
    pub nearest: Vec<NearestQuery>,
    pub raycast: Vec<RaycastQuery>,
    pub move_along_surface: Vec<MoveQuery>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenTool {
    pub recast_commit: String,
    pub tool_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenInput {
    /// Relative to `tests/data/meshes`.
    pub mesh: String,
    pub vertices: usize,
    pub triangles: usize,
    /// FNV-1a 64 of the mesh file's bytes, so a changed mesh is noticed instead of compared
    /// against a reference built from the old one.
    pub mesh_fnv1a64: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Golden {
    pub tool: GoldenTool,
    pub input: GoldenInput,
    pub config_name: String,
    pub recast_config: RecastConfig,
    /// `None` when Recast produced no walkable surface (still a valid reference: ours must agree).
    pub stages: Option<GoldenStages>,
    pub queries: Option<GoldenQueries>,
}

/// Where the test data lives. Overridable with `SPACE_SOUP_NAV_TEST_DATA`, which is how the test
/// binary finds it when run on the Quest (the source tree is not there).
pub fn test_data_dir() -> PathBuf {
    match std::env::var_os("SPACE_SOUP_NAV_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data"),
    }
}

pub fn load_configs(data: &Path) -> Result<GoldenConfigs, String> {
    let path = data.join("golden_configs.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Every golden file, sorted by name.
pub fn load_all(data: &Path) -> Result<Vec<(String, Golden)>, String> {
    let dir = data.join("golden");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let path = dir.join(&n);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let g: Golden = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((n, g))
        })
        .collect()
}
