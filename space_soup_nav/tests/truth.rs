//! What the geometry says, as opposed to what Recast says (`tests/parity.rs`). These are the
//! targets for doing better than Recast; the ones the voxel baker does not meet yet are ignored,
//! so `cargo test -- --ignored` lists exactly how far there is to go.

use space_soup_nav::bake::{self, Partition, Settings};
use space_soup_nav::golden;
use space_soup_nav::obj::TriMesh;

fn settings(cs: f32, radius: f32, height: f32) -> Settings {
    Settings {
        cell_size: cs,
        cell_height: 0.1,
        agent_radius: radius,
        agent_height: height,
        agent_max_climb: 0.3,
        agent_max_slope_deg: 45.0,
        partition: Partition::Layers,
        region_min_size: 8,
        region_merge_size: 20,
        edge_max_len: 12.0,
        edge_max_error: 1.3,
        verts_per_poly: 6,
        detail_sample_dist: 6.0,
        detail_sample_max_error: 1.0,
        fixes: space_soup_nav::bake::Fixes::default(),
    }
}

/// x ranges of polygons crossing the wall line z = 4 of `narrow_gaps`.
fn crossings(s: &Settings) -> Vec<(f32, f32)> {
    let mesh = TriMesh::load(&golden::test_data_dir().join("meshes/synthetic/narrow_gaps.obj")).unwrap();
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
    let b = bake::build(&verts, &mesh.triangles, s);
    let pm = &b.polymesh;
    (0..pm.npolys())
        .filter_map(|i| {
            let pts: Vec<[f32; 2]> = pm.poly_verts(i).iter().map(|&v| {
                let v = pm.verts[v as usize];
                [pm.bmin[0] + v[0] as f32 * pm.cs, pm.bmin[2] + v[2] as f32 * pm.cs]
            }).collect();
            let (zlo, zhi) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
            (zlo < 3.9 && zhi > 4.1).then(|| pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0]))))
        })
        .collect()
}

fn passable_gaps(s: &Settings) -> Vec<f32> {
    let gaps = [(2.0, 0.8), (5.0, 0.9), (8.0, 1.0), (11.5, 1.7)];
    let c = crossings(s);
    gaps.iter().filter(|(x, _)| c.iter().any(|(lo, hi)| lo <= x && x <= hi)).map(|g| g.1).collect()
}

#[test]
fn a_gap_wider_than_the_agent_by_a_cell_is_passable() {
    assert!(passable_gaps(&settings(0.2, 0.4, 1.8)).contains(&1.7));
    assert!(passable_gaps(&settings(0.1, 0.4, 1.8)).contains(&1.7));
}

/// A voxel grid cannot hold a corridor narrower than a cell. With a margin of `m` cells a gap is
/// guaranteed open when it leaves `(1 + 2m)` cells of clearance, and no gap narrower than the
/// agent ever opens. Checked for the safe default (m = 0.5: the navmesh never comes nearer a
/// wall than the radius) and for m = 0 (agents may graze walls by half a cell). Recast's voxels
/// lose 1-2 cells more (`tests/data/golden/README.md`).
#[test]
fn every_gap_with_the_promised_clearance_is_passable_and_none_too_narrow_is() {
    let gaps = [0.8f32, 0.9, 1.0, 1.7];
    let mut misses = Vec::new();
    for margin in [0.5f32, 0.0] {
        for (cs, r) in [(0.2f32, 0.4f32), (0.1, 0.4), (0.1, 0.8), (0.05, 0.8)] {
            let mut s = settings(cs, r, 1.8);
            s.fixes.wall_margin_cells = margin;
            let got = passable_gaps(&s);
            for g in gaps {
                let clearance = g - 2.0 * r;
                let promised = clearance >= cs * (1.0 + 2.0 * margin) - 1e-4;
                let impossible = clearance < -1e-4;
                if promised && !got.contains(&g) {
                    misses.push(format!("margin {margin}, cs {cs}, radius {r}: {g} m gap closed ({clearance:.2} m to spare)"));
                }
                if impossible && got.contains(&g) {
                    misses.push(format!("margin {margin}, cs {cs}, radius {r}: {g} m gap open, but the agent does not fit"));
                }
            }
            println!("margin {margin}, cs {cs}, radius {r}: passable {got:?}");
        }
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

#[test]
fn recast_itself_still_loses_those_gaps() {
    let mut s = settings(0.2, 0.4, 1.8);
    s.fixes = space_soup_nav::bake::Fixes::recast();
    assert_eq!(passable_gaps(&s), vec![1.7]);
}

#[test]
fn stairs_within_the_climb_are_walkable_at_any_cell_size() {
    let mesh = TriMesh::load(&golden::test_data_dir().join("meshes/synthetic/stairs.obj")).unwrap();
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
    for cs in [0.1, 0.2] {
        let b = bake::build(&verts, &mesh.triangles, &settings(cs, 0.4, 1.8));
        let top = b.detail.verts.iter().any(|v| v[1] > 1.0);
        assert!(top, "cs {cs}: the 1.2 m landing reached by 0.2 m risers is not walkable");
    }
}

/// Which polygons of a bake are connected, by flood fill over their shared edges.
fn islands(b: &bake::Build) -> Vec<usize> {
    let pm = &b.polymesh;
    let n = pm.npolys();
    let mut label = vec![usize::MAX; n];
    for start in 0..n {
        if label[start] != usize::MAX {
            continue;
        }
        let mut stack = vec![start];
        label[start] = start;
        while let Some(p) = stack.pop() {
            for &nb in pm.poly_neis(p) {
                if nb & 0x8000 == 0 && (nb as usize) < n && label[nb as usize] == usize::MAX {
                    label[nb as usize] = start;
                    stack.push(nb as usize);
                }
            }
        }
    }
    label
}

/// The island of the polygon whose floor at `(x, z)`, read from its height detail, is near `y`.
fn island_at(b: &bake::Build, labels: &[usize], x: f32, z: f32, y: f32) -> Option<usize> {
    let d = &b.detail;
    (0..b.polymesh.npolys()).find(|&p| {
        let [vb, _, tb, tc] = d.meshes[p];
        d.tris[tb as usize..(tb + tc) as usize].iter().any(|t| {
            let v = [0, 1, 2].map(|k| d.verts[(vb + t[k] as u32) as usize]);
            let det = (v[1][0] - v[0][0]) * (v[2][2] - v[0][2]) - (v[2][0] - v[0][0]) * (v[1][2] - v[0][2]);
            if det.abs() < 1e-9 {
                return false;
            }
            let u = ((x - v[0][0]) * (v[2][2] - v[0][2]) - (v[2][0] - v[0][0]) * (z - v[0][2])) / det;
            let w = ((v[1][0] - v[0][0]) * (z - v[0][2]) - (x - v[0][0]) * (v[1][2] - v[0][2])) / det;
            let inside = u >= -1e-4 && w >= -1e-4 && u + w <= 1.0 + 1e-4;
            let h = v[0][1] + (v[1][1] - v[0][1]) * u + (v[2][1] - v[0][1]) * w - b.config.ch;
            inside && (h - y).abs() < 0.25
        })
    }).map(|p| labels[p])
}

#[test]
fn nothing_too_high_or_too_steep_becomes_climbable() {
    for cs in [0.1, 0.2] {
        let s = settings(cs, 0.4, 1.8);
        let load = |name: &str| {
            let mesh = TriMesh::load(&golden::test_data_dir().join(format!("meshes/synthetic/{name}.obj"))).unwrap();
            let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
            bake::build(&verts, &mesh.triangles, &s)
        };
        let b = load("stairs");
        let l = islands(&b);
        let floor = island_at(&b, &l, 1.0, 6.0, 0.0).expect("floor");
        let steep_top = island_at(&b, &l, 6.0, 6.0, 1.2);
        println!("cs {cs}: landing above the 0.4 m risers {}", if steep_top.is_some() { "exists, cut off" } else { "too small to keep" });
        if let Some(top) = steep_top {
            assert_ne!(top, floor, "cs {cs}: 0.4 m risers became climbable");
        }
        let a_top = island_at(&b, &l, 6.0, 2.0, 1.2).expect("landing A");
        assert_eq!(a_top, floor, "cs {cs}: 0.2 m risers do not reach the landing");
        let b = load("ledge");
        let l = islands(&b);
        let floor = island_at(&b, &l, 1.0, 1.0, 0.0).expect("floor");
        let top = island_at(&b, &l, 5.0, 5.0, 1.0).expect("the block's top is a floor of its own");
        assert_ne!(top, floor, "cs {cs}: a 1 m ledge became climbable");
        let b = load("ramps");
        let l = islands(&b);
        let floor = island_at(&b, &l, 10.0, 5.0, 0.0).expect("floor");
        let top = island_at(&b, &l, 5.5, 7.5, 1.5).expect("the 50 degree platform is a floor of its own");
        assert_ne!(top, floor, "cs {cs}: the 50 degree ramp became walkable");
        assert_eq!(island_at(&b, &l, 5.5, 2.5, 1.5), Some(floor), "cs {cs}: the 30 degree ramp is not walkable");
    }
}
