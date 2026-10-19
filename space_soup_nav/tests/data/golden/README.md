# Golden reference files

What C++ Recast/Detour produces on every mesh in `../meshes` under every config in
`../golden_configs.json`: one file per pair, named `<group>__<mesh>__<config>.json`, read by
`space_soup_nav::golden`. The Rust build is tested against these stage by stage.

## How they were made

- Recast/Detour at commit `9f4ce64458dfae86e1239c525ddc219c4e9e06f1` (2026-02-27).
- The tool is `nav_golden_tool/golden.cpp`, **outside** the Space Soup repositories, so no C++ is
  built or shipped with the engine. It mirrors RecastDemo's `Sample_SoloMesh::build` exactly:
  - the same config derivation;
  - all three span filters;
  - mesh bounds as build bounds;
  - one Detour tile with every polygon's flag set to 1.
- Compiled with `-O2 -ffp-contract=off` (no fused multiply-add, no fast-math).

```sh
cd nav_golden_tool && ./build.sh
./golden <path to space_soup_nav/tests/data> 9f4ce64458dfae86e1239c525ddc219c4e9e06f1
```

Regenerate whenever a mesh or config changes. Each file records an FNV-1a 64 hash of its mesh, and
`tests/test_data.rs` fails if a mesh no longer matches the reference built from it.

## What is recorded

- Each build stage the port keeps:
  - span counts after rasterising, after filtering, compacted, and after erosion;
  - regions (layers and monotone; watershed is deferred);
  - contours;
  - the polygon mesh;
  - the detail mesh.
- Detour's geometric queries, from a seeded LCG with seed 12345. Every input is written out, so
  nothing needs to reproduce the generator:
  - 200 nearest-polygon queries;
  - 100 raycasts;
  - 100 move-along-surface queries.
- Polygon references are tile polygon indices; -1 means none.
- Path search is not recorded, on purpose: Detour's A* is not optimal, so ours is tested against
  the true shortest path instead.

## Where Recast is wrong, measured here

These files are the reference for **parity**, not for correctness. Two things the synthetic
meshes were built to probe came out wrong in Recast itself (2026-10-17):

| Mesh | Config | Geometry says | Recast gives |
|---|---|---|---|
| `narrow_gaps` | human r 0.4, cs 0.2 | 0.9, 1.0 and 1.7 m gaps passable | only 1.7 m |
| `narrow_gaps` | human r 0.4, cs 0.1 | 0.9, 1.0 and 1.7 m gaps passable | 1.0 m as a 0.1 m sliver, and 1.7 m |
| `narrow_gaps` | large r 0.8, cs 0.1 | 1.7 m gap passable (0.1 m spare) | nothing passes |
| `stairs` | human, cs 0.2 | 0.2 m risers (climb 0.3) reach the 1.2 m landing | no walkable surface above the floor at all |

- **Narrow gaps.** Walls rasterise onto every cell they touch, and the erosion radius is rounded
  up, so one to two cells of width are lost.
- **Stairs.** The ledge filter rejects a span whose walkable neighbours differ by more than the
  climb. At 0.2 m cells, two 0.3 m treads put neighbours 0.4 m apart.

Both are now fixed in the baker, behind `bake::Fixes` (on by default; `Fixes::recast()` turns them
off and the parity test still matches Recast exactly). `tests/truth.rs` checks each:

- **Stairs.** The ledge filter allows reachable neighbours to differ by as much as a walkable
  slope rises over two cells, not just one climb. 0.2 m stairs now reach their landing at 0.2 m
  cells, while 0.4 m risers, a 1 m ledge and a 50 degree ramp stay cut off.
- **Narrow gaps.** Walls are kept at their real distance: the voxels still decide whether a missing
  neighbour is a wall or a drop, but a span next to a wall is judged by the exact distance from
  its cell centre to the wall's geometry. With the default margin (half a cell) no point of the
  navmesh is nearer a wall than the agent's radius, and every gap leaving two cells of clearance
  is open; with no margin every gap leaving one cell is open, and agents may graze walls by half a
  cell. A voxel grid cannot do better than a cell either way.

The original plan for these was:

1. Match these files where Recast is right.
2. Pass the geometric truth where it is not (the clearance at doorways is exactly the case the
   design research flagged).
