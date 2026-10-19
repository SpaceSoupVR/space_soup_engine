//! Phase 6: what the game must survive. Damaged assets, odd geometry, points nowhere near the
//! floor, joins that should not be made. The promise each test holds: an error is fine, a panic
//! is not, and whatever is accepted is safe to use.

mod common;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use glam::{DVec3, IVec3};
use space_soup_nav::asset::{NavPieceAsset, Turn};
use space_soup_nav::path::find_path;
use space_soup_nav::validate::check_piece;
use space_soup_nav::world::NavWorld;

/// xorshift64*: the tests' own generator, so a failing case is reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Replaces the trailing hash so the damage reaches the parser instead of stopping at the hash.
fn rehash(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() >= 8 {
        let n = bytes.len() - 8;
        let h = fnv1a64(&bytes[..n]);
        bytes[n..].copy_from_slice(&h.to_le_bytes());
    }
    bytes
}

fn mutate(rng: &mut Rng, bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    for _ in 0..1 + rng.below(4) {
        let body = b.len().saturating_sub(8);
        if body == 0 {
            break;
        }
        let at = rng.below(body);
        match rng.below(6) {
            0 => b[at] ^= 1 << rng.below(8),
            1 => b[at] = rng.next() as u8,
            2 => {
                let v = [0u32, 1, 0xffff, 0x7fff_ffff, 0x8000_0000, u32::MAX, rng.next() as u32][rng.below(7)];
                for (k, x) in v.to_le_bytes().iter().enumerate() {
                    if at + k < body {
                        b[at + k] = *x;
                    }
                }
            }
            3 => {
                b.remove(at);
            }
            4 => b.insert(at, rng.next() as u8),
            _ => b.truncate(at + 8),
        }
    }
    rehash(b)
}

/// Uses an accepted asset the way the game would. Must not panic.
fn exercise(asset: Arc<NavPieceAsset>, rng: &mut Rng) {
    let _ = check_piece(&asset);
    let mut w = NavWorld::new();
    w.add_island(asset.clone(), Turn(rng.below(4) as u8), IVec3::new(5000, 0, -3000));
    w.add_island(asset, Turn(0), IVec3::ZERO);
    let (lo, hi) = w.islands.iter().fold((IVec3::MAX, IVec3::MIN), |(a, b), i| (a.min(i.min), b.max(i.max)));
    let (lo, hi) = (lo.as_dvec3() - 2000.0, hi.as_dvec3() + 2000.0);
    let pick = |rng: &mut Rng| DVec3::new(rng.range(lo.x, hi.x), rng.range(lo.y, hi.y), rng.range(lo.z, hi.z));
    let ext = DVec3::new(2000.0, 2000.0, 2000.0);
    for _ in 0..8 {
        let (a, b) = (pick(rng), pick(rng));
        let _ = find_path(&w, a, b, ext);
        if let Some(n) = w.nearest_poly(a, ext) {
            let _ = w.raycast(n.poly, n.point, b);
            let _ = w.move_along_surface(n.poly, n.point, b);
            let _ = w.component(n.poly);
        }
    }
    let _ = w.check_joins();
    let _ = w.room_graph();
}

#[test]
fn damaged_assets_are_refused_or_safe() {
    let kit = common::bake_kit("human_cs10_monotone");
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut accepted, mut refused) = (0, 0);
    let mut failures = Vec::new();
    for asset in kit.iter().filter(|a| !a.polys.is_empty()) {
        let bytes = asset.to_bytes();
        for case in 0..400 {
            let damaged = mutate(&mut rng, &bytes);
            let seed = rng.next() | 1;
            let outcome = catch_unwind(AssertUnwindSafe(|| match NavPieceAsset::from_bytes(&damaged) {
                Ok(a) => {
                    exercise(Arc::new(a), &mut Rng(seed));
                    true
                }
                Err(_) => false,
            }));
            match outcome {
                Ok(true) => accepted += 1,
                Ok(false) => refused += 1,
                Err(_) => failures.push(format!("{} case {case}", asset.name)),
            }
        }
        let mut flipped = bytes.clone();
        flipped[bytes.len() / 2] ^= 0x10;
        assert!(NavPieceAsset::from_bytes(&flipped).is_err());
    }
    eprintln!("damaged assets: {accepted} accepted and used, {refused} refused, {} panicked", failures.len());
    assert!(failures.is_empty(), "panicked on: {}", failures.join(", "));
}

/// An axis-aligned box as 12 triangles facing out.
fn add_box(v: &mut Vec<[f32; 3]>, t: &mut Vec<[u32; 3]>, lo: [f32; 3], hi: [f32; 3]) {
    let b = v.len() as u32;
    for k in 0..8 {
        v.push([if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }]);
    }
    for q in [[0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5], [0, 2, 3, 1], [4, 5, 7, 6]] {
        t.push([b + q[0], b + q[1], b + q[2]]);
        t.push([b + q[0], b + q[2], b + q[3]]);
    }
}

/// A random room-ish mesh: a floor, slabs and walls at random heights, and loose triangles, some
/// degenerate.
fn random_mesh(rng: &mut Rng) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    let size = rng.range(1.0, 8.0) as f32;
    add_box(&mut v, &mut t, [-size, -0.2, -size], [size, 0.0, size]);
    for _ in 0..rng.below(8) {
        let (x, z, y) = (rng.range(-8.0, 8.0) as f32, rng.range(-8.0, 8.0) as f32, rng.range(-1.0, 4.0) as f32);
        let (w, d, h) = (rng.range(0.05, 4.0) as f32, rng.range(0.05, 4.0) as f32, rng.range(0.01, 3.0) as f32);
        add_box(&mut v, &mut t, [x, y, z], [x + w, y + h, z + d]);
    }
    for _ in 0..rng.below(20) {
        let b = v.len() as u32;
        let p = [rng.range(-8.0, 8.0) as f32, rng.range(-1.0, 4.0) as f32, rng.range(-8.0, 8.0) as f32];
        for _ in 0..3 {
            let s = [0.0, 1e-6, 0.01, 2.0][rng.below(4)];
            v.push([p[0] + rng.range(-s, s) as f32, p[1] + rng.range(-s, s) as f32, p[2] + rng.range(-s, s) as f32]);
        }
        t.push(if rng.below(2) == 0 { [b, b + 1, b + 2] } else { [b, b + 2, b + 1] });
    }
    (v, t)
}

fn random_sockets(rng: &mut Rng) -> Vec<space_soup_nav::bake::export::SocketSpec> {
    use space_soup_nav::asset::Dir;
    (0..rng.below(4))
        .map(|i| space_soup_nav::bake::export::SocketSpec {
            name: format!("S{i}"),
            position: [rng.range(-9.0, 9.0) as f32, rng.range(-0.5, 1.0) as f32, rng.range(-9.0, 9.0) as f32],
            outward: [Dir::PosX, Dir::NegX, Dir::PosZ, Dir::NegZ][rng.below(4)],
            size_class: "A".into(),
        })
        .collect()
}

#[test]
fn the_baker_survives_random_and_broken_meshes() {
    use space_soup_nav::bake::export::{bake_piece, ExportOptions, PieceInput};
    let mut failures = Vec::new();
    let (mut baked, mut refused) = (0, 0);
    for config in ["human_cs20_layers", "human_cs10_monotone"] {
        let s = common::settings(&common::config(config));
        let mut rng = Rng(0x5eed_0000 + config.len() as u64);
        for case in 0..if config == "human_cs20_layers" { 300 } else { 100 } {
            let (mut v, mut t) = random_mesh(&mut rng);
            match case % 10 {
                7 => {
                    let (i, k) = (rng.below(v.len()), rng.below(3));
                    v[i][k] = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30][rng.below(4)];
                }
                8 => t.push([0, 1, v.len() as u32 + 5]),
                9 if case % 20 == 9 => t.clear(),
                _ => {}
            }
            let sockets = random_sockets(&mut rng);
            if std::env::var_os("FUZZ_TRACE").is_some() {
                eprintln!("{config} case {case}: {} verts {} tris", v.len(), t.len());
            }
            let input = PieceInput { name: "fuzz", vertices: &v, triangles: &t, sockets: &sockets, links: Vec::new() };
            let r = catch_unwind(AssertUnwindSafe(|| {
                let a = bake_piece(&input, &s, [1.0; 3], &ExportOptions::default()).map(|x| x.0);
                if let Ok(a) = &a {
                    a.validate().unwrap();
                    assert_eq!(&NavPieceAsset::from_bytes(&a.to_bytes()).unwrap(), a);
                    let again = bake_piece(&input, &s, [1.0; 3], &ExportOptions::default()).unwrap().0;
                    assert_eq!(again.content_hash(), a.content_hash(), "not deterministic");
                }
                a.is_ok()
            }));
            match r {
                Ok(true) => baked += 1,
                Ok(false) => refused += 1,
                Err(_) => failures.push(format!("{config} case {case}")),
            }
        }
    }
    eprintln!("random meshes: {baked} baked, {refused} refused, {} panicked", failures.len());
    assert!(failures.is_empty(), "panicked on: {}", failures.join(", "));
}

#[test]
fn random_queries_on_real_levels_agree_with_the_world() {
    use space_soup_nav::path::find_path_guided;
    let mut checked = 0;
    for class in ["human_cs10_monotone", "large_cs10_layers"] {
        let kit = common::bake_kit(class);
        for layout in common::layouts().iter().take(4) {
            let mut w = common::assemble(&kit, layout, 0);
            let graph = w.room_graph();
            let mut rng = Rng(layout.seed as u64 * 7919 + 1);
            for g in 0..w.gates.len() as u32 {
                if rng.below(4) == 0 {
                    w.set_gate(g, false);
                }
            }
            let (lo, hi) = w.islands.iter().fold((IVec3::MAX, IVec3::MIN), |(a, b), i| (a.min(i.min), b.max(i.max)));
            let (lo, hi) = (lo.as_dvec3() - 3000.0, hi.as_dvec3() + 3000.0);
            let ext = DVec3::new(1500.0, 2500.0, 1500.0);
            for _ in 0..150 {
                let (a, b) = (pick(&w, &mut rng, lo, hi), pick(&w, &mut rng, lo, hi));
                let p = find_path(&w, a, b, ext);
                assert_eq!(p, find_path(&w, a, b, ext), "the same query twice differs");
                let (Some(na), Some(nb)) = (w.nearest_poly(a, ext), w.nearest_poly(b, ext)) else {
                    assert!(p.is_none(), "a path between points off the mesh");
                    continue;
                };
                let p = p.expect("both ends are on the mesh");
                checked += 1;
                assert_eq!(p.complete, w.same_component(na.poly, nb.poly), "{class} seed {}: {a:?} -> {b:?}", layout.seed);
                assert_eq!(p.corridor.first().unwrap().0, na.poly);
                assert!((p.waypoints[0].pos() - na.point).length() < 1.0);
                if p.complete {
                    assert_eq!(p.corridor.last().unwrap().0, nb.poly);
                    assert!((p.waypoints.last().unwrap().pos() - nb.point).length() < 1.0);
                }
                for c in p.corridor.windows(2) {
                    assert!(w.neighbours(c[0].0).any(|(n, _)| n == c[1].0), "{class} seed {}: corridor jumps", layout.seed);
                }
                let g = find_path_guided(&w, &graph, a, b, ext).unwrap();
                assert_eq!(g.complete, p.complete);
                assert!(!p.complete || g.length() <= p.length() * 1.001 + 1.0, "guided path longer: {} vs {}", g.length(), p.length());
            }
        }
    }
    eprintln!("random level queries: {checked} paths checked");
}

#[test]
fn bad_joins_are_refused_and_good_ones_hold() {
    use space_soup_nav::world::SocketRef;
    let kit = common::bake_kit("human_cs10_monotone");
    let mut rng = Rng(0xbad_1015);
    let (mut joined, mut refused) = (0, 0);
    for layout in common::layouts().iter().take(6) {
        let mut w = NavWorld::new();
        for p in &layout.pieces {
            w.add_island(kit[p.kind].clone(), Turn(p.quarter_turns), common::mm(p.position));
        }
        let mut good: Vec<(SocketRef, SocketRef)> = Vec::new();
        for (i, p) in layout.pieces.iter().enumerate() {
            for (s, l) in p.links.iter().enumerate() {
                if let Some([j, t]) = *l {
                    if (j, t) > (i, s) {
                        good.push((SocketRef { island: i as u32, socket: s as u16 }, SocketRef { island: j as u32, socket: t as u16 }));
                    }
                }
            }
        }
        let is_good = |a: SocketRef, b: SocketRef| good.iter().any(|&(x, y)| (x, y) == (a, b) || (y, x) == (a, b));
        let n = layout.pieces.len();
        let mut todo = good.clone();
        while !todo.is_empty() {
            for _ in 0..rng.below(6) {
                let s = |rng: &mut Rng| SocketRef { island: rng.below(n + 1) as u32, socket: rng.below(6) as u16 };
                let (a, b) = if rng.below(8) == 0 { let a = s(&mut rng); (a, a) } else { (s(&mut rng), s(&mut rng)) };
                if is_good(a, b) {
                    continue;
                }
                let r = catch_unwind(AssertUnwindSafe(|| w.connect(a, b, Some("door"))));
                match r {
                    Ok(Err(_)) => refused += 1,
                    Ok(Ok(_)) => panic!("seed {}: joined {a:?} and {b:?}, which the level does not join", layout.seed),
                    Err(_) => panic!("seed {}: connect {a:?} {b:?} panicked", layout.seed),
                }
            }
            let (a, b) = todo.swap_remove(rng.below(todo.len()));
            w.connect(a, b, None).unwrap_or_else(|e| panic!("seed {}: real join {a:?} {b:?} refused: {e:?}", layout.seed));
            joined += 1;
            assert!(w.connect(a, b, None).is_err() && w.connect(b, a, None).is_err());
        }
        assert!(w.check_joins().iter().all(|j| !matches!(j, space_soup_nav::world::JoinIssue::MissedJoin(..))));
        exercise_world(&mut w, &mut rng);
    }
    eprintln!("joins: {joined} real ones made, {refused} bad ones refused");
}

/// A point to query from: mostly on some floor (a little above it, as a player's feet are), some
/// anywhere in the level's box.
fn pick(w: &NavWorld, rng: &mut Rng, lo: DVec3, hi: DVec3) -> DVec3 {
    if rng.below(5) > 0 && w.poly_count() > 0 {
        let isl = rng.below(w.islands.len());
        let n = w.islands[isl].asset.polys.len();
        if n > 0 {
            let tris = w.floor_triangles(space_soup_nav::world::PolyRef { island: isl as u32, poly: rng.below(n) as u16 });
            let t = tris[rng.below(tris.len())];
            let (mut u, mut v) = (rng.range(0.0, 1.0), rng.range(0.0, 1.0));
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            return t[0] + (t[1] - t[0]) * u + (t[2] - t[0]) * v + DVec3::new(0.0, rng.range(0.0, 400.0), 0.0);
        }
    }
    DVec3::new(rng.range(lo.x, hi.x), rng.range(lo.y, hi.y), rng.range(lo.z, hi.z))
}

fn exercise_world(w: &mut NavWorld, rng: &mut Rng) {
    if w.islands.is_empty() {
        return;
    }
    let (lo, hi) = w.islands.iter().fold((IVec3::MAX, IVec3::MIN), |(a, b), i| (a.min(i.min), b.max(i.max)));
    let ext = DVec3::new(1500.0, 2500.0, 1500.0);
    for _ in 0..20 {
        let mut pick = || DVec3::new(rng.range(lo.x as f64, hi.x as f64), rng.range(lo.y as f64, hi.y as f64), rng.range(lo.z as f64, hi.z as f64));
        let (a, b) = (pick(), pick());
        if let (Some(p), Some(na), Some(nb)) = (find_path(w, a, b, ext), w.nearest_poly(a, ext), w.nearest_poly(b, ext)) {
            assert_eq!(p.complete, w.same_component(na.poly, nb.poly));
        }
    }
}
