//! The phase-0 test data: synthetic meshes (generated here, checked in, and checked against this
//! generator so they cannot drift), the shared Recast configs, the crowd scenarios, and the
//! golden reference files.
//!
//! After changing a synthetic mesh below, rewrite the files with
//!   SPACE_SOUP_NAV_REGEN=1 cargo test --test test_data
//! and regenerate the golden files (`tests/data/golden/README.md`): they hash every mesh, so a
//! stale reference fails here rather than being compared against a mesh it was not built from.

use std::path::Path;

use glam::Vec3;
use space_soup_nav::asset::fnv1a64;
use space_soup_nav::golden::{self, Golden};
use space_soup_nav::obj::TriMesh;
use space_soup_nav::scenario::{self, Goal};

// --- synthetic meshes ----------------------------------------------------------------------------
//
// Metres, +Y up. Every floor's walkable top is at y = 0 (a 0.2 m slab under it); walls are 3 m.
// Each mesh isolates one situation the research found voxel navmeshes getting wrong, with the
// tricky dimension sitting on, just under, or just over a threshold of the configs.

const WALL: f32 = 3.0;

fn bx(min: [f32; 3], max: [f32; 3]) -> TriMesh {
    TriMesh::cuboid(Vec3::from(min), Vec3::from(max))
}

fn floor(x0: f32, z0: f32, x1: f32, z1: f32) -> TriMesh {
    bx([x0, -0.2, z0], [x1, 0.0, z1])
}

/// A wedge rising along +X from height 0 at `x0` to `h` at `x1`, across `z0..z1`. No bottom
/// face: it sits on a floor.
fn ramp(x0: f32, x1: f32, h: f32, z0: f32, z1: f32) -> TriMesh {
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    TriMesh {
        vertices: vec![v(x0, 0.0, z0), v(x0, 0.0, z1), v(x1, h, z1), v(x1, h, z0), v(x1, 0.0, z0), v(x1, 0.0, z1)],
        triangles: vec![
            [0, 1, 2], [0, 2, 3], // the slope, facing up and back
            [4, 3, 2], [4, 2, 5], // the high end, facing +X
            [0, 3, 4],            // the -Z side
            [1, 5, 2],            // the +Z side
        ],
    }
}

fn join(parts: &[TriMesh]) -> TriMesh {
    let mut m = TriMesh::default();
    for p in parts {
        m.append(p);
    }
    m
}

/// A wall along Z at `x` (0.2 m thick) from `z0` to `z1`, with `(centre, width)` doorways, 2.2 m
/// high, and a lintel above each.
fn wall_x(x: f32, z0: f32, z1: f32, doors: &[(f32, f32)]) -> TriMesh {
    let mut parts = Vec::new();
    let mut at = z0;
    for &(c, w) in doors {
        parts.push(bx([x - 0.1, 0.0, at], [x + 0.1, WALL, c - w / 2.0]));
        parts.push(bx([x - 0.1, 2.2, c - w / 2.0], [x + 0.1, WALL, c + w / 2.0]));
        at = c + w / 2.0;
    }
    parts.push(bx([x - 0.1, 0.0, at], [x + 0.1, WALL, z1]));
    join(&parts)
}

/// A wall along X at `z` (0.2 m thick) from `x0` to `x1`, with full-height gaps `(centre, width)`.
fn wall_z_gaps(z: f32, x0: f32, x1: f32, gaps: &[(f32, f32)]) -> TriMesh {
    let mut parts = Vec::new();
    let mut at = x0;
    for &(c, w) in gaps {
        parts.push(bx([at, 0.0, z - 0.1], [c - w / 2.0, WALL, z + 0.1]));
        at = c + w / 2.0;
    }
    parts.push(bx([at, 0.0, z - 0.1], [x1, WALL, z + 0.1]));
    join(&parts)
}

fn synthetic() -> Vec<(&'static str, &'static str, TriMesh)> {
    vec![
        (
            "two_rooms_door",
            "Two 6 x 8 m rooms joined by one 1.2 m doorway in a 0.2 m wall at x = 6. The doorway crowd scenarios run here.",
            join(&[floor(0.0, 0.0, 12.0, 8.0), wall_x(6.0, 0.0, 8.0, &[(4.0, 1.2)])]),
        ),
        (
            "doorway_offsets",
            "The same 1.2 m doorway three times, the wall shifted 0, 0.05 and 0.13 m off the voxel grid.\n\
             Recast's doorway width depends on where the grid falls; a piece baked once and placed\n\
             anywhere must not depend on that.",
            join(
                &[0.0f32, 0.05, 0.13]
                    .iter()
                    .enumerate()
                    .flat_map(|(i, off)| {
                        let z = i as f32 * 8.0;
                        [floor(0.0, z, 10.0, z + 6.0), wall_x(5.0 + off, z, z + 6.0, &[(z + 3.0, 1.2)])]
                    })
                    .collect::<Vec<_>>(),
            ),
        ),
        (
            "narrow_gaps",
            "Full-height gaps 0.8, 0.9, 1.0 and 1.7 m wide in a wall. A 0.4 m radius agent fits through\n\
             0.8 only exactly; the large 0.8 m agent only through 1.7.",
            join(&[
                floor(0.0, 0.0, 14.0, 8.0),
                wall_z_gaps(4.0, 0.0, 14.0, &[(2.0, 0.8), (5.0, 0.9), (8.0, 1.0), (11.5, 1.7)]),
            ]),
        ),
        (
            "thin_wall",
            "A 2 cm wall splitting a floor completely (the halves must not connect), and a 2 cm, 25 cm\n\
             high kerb an agent can step over.",
            join(&[
                floor(0.0, 0.0, 10.0, 10.0),
                bx([5.03, 0.0, 0.0], [5.05, WALL, 10.0]),
                bx([2.51, 0.0, 0.0], [2.53, 0.25, 10.0]),
            ]),
        ),
        (
            "stairs",
            "Two flights to a 1.2 m landing: 0.2 m risers (climbable at 0.3) and 0.4 m risers (not).",
            join(
                &std::iter::once(floor(0.0, 0.0, 12.0, 8.0))
                    .chain((1..=6).map(|i| bx([3.0 + 0.3 * (i - 1) as f32, 0.0, 1.0], [7.0, 0.2 * i as f32, 3.0])))
                    .chain((1..=3).map(|i| bx([3.0 + 0.4 * (i - 1) as f32, 0.0, 5.0], [7.0, 0.4 * i as f32, 7.0])))
                    .collect::<Vec<_>>(),
            ),
        ),
        (
            "ledge",
            "A 1 m high block (a ledge to walk off, not up) and a 0.25 m step that can be climbed.",
            join(&[floor(0.0, 0.0, 10.0, 10.0), bx([3.0, 0.0, 3.0], [7.0, 1.0, 7.0]), bx([8.0, 0.0, 8.0], [9.5, 0.25, 9.5])]),
        ),
        (
            "catwalk",
            "A 1.5 m catwalk 3 m over a floor: two walkable layers in the same columns, 2.8 m apart.",
            join(&[floor(0.0, 0.0, 10.0, 10.0), bx([0.0, 2.8, 4.25], [10.0, 3.0, 5.75])]),
        ),
        (
            "ramps",
            "Ramps of 30 degrees (walkable at 45) and 50 degrees (not), each up to a 1.5 m platform.",
            join(&[
                floor(0.0, 0.0, 16.0, 10.0),
                ramp(2.0, 2.0 + 2.598_076, 1.5, 1.0, 4.0),
                bx([2.0 + 2.598_076, 0.0, 1.0], [7.0, 1.5, 4.0]),
                ramp(2.0, 2.0 + 1.258_638, 1.5, 6.0, 9.0),
                bx([2.0 + 1.258_638, 0.0, 6.0], [7.0, 1.5, 9.0]),
            ]),
        ),
        (
            "l_corridor",
            "A 2 m wide corridor turning a right-angled corner, for crowds rounding it.",
            join(&[
                floor(0.0, 0.0, 10.0, 2.0),
                floor(8.0, 2.0, 10.0, 10.0),
                bx([-0.2, 0.0, -0.2], [10.2, WALL, 0.0]),
                bx([10.0, 0.0, 0.0], [10.2, WALL, 10.2]),
                bx([-0.2, 0.0, 2.0], [7.8, WALL, 2.2]),
                bx([7.8, 0.0, 2.0], [8.0, WALL, 10.2]),
                bx([-0.2, 0.0, 0.0], [0.0, WALL, 2.0]),
                bx([8.0, 0.0, 10.0], [10.0, WALL, 10.2]),
            ]),
        ),
    ]
}

fn data() -> std::path::PathBuf {
    golden::test_data_dir()
}

#[test]
fn synthetic_meshes_match_their_generator() {
    let dir = data().join("meshes/synthetic");
    let regen = std::env::var_os("SPACE_SOUP_NAV_REGEN").is_some();
    if regen {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, about, mesh) in synthetic() {
        let text = mesh.to_obj(&format!("{name}: generated by tests/test_data.rs, do not edit\n{about}"));
        let path = dir.join(format!("{name}.obj"));
        if regen {
            std::fs::write(&path, &text).unwrap();
        } else {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(on_disk == text, "{} differs from its generator: SPACE_SOUP_NAV_REGEN=1 cargo test --test test_data", path.display());
        }
    }
}

#[test]
fn synthetic_meshes_are_what_they_claim() {
    let meshes = synthetic();
    let get = |n: &str| &meshes.iter().find(|m| m.0 == n).unwrap().2;
    for (name, _, m) in &meshes {
        for t in &m.triangles {
            let (a, b, c) = (m.vertices[t[0] as usize], m.vertices[t[1] as usize], m.vertices[t[2] as usize]);
            assert!((b - a).cross(c - a).length() > 1e-6, "{name}: degenerate triangle");
        }
    }
    // The ramps' slopes, by the up component of their normals: cos 30 and cos 50.
    let ups: Vec<f32> = {
        let m = get("ramps");
        m.triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (m.vertices[t[0] as usize], m.vertices[t[1] as usize], m.vertices[t[2] as usize]);
                (b - a).cross(c - a).normalize().y
            })
            .filter(|y| *y > 0.1 && *y < 0.99)
            .collect()
    };
    assert_eq!(ups.len(), 4, "two triangles per slope");
    assert!(ups.iter().filter(|y| (**y - 0.866_025_4).abs() < 1e-4).count() == 2, "30 degree ramp: {ups:?}");
    assert!(ups.iter().filter(|y| (**y - 0.642_787_6).abs() < 1e-4).count() == 2, "50 degree ramp: {ups:?}");
}

// --- every mesh, the configs, the scenarios ------------------------------------------------------

fn all_meshes(data: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for group in ["recast", "synthetic", "blacksite"] {
        let mut names: Vec<String> = std::fs::read_dir(data.join("meshes").join(group))
            .unwrap_or_else(|e| panic!("meshes/{group}: {e}"))
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".obj"))
            .map(|n| format!("{group}/{n}"))
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no meshes in meshes/{group}");
        out.extend(names);
    }
    out
}

#[test]
fn every_mesh_parses_and_has_a_walkable_floor() {
    let data = data();
    for name in all_meshes(&data) {
        let m = TriMesh::load(&data.join("meshes").join(&name)).unwrap();
        assert!(!m.triangles.is_empty(), "{name}: no triangles");
        let up = m.triangles.iter().any(|t| {
            let (a, b, c) = (m.vertices[t[0] as usize], m.vertices[t[1] as usize], m.vertices[t[2] as usize]);
            let n = (b - a).cross(c - a);
            n.length() > 0.0 && n.normalize().y > 0.9
        });
        assert!(up || name.contains("NO_PATH"), "{name}: nothing faces up, so Recast would find no floor");
    }
}

#[test]
fn configs_load_and_are_buildable() {
    let configs = golden::load_configs(&data()).unwrap().configs;
    assert!(configs.len() >= 3);
    let mut names: Vec<_> = configs.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), configs.len(), "config names must be unique: golden files are named by them");
    for c in &configs {
        assert!(c.partition == "layers" || c.partition == "monotone", "{}: partition {}", c.name, c.partition);
        assert!(c.cell_size > 0.0 && c.cell_height > 0.0 && c.agent_radius > 0.0, "{}", c.name);
        assert!((3..=6).contains(&c.verts_per_poly), "{}: Detour takes at most 6", c.name);
    }
}

#[test]
fn scenarios_load_and_stand_on_their_mesh() {
    let data = data();
    let scenarios = scenario::load_all(&data).unwrap();
    for s in ["doorway_swap", "doorway_swap_36hz", "player_through_doorway", "corridor_chase", "converge", "corner"] {
        assert!(scenarios.iter().any(|x| x.name == s), "missing scenario {s}");
    }
    for s in &scenarios {
        let m = TriMesh::load(&data.join("meshes").join(&s.mesh)).unwrap_or_else(|e| panic!("{}: {e}", s.name));
        let (lo, hi) = m.bounds();
        let inside = |p: [f32; 3]| p[0] > lo.x && p[0] < hi.x && p[2] > lo.z && p[2] < hi.z;
        for a in &s.agents {
            assert!(inside(a.start), "{}: agent starts off the mesh at {:?}", s.name, a.start);
            if let Goal::Point(g) = a.goal {
                assert!(inside(g), "{}: goal off the mesh at {g:?}", s.name);
            }
        }
        for p in &s.players {
            assert!(p.waypoints.iter().all(|w| inside(*w)), "{}: player leaves the mesh", s.name);
        }
    }
}

// --- golden reference files ----------------------------------------------------------------------

#[test]
fn golden_files_cover_every_mesh_and_config_and_match_the_meshes() {
    let data = data();
    let goldens: Vec<(String, Golden)> = golden::load_all(&data).unwrap();
    let configs = golden::load_configs(&data).unwrap().configs;
    let meshes = all_meshes(&data);
    assert_eq!(goldens.len(), meshes.len() * configs.len(), "one golden file per mesh and config");

    for (file, g) in &goldens {
        let bytes = std::fs::read(data.join("meshes").join(&g.input.mesh)).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(
            g.input.mesh_fnv1a64,
            format!("{:016x}", fnv1a64(&bytes)),
            "{file}: {} changed since its reference was made; regenerate the golden files",
            g.input.mesh
        );
        let mesh = TriMesh::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!((g.input.vertices, g.input.triangles), (mesh.vertices.len(), mesh.triangles.len()), "{file}: the C++ loader read a different mesh");

        let c = configs.iter().find(|c| c.name == g.config_name).unwrap_or_else(|| panic!("{file}: unknown config"));
        let r = &g.recast_config;
        // The integer settings, derived from the shared config the way Recast's sample does.
        assert_eq!(r.walkable_radius, (c.agent_radius / c.cell_size).ceil() as i32, "{file}");
        assert_eq!(r.walkable_height, (c.agent_height / c.cell_height).ceil() as i32, "{file}");
        assert_eq!(r.walkable_climb, (c.agent_max_climb / c.cell_height).floor() as i32, "{file}");
        assert_eq!(r.max_edge_len, (c.edge_max_len / c.cell_size) as i32, "{file}");

        let Some(st) = &g.stages else { continue };
        assert!(st.heightfield_spans >= st.walkable_spans_after_filters, "{file}");
        assert!(st.compact_spans >= st.walkable_after_erode, "{file}");
        let pm = &st.polymesh;
        for p in &pm.polys {
            assert!((3..=pm.nvp as usize).contains(&p.verts.len()), "{file}");
            assert_eq!(p.verts.len(), p.neis.len(), "{file}");
            assert!(p.verts.iter().all(|&v| (v as usize) < pm.verts.len()), "{file}");
        }
        assert_eq!(st.detail.meshes.len(), pm.polys.len(), "{file}: one detail mesh per polygon");

        if let Some(q) = &g.queries {
            let n = pm.polys.len() as i32;
            assert!(q.nearest.iter().all(|x| x.poly >= -1 && x.poly < n), "{file}");
            assert!(q.raycast.iter().all(|x| x.path.iter().all(|&p| p >= 0 && p < n)), "{file}");
            assert!(q.move_along_surface.iter().all(|x| x.visited.iter().all(|&p| p >= 0 && p < n)), "{file}");
        }
    }
}
