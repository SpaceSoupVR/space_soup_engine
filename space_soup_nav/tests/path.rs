//! Paths: valid (every straight leg is walkable) and how close to the true shortest path.
//!
//! The reference is computed independently of the search: the exact shortest path on a flat
//! navmesh bends only at wall corners, so a visibility graph over the corners (two corners joined
//! when the floor raycast between them gets through) plus Dijkstra gives it exactly.

mod common;

use std::collections::BinaryHeap;
use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::{EdgeLink, NavPieceAsset, Turn};
use space_soup_nav::path::{find_path, Path, PathScheduler, Request, Waypoint};
use space_soup_nav::world::{NavWorld, PolyRef};

const EXT: DVec3 = DVec3::new(500.0, 2000.0, 500.0);

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f64 / 16777216.0
    }
}

fn flat(a: DVec3, b: DVec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn length_2d(p: &Path) -> f64 {
    p.waypoints.windows(2).map(|w| flat(w[0].pos(), w[1].pos())).sum()
}

/// A random point on a random polygon.
fn random_point(w: &NavWorld, rng: &mut Lcg) -> DVec3 {
    let n = w.poly_count();
    let mut k = (rng.next() * n as f64) as usize % n;
    for (ii, isl) in w.islands.iter().enumerate() {
        if k < isl.asset.polys.len() {
            let v = w.poly_dverts(PolyRef { island: ii as u32, poly: k as u16 });
            let i = 1 + (rng.next() * (v.len() - 2) as f64) as usize % (v.len() - 2);
            let (mut a, mut b) = (rng.next(), rng.next());
            if a + b > 1.0 {
                a = 1.0 - a;
                b = 1.0 - b;
            }
            return v[0] + (v[i] - v[0]) * a + (v[i + 1] - v[0]) * b;
        }
        k -= isl.asset.polys.len();
    }
    unreachable!()
}

/// Every point of the leg is on the floor (within 1 mm), sampled every 50 mm. Stricter and more
/// robust than a raycast for legs that run exactly along a wall.
fn on_floor(w: &NavWorld, a: DVec3, b: DVec3) -> bool {
    let steps = ((b - a).length() / 50.0).ceil().max(1.0) as usize;
    (0..=steps).all(|k| {
        let p = a.lerp(b, k as f64 / steps as f64);
        w.nearest_poly(p, DVec3::new(20.0, 2000.0, 20.0)).is_some_and(|n| flat(n.point, p) <= 1.0 && (n.point.y - p.y).abs() <= 300.0) || in_welded_gap(w, p)
    })
}

/// Between two doorway planes the level generator welded a few centimetres apart: no floor, by
/// design, and a portal carries the path across.
fn in_welded_gap(w: &NavWorld, p: DVec3) -> bool {
    w.connections.iter().filter(|c| c.gap_mm.abs() > 1).any(|c| {
        let s = w.socket(c.a).unwrap();
        let axis = if matches!(s.outward, space_soup_nav::asset::Dir::PosX | space_soup_nav::asset::Dir::NegX) { 0 } else { 2 };
        let lat = 2 - axis;
        let m = c.midpoint.as_dvec3();
        (p[axis] - m[axis]).abs() <= c.gap_mm.abs() as f64 && (p[lat] - m[lat]).abs() <= 2000.0
    })
}

/// A floor raycast gets from `a` to `b`, and ends on the floor at `b`'s height (a raycast works
/// from above, so it would also "reach" a point on a floor overhead).
fn walkable(w: &NavWorld, a: DVec3, b: DVec3) -> bool {
    let Some(s) = w.nearest_poly(a, EXT) else { return false };
    w.walkable_line(s.poly, s.point, b)
}

/// Wall corners of every polygon, nudged 1 mm into the polygon so they are on the floor.
fn corners(w: &NavWorld) -> Vec<DVec3> {
    let mut out = Vec::new();
    for (ii, isl) in w.islands.iter().enumerate() {
        for (pi, poly) in isl.asset.polys.iter().enumerate() {
            let v = w.poly_dverts(PolyRef { island: ii as u32, poly: pi as u16 });
            let c = v.iter().copied().sum::<DVec3>() / v.len() as f64;
            let n = v.len();
            for (k, &vk) in v.iter().enumerate() {
                let wall_before = !matches!(poly.edges[(k + n - 1) % n], EdgeLink::Internal(_));
                let wall_after = !matches!(poly.edges[k], EdgeLink::Internal(_));
                if wall_before || wall_after {
                    out.push(vk + (c - vk).normalize() * 1.0);
                }
            }
        }
    }
    out
}

/// Exact shortest walk from `a` to `b` on the floor plane, by the corner visibility graph.
fn shortest(w: &NavWorld, pts: &[DVec3], vis: &[Vec<(usize, f64)>], a: DVec3, b: DVec3) -> Option<f64> {
    if visible(w, a, b) {
        return Some(flat(a, b));
    }
    let n = pts.len();
    let mut dist = vec![f64::MAX; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if visible(w, a, pts[i]) {
            dist[i] = flat(a, pts[i]);
            heap.push(std::cmp::Reverse((ordered(dist[i]), i)));
        }
    }
    let to_b: Vec<bool> = (0..n).map(|i| visible(w, pts[i], b)).collect();
    let mut best = f64::MAX;
    while let Some(std::cmp::Reverse((d, i))) = heap.pop() {
        let d = f64::from_bits(d);
        if d > dist[i] || d >= best {
            continue;
        }
        if to_b[i] {
            best = best.min(d + flat(pts[i], b));
        }
        for &(j, l) in &vis[i] {
            if d + l < dist[j] {
                dist[j] = d + l;
                heap.push(std::cmp::Reverse((ordered(dist[j]), j)));
            }
        }
    }
    (best < f64::MAX).then_some(best)
}

fn ordered(x: f64) -> u64 {
    x.to_bits()
}

/// Can one walk straight from `a` to `b`? A raycast, double-checked by sampling when it fails
/// (a ray grazing a wall can fail where the walk is fine).
fn visible(w: &NavWorld, a: DVec3, b: DVec3) -> bool {
    walkable(w, a, b) || on_floor(w, a, b)
}

fn visibility(w: &NavWorld, pts: &[DVec3]) -> Vec<Vec<(usize, f64)>> {
    (0..pts.len()).map(|i| (0..pts.len()).filter(|&j| j != i && visible(w, pts[i], pts[j])).map(|j| (j, flat(pts[i], pts[j]))).collect()).collect()
}

fn single(asset: Arc<NavPieceAsset>) -> NavWorld {
    let mut w = NavWorld::new();
    w.add_island(asset, Turn(0), IVec3::ZERO);
    w
}

#[test]
fn a_straight_hallway_gives_a_straight_path() {
    let kit = common::bake_kit("human_cs10_monotone");
    let names: Vec<String> = common::kit_index().pieces.into_iter().map(|p| p.piece).collect();
    let h = kit[names.iter().position(|n| n == "HallwayShort_DoorType_A").unwrap()].clone();
    let w = single(h);
    let (a, b) = (DVec3::new(0.0, 0.0, -9000.0), DVec3::new(0.0, 0.0, 9000.0));
    let p = find_path(&w, a, b, EXT).unwrap();
    assert!(p.complete);
    assert_eq!(p.waypoints.len(), 2, "{:?}", p.waypoints);
    assert!((p.length() - 18_000.0).abs() < 1.0);
}

#[test]
fn paths_are_walkable_and_near_the_true_shortest() {
    let kit = common::bake_kit("human_cs10_monotone");
    let mut rng = Lcg(7);
    let (mut n, mut total_excess, mut worst) = (0usize, 0.0f64, (0.0f64, String::new()));
    let mut cases: Vec<(String, NavWorld)> = kit
        .iter()
        .zip(common::kit_index().pieces)
        .filter(|(a, _)| !a.polys.is_empty())
        .map(|(a, p)| (p.piece, single(a.clone())))
        .collect();
    for name in ["two_rooms_door", "l_corridor", "narrow_gaps", "doorway_offsets", "ledge"] {
        cases.push((name.to_string(), single(synthetic(name))));
    }
    for (name, w) in &cases {
        let pts = corners(w);
        let vis = visibility(w, &pts);
        for _ in 0..40 {
            let (a, b) = (random_point(w, &mut rng), random_point(w, &mut rng));
            let Some(opt) = shortest(w, &pts, &vis, a, b) else { continue };
            let p = find_path(w, a, b, EXT).unwrap();
            if !p.complete {
                let (sa, sb) = (w.nearest_poly(a, EXT).unwrap(), w.nearest_poly(b, EXT).unwrap());
                let mut w2 = NavWorld::new();
                w2.add_island(w.islands[0].asset.clone(), Turn(0), IVec3::ZERO);
                panic!("{name}: incomplete path; reference length {opt:.0} from {a:?} (poly {:?}) to {b:?} (poly {:?}); same component: {}; straight walk on floor: {}",
                    sa.poly, sb.poly, w2.same_component(sa.poly, sb.poly), on_floor(w, a, b));
            }
            for leg in p.waypoints.windows(2) {
                assert!(on_floor(w, leg[0].pos(), leg[1].pos()), "{name}: a leg leaves the floor: {leg:?}");
            }
            let ours = length_2d(&p);
            assert!(ours >= opt - 5.0, "{name}: shorter than the shortest ({ours} < {opt}): the reference is wrong");
            let excess = ours / opt.max(1.0) - 1.0;
            total_excess += excess;
            n += 1;
            if excess > worst.0 {
                worst = (excess, format!("{name}: {ours:.0} mm vs {opt:.0} mm"));
            }
        }
    }
    let mean = total_excess / n as f64;
    println!("{n} paths: mean {:.3}% longer than the shortest, worst {:.2}% ({})", mean * 100.0, worst.0 * 100.0, worst.1);
    assert!(mean < 0.01, "paths average {:.2}% longer than the shortest", mean * 100.0);
}

fn synthetic(name: &str) -> Arc<NavPieceAsset> {
    use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput};
    let mesh = space_soup_nav::obj::TriMesh::load(&space_soup_nav::golden::test_data_dir().join(format!("meshes/synthetic/{name}.obj"))).unwrap();
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.to_array()).collect();
    let input = PieceInput { name, vertices: &verts, triangles: &mesh.triangles, sockets: &[], links: Vec::new() };
    Arc::new(bake_piece(&input, &common::settings(&common::config("human_cs10_monotone")), [1.0; 3], &ExportOptions::default()).unwrap().0)
}

#[test]
fn across_a_whole_level_through_doorways_and_doors() {
    let kit = common::bake_kit("human_cs10_monotone");
    let layout = &common::layouts()[0];
    let mut w = common::assemble(&kit, layout, 0);
    let mut rng = Lcg(11);
    let mut lengths = Vec::new();
    let mut expanded = Vec::new();
    let mut search = std::time::Duration::ZERO;
    let mut locate = std::time::Duration::ZERO;
    let mut q = space_soup_nav::path::PathQuery::new(&w);
    for _ in 0..200 {
        let (a, b) = (random_point(&w, &mut rng), random_point(&w, &mut rng));
        let t0 = std::time::Instant::now();
        let (Some(na), Some(nb)) = (w.nearest_poly(a, EXT), w.nearest_poly(b, EXT)) else { continue };
        locate += t0.elapsed();
        let t1 = std::time::Instant::now();
        q.begin(&w, na.poly, na.point, nb.poly, nb.point);
        while q.step(&w, usize::MAX) == space_soup_nav::path::Status::InProgress {}
        let p = q.finish(&w).unwrap();
        search += t1.elapsed();
        expanded.push(q.expanded);
        let (sa, sb) = (w.nearest_poly(a, EXT).unwrap().poly, w.nearest_poly(b, EXT).unwrap().poly);
        assert_eq!(p.complete, w.same_component(sa, sb), "complete exactly when the goal is reachable");
        for leg in p.waypoints.windows(2) {
            if matches!(leg[1], Waypoint::Walk(_)) {
                assert!(on_floor(&w, leg[0].pos(), leg[1].pos()), "a leg leaves the floor: {leg:?}");
            }
        }
        lengths.push(p.length());
    }
    expanded.sort();
    let n = lengths.len() as f64;
    println!(
        "{} paths across seed {}: locate {:.3} ms + search {:.3} ms each (Mac, release), median {} / max {} polygons expanded, longest {:.0} m",
        lengths.len(), layout.seed, locate.as_secs_f64() * 1000.0 / n, search.as_secs_f64() * 1000.0 / n, expanded[expanded.len() / 2], expanded.last().unwrap(),
        lengths.iter().cloned().fold(0.0, f64::max) / 1000.0
    );

    let (a, b) = (random_point(&w, &mut rng), random_point(&w, &mut rng));
    let p = find_path(&w, a, b, EXT).unwrap();
    let through_door = p.links().find(|&l| w.links[l as usize].gate.is_some());
    if let Some(l) = through_door {
        let gate = w.links[l as usize].gate.unwrap();
        w.set_gate(gate, false);
        let q = find_path(&w, a, b, EXT).unwrap();
        assert!(q.links().all(|l| w.links[l as usize].gate != Some(gate)), "a path went through a closed door");
        assert!(!q.complete || q.length() >= p.length() - 1.0);
    }
}

#[test]
fn the_scheduler_keeps_to_its_budget_and_finishes_everything() {
    let kit = common::bake_kit("human_cs10_monotone");
    let w = common::assemble(&kit, &common::layouts()[0], 0);
    let mut rng = Lcg(3);
    let mut s = PathScheduler::new(&w);
    for id in 0..30 {
        s.request(Request { id, start: random_point(&w, &mut rng), goal: random_point(&w, &mut rng) });
    }
    let mut finished = Vec::new();
    let mut frames = 0;
    while s.pending() > 0 {
        finished.extend(s.update(&w, 200));
        frames += 1;
        assert!(frames < 10_000);
    }
    let ids: Vec<u64> = finished.iter().map(|f| f.0).collect();
    assert_eq!(ids, (0..30).collect::<Vec<_>>(), "first come, first served, none lost");
    println!("30 requests at 200 expansions a frame: {frames} frames");
}

#[test]
fn the_room_graph_measures_real_walking_distances() {
    let kit = common::bake_kit("human_cs10_monotone");
    let names: Vec<String> = common::kit_index().pieces.into_iter().map(|p| p.piece).collect();
    let layout = &common::layouts()[0];
    let mut w = common::assemble(&kit, layout, 0);
    let g = w.room_graph();

    let h = layout.pieces.iter().position(|p| names[p.kind] == "HallwayShort_DoorType_A").unwrap();
    let across = g.crossing[h][0][1].unwrap();
    assert!((20_400..=20_600).contains(&across), "hallway crossing {across} mm");

    let mut rng = Lcg(5);
    let (mut n, mut worst) = (0, 0.0f64);
    let passages: Vec<u32> = (0..w.connections.len() as u32).filter(|&c| !w.connections[c as usize].links.is_empty()).collect();
    for _ in 0..60 {
        let c1 = passages[(rng.next() * passages.len() as f64) as usize % passages.len()];
        let c2 = passages[(rng.next() * passages.len() as f64) as usize % passages.len()];
        let (m1, m2) = (w.connections[c1 as usize].midpoint.as_dvec3(), w.connections[c2 as usize].midpoint.as_dvec3());
        let d = g.from_doorway(&w, c1)[c2 as usize];
        let real = space_soup_nav::path::find_path_guided(&w, &g, m1, m2, EXT).filter(|p| p.complete);
        assert_eq!(d < u64::MAX, real.is_some(), "graph and polygons disagree about reaching {c2} from {c1}");
        if let Some(p) = real {
            let ratio = d as f64 / p.length().max(1.0);
            assert!(ratio >= 0.99, "graph distance {d} shorter than the real path {}", p.length());
            worst = f64::max(worst, ratio);
            n += 1;
        }
    }
    println!("{n} doorway pairs: room graph at most {:.1}% longer than the real path", (worst - 1.0) * 100.0);
    assert!(worst < 1.25);

    let target = layout.pieces.len() as u32 - 1;
    let field = g.field_to(&w, target);
    for room in 0..layout.pieces.len() as u32 {
        if let Some(c) = g.toward(&w, &field, room) {
            let conn = &w.connections[c as usize];
            assert!(conn.a.island == room || conn.b.island == room);
        }
    }
    let first = g.toward(&w, &field, 0);
    if let Some(c) = first {
        if let Some(gate) = w.connections[c as usize].gate {
            w.set_gate(gate, false);
            let field2 = g.field_to(&w, target);
            assert_ne!(g.toward(&w, &field2, 0), Some(c), "a closed door is not the way");
        }
    }
}

#[test]
fn guided_search_against_plain_search_across_levels() {
    let kit = common::bake_kit("human_cs10_monotone");
    let (mut n, mut shorter, mut longer, mut best_gain, mut worst_loss) = (0, 0, 0, 0.0f64, 0.0f64);
    let (mut t_plain, mut t_guided) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    for layout in common::layouts().iter().take(5) {
        let w = common::assemble(&kit, layout, 0);
        let g = w.room_graph();
        let mut rng = Lcg(layout.seed);
        for _ in 0..60 {
            let (a, b) = (random_point(&w, &mut rng), random_point(&w, &mut rng));
            let t0 = std::time::Instant::now();
            let plain = find_path(&w, a, b, EXT);
            t_plain += t0.elapsed();
            let t1 = std::time::Instant::now();
            let guided = space_soup_nav::path::find_path_guided(&w, &g, a, b, EXT);
            t_guided += t1.elapsed();
            let (Some(p), Some(q)) = (plain, guided) else { continue };
            assert_eq!(p.complete, q.complete);
            if !p.complete {
                continue;
            }
            for leg in q.waypoints.windows(2) {
                assert!(on_floor(&w, leg[0].pos(), leg[1].pos()), "a guided leg leaves the floor: {leg:?}");
            }
            let gain = p.length() / q.length() - 1.0;
            if gain > 1e-6 {
                shorter += 1;
                best_gain = best_gain.max(gain);
            } else if gain < -1e-6 {
                longer += 1;
                worst_loss = worst_loss.max(-gain);
            }
            n += 1;
        }
    }
    println!(
        "{n} paths in 5 levels: guided shorter in {shorter} (by up to {:.1}%), longer in {longer} (by up to {:.2}%); {:.3} ms plain vs {:.3} ms guided each",
        best_gain * 100.0, worst_loss * 100.0, t_plain.as_secs_f64() * 1000.0 / n as f64, t_guided.as_secs_f64() * 1000.0 / n as f64
    );
    assert!(worst_loss < 0.01, "guided search lost {:.2}% somewhere", worst_loss * 100.0);
}
