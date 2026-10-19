//! Navigation for Space Soup.
//!
//! Levels here are built from room pieces joined at doorway sockets, so navigation is built the
//! same way: each piece's navmesh is baked once, offline, per agent size class and scale variant
//! ([`asset::NavPieceAsset`]); at load the pieces are placed by quarter turns and joined at their
//! sockets; paths and crowds run on the result. The design and the research behind it are in
//! `reports/Rust navmesh design research.md`; the determinism rules in `DETERMINISM.md`.
//!
//! Done so far:
//! - Phase 0: the asset format ([`asset`]), the reference ("golden") data from C++ Recast
//!   ([`golden`]), the crowd scenarios ([`scenario`]) and the Quest test harness
//!   (`scripts/quest_test.sh`).
//! - Phase 1: the offline baker ([`bake`]). A port of Recast that reproduces its output exactly
//!   on every reference mesh (`tests/parity.rs`), plus what a kit of room pieces needs
//!   ([`bake::export`]): doorways cut exactly at their socket planes, integer millimetres, convex
//!   merging across regions, detail only on stairs and ramps. `examples/bake_kit.rs` bakes a
//!   whole kit; `tests/truth.rs` lists where the geometry and Recast still disagree.
//! - Phase 2: the runtime world ([`world`]). Baked pieces placed by quarter turns and joined at
//!   their doorways into portals, typed links (vents, ladders, drops), doors that switch links off
//!   and announce it, connected-area queries and the room graph. A whole generated level (about
//!   200 pieces) assembles in well under a millisecond (`tests/world.rs`).
//! - Phase 3: queries and paths. Point queries ([`query`]) agree with C++ Detour on its own
//!   test queries (`tests/query_parity.rs`); path search ([`path`]) is A* with moving entry points
//!   plus the funnel, guided across rooms by the room graph's real walking distances, measured
//!   within 0.01% of the true shortest path on average (`tests/path.rs`); searches run in slices
//!   under a per-frame budget ([`path::PathScheduler`]); [`follow::PathFollower`] turns a path
//!   into where an agent should steer each tick (`tests/chase.rs`).
//! - Phase 4: crowds ([`crowd`]). ORCA avoidance with walls and players that never yield, turns
//!   at narrow places (found on the navmesh), attack slots round each player, stopped agents
//!   making way; the game moves the agents and reports back. Judged by the scenarios written in
//!   phase 0 (`tests/crowd.rs`).
//! - Phase 5: tooling. Debug drawing of the world, paths and crowds for any renderer, or as an
//!   SVG ([`debug`]); checks for bakes, kits and levels with a connectivity report ([`validate`],
//!   `examples/nav_report.rs`).

pub mod asset;
pub mod bake;
pub mod crowd;
pub mod debug;
pub mod follow;
pub mod golden;
pub mod net;
pub mod obj;
pub mod path;
pub mod query;
pub mod scenario;
pub mod validate;
pub mod world;
