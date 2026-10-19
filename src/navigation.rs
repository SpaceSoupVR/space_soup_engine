//! NAVIGATION: a scene's navmesh, baked from what its players collide with, and the paths a
//! creature walks on it. The navmesh itself is `space_soup_nav` (a Recast port matching C++
//! Recast's output, its own crate); this is the engine's side of it.
//!
//! - BAKED OFFLINE, from every static collider (`PhysicsWorld::static_collision_triangles`):
//!   cuboids, brushes, models' mesh colliders and the terrain alike, so a creature walks exactly
//!   where a player can stand. One piece for the whole scene, written beside the scene's other
//!   baked data as `navmesh/scenes/<scene>.human.ssnav`, with a `.json` saying what it was baked
//!   from. Never baked at load: a large level takes seconds.
//! - A BAKE KNOWS ITS SCENE: the geometry's fingerprint (its triangles to the millimetre) is
//!   kept with it, and a scene whose colliders have moved since says so when it loads, as the
//!   lightmaps do. It is still used: a stale navmesh is better than none.
//! - BIG SCENES: one bake holds at most `MAX_BAKE_COLUMNS` voxel columns (a 290 m square at the
//!   0.1 m cells a human-sized agent wants), so a larger scene is baked in coarser cells, chosen
//!   here and recorded. Tiling a level too big for that is not done.
//!
//! Metres in the API, as the engine uses; the navmesh works in integer millimetres inside.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DVec3, IVec3, Vec3};
use serde::{Deserialize, Serialize};
use space_soup_nav::asset::{NavPieceAsset, Turn};
use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput, MAX_BAKE_COLUMNS};
use space_soup_nav::bake::{Fixes, Partition, Settings};
use space_soup_nav::path::find_path;
use space_soup_nav::world::NavWorld;

use crate::rigid_physics::PhysicsWorld;
use crate::rigid_physics_nav::CollisionTriangles;

/// The agent every scene is baked for: a standing person (Blacksite's human class).
pub const AGENT: &str = "human";

/// How far from the floor a point may be and still be found on it (mm): a body's width across,
/// and below as far as a head is above the feet plus a jump.
const FLOOR_MM: DVec3 = DVec3::new(600.0, 2500.0, 600.0);

/// The human class's bake settings, at `cell_size` metres a cell.
pub fn human_settings(cell_size: f32) -> Settings {
    Settings {
        cell_size,
        cell_height: 0.1,
        agent_radius: 0.4,
        agent_height: 1.8,
        agent_max_climb: 0.3,
        agent_max_slope_deg: 45.0,
        partition: Partition::Monotone,
        region_min_size: 8,
        region_merge_size: 20,
        edge_max_len: 12.0,
        edge_max_error: 1.3,
        verts_per_poly: 6,
        detail_sample_dist: 6.0,
        detail_sample_max_error: 1.0,
        fixes: Fixes::default(),
    }
}

/// The finest cell (whole millimetres, at least 0.1 m) at which a scene this wide fits one bake.
pub fn cell_size_for(geometry: &CollisionTriangles) -> f32 {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in &geometry.vertices {
        lo = lo.min(Vec3::from(*v));
        hi = hi.max(Vec3::from(*v));
    }
    if lo.x > hi.x {
        return 0.1;
    }
    // A margin for the grid's border and rounding.
    let area = ((hi.x - lo.x + 2.0) * (hi.z - lo.z + 2.0)) as f64;
    let fits = (area / (MAX_BAKE_COLUMNS * 0.9)).sqrt();
    ((fits * 1000.0).ceil() / 1000.0).max(0.1) as f32
}

/// The geometry's fingerprint: every triangle's corners to the millimetre, in order (FNV-1a, so
/// the same on every machine and Rust version).
pub fn fingerprint(geometry: &CollisionTriangles) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |x: i64| {
        for b in x.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for t in &geometry.triangles {
        for &i in t {
            for c in geometry.vertices[i as usize] {
                eat((c as f64 * 1000.0).round() as i64);
            }
        }
    }
    format!("{h:016x}")
}

/// What a scene's navmesh was baked from, kept beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNavInfo {
    pub scene: String,
    pub agent: String,
    /// The colliders' [`fingerprint`] when it was baked.
    pub geometry: String,
    pub triangles: usize,
    pub cell_size: f32,
    pub polygons: usize,
    pub bake_ms: f64,
}

/// A scene's navmesh: the world the paths run on.
pub struct SceneNav {
    pub world: NavWorld,
    pub info: SceneNavInfo,
    asset: Arc<NavPieceAsset>,
    /// The scene's colliders have changed since it was baked.
    pub stale: bool,
}

/// Where scene `scene`'s navmesh and its info are kept under `game_dir`.
pub fn files(game_dir: &Path, scene: &str) -> (PathBuf, PathBuf) {
    let dir = game_dir.join("navmesh/scenes");
    (dir.join(format!("{scene}.{AGENT}.ssnav")), dir.join(format!("{scene}.{AGENT}.json")))
}

fn world_of(asset: &Arc<NavPieceAsset>) -> NavWorld {
    let mut w = NavWorld::new();
    // Baked in world space: placed where it was baked.
    w.add_island(asset.clone(), Turn(0), IVec3::ZERO);
    w
}

impl SceneNav {
    /// Bakes scene `scene`'s navmesh from the colliders in `physics` (a scene built into it).
    pub fn bake(physics: &PhysicsWorld, scene: &str) -> Result<SceneNav, String> {
        let geometry = physics.static_collision_triangles();
        if geometry.triangles.is_empty() {
            return Err(format!("{scene}: no static colliders to bake a navmesh from"));
        }
        for s in &geometry.skipped {
            log::warn!("navigation: {scene}: a {s} collider is left out of the navmesh");
        }
        let cell_size = cell_size_for(&geometry);
        let settings = human_settings(cell_size);
        let input = PieceInput { name: scene, vertices: &geometry.vertices, triangles: &geometry.triangles, sockets: &[], links: Vec::new() };
        let (asset, report) = bake_piece(&input, &settings, [1.0; 3], &ExportOptions::default())?;
        let info = SceneNavInfo {
            scene: scene.to_string(),
            agent: AGENT.to_string(),
            geometry: fingerprint(&geometry),
            triangles: geometry.triangles.len(),
            cell_size,
            polygons: asset.polys.len(),
            bake_ms: report.build_ms + report.export_ms,
        };
        let asset = Arc::new(asset);
        Ok(SceneNav { world: world_of(&asset), info, asset, stale: false })
    }

    /// Writes it under `game_dir` ([`files`]).
    pub fn save(&self, game_dir: &Path) -> Result<(), String> {
        let (nav, info) = files(game_dir, &self.info.scene);
        if let Some(dir) = nav.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&nav, self.asset.to_bytes()).map_err(|e| format!("{}: {e}", nav.display()))?;
        let text = serde_json::to_string_pretty(&self.info).map_err(|e| e.to_string())? + "\n";
        std::fs::write(&info, text).map_err(|e| format!("{}: {e}", info.display()))
    }

    /// Scene `scene`'s navmesh from under `game_dir`, if it has been baked. With `physics` (the
    /// scene built into it), checked against the colliders it now has: [`Self::stale`].
    pub fn load(game_dir: &Path, scene: &str, physics: Option<&PhysicsWorld>) -> Result<Option<SceneNav>, String> {
        let (nav, info_path) = files(game_dir, scene);
        let Ok(bytes) = std::fs::read(&nav) else { return Ok(None) };
        let asset = NavPieceAsset::from_bytes(&bytes).map_err(|e| format!("{}: {e:?}", nav.display()))?;
        let text = std::fs::read_to_string(&info_path).map_err(|e| format!("{}: {e}", info_path.display()))?;
        let info: SceneNavInfo = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", info_path.display()))?;
        let stale = physics.is_some_and(|p| fingerprint(&p.static_collision_triangles()) != info.geometry);
        let asset = Arc::new(asset);
        Ok(Some(SceneNav { world: world_of(&asset), info, asset, stale }))
    }

    /// The nearest point on the floor to `at`, if there is floor near it (metres).
    pub fn nearest(&self, at: Vec3) -> Option<Vec3> {
        self.world.nearest_poly(at.as_dvec3() * 1000.0, FLOOR_MM).map(|n| (n.point / 1000.0).as_vec3())
    }

    /// A walkable path from `from` to `to` (metres), corners and all, or `None` if either is off
    /// the floor or nothing joins them.
    pub fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let path = find_path(&self.world, from.as_dvec3() * 1000.0, to.as_dvec3() * 1000.0, FLOOR_MM).filter(|p| p.complete)?;
        Some(path.waypoints.iter().map(|w| (w.pos() / 1000.0).as_vec3()).collect())
    }
}
