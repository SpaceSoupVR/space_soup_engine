//! WHAT IS NEAR A HAND, without asking every object in the level.
//!
//! A grab used to measure the hand against every object the server sends --
//! room shells, lamps and markers included -- on each press. This keeps the
//! objects' boxes in a grid of cells: an object is filed under each cell its
//! box (as it stands in the world) touches, and a hand asks only the cells
//! within its reach. Told of an object that has not moved, the grid does
//! nothing; told of one that moved within its cells, it rewrites one entry.
//! So a frame's update costs a comparison an object and no allocation, and a
//! press costs what is near the hand.
//!
//! A box too big to file by cell (a room's shell, a sea's bounds) is kept in a
//! short list every query reads: such things are few, and they are not what a
//! hand picks up.

use std::collections::HashMap;

use glam::{Quat, Vec3};

use crate::scene_cuboid::distance_to_oriented_box;

/// A cell's side, metres: a few times a hand's reach, and about the size of
/// what a hand carries.
pub const CELL: f32 = 0.5;
/// A box filed under more cells than this is kept in the list of big things.
pub const MAX_CELLS: usize = 64;

type Cell = (i32, i32, i32);

#[derive(Debug, Clone)]
struct Entry {
    id: String,
    position: Vec3,
    rotation: Quat,
    half: Vec3,
    /// The cells it is filed under, lowest and highest; none for a big thing.
    cells: Option<(Cell, Cell)>,
    /// The update it was last told of in (`GrabGrid::begin`).
    seen: u64,
}

#[derive(Debug, Default, Clone)]
pub struct GrabGrid {
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    cells: HashMap<Cell, Vec<usize>>,
    big: Vec<usize>,
    round: u64,
    /// How many entries were filed again since the grid was made: what a
    /// test watches to see that a still object costs nothing.
    refiled: u64,
}

fn cell_of(p: Vec3) -> Cell {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32)
}

fn span(position: Vec3, rotation: Quat, half: Vec3) -> Option<(Cell, Cell)> {
    let extent = crate::water::world_half_extents(half, rotation);
    if !(position.is_finite() && extent.is_finite()) {
        return None;
    }
    let (lo, hi) = (cell_of(position - extent), cell_of(position + extent));
    let count = i64::from(hi.0 - lo.0 + 1) * i64::from(hi.1 - lo.1 + 1) * i64::from(hi.2 - lo.2 + 1);
    (count <= MAX_CELLS as i64).then_some((lo, hi))
}

fn each_cell((lo, hi): (Cell, Cell), mut f: impl FnMut(Cell)) {
    for x in lo.0..=hi.0 {
        for y in lo.1..=hi.1 {
            for z in lo.2..=hi.2 {
                f((x, y, z));
            }
        }
    }
}

impl GrabGrid {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many times an entry has been filed under cells: see `refiled`.
    #[must_use]
    pub fn refiled(&self) -> u64 {
        self.refiled
    }

    /// Start an update: everything told of with [`Self::set`] before
    /// [`Self::end`] is kept, the rest forgotten.
    pub fn begin(&mut self) {
        self.round += 1;
    }

    /// An object as it is now. Nothing is done for one that has not moved.
    pub fn set(&mut self, id: &str, position: Vec3, rotation: Quat, half: Vec3) {
        let round = self.round;
        if let Some(&i) = self.by_id.get(id) {
            let e = &mut self.entries[i];
            e.seen = round;
            if e.position == position && e.rotation == rotation && e.half == half {
                return;
            }
            let cells = span(position, rotation, half);
            (e.position, e.rotation, e.half) = (position, rotation, half);
            if cells != e.cells {
                let was = std::mem::replace(&mut e.cells, cells);
                self.unfile(i, was);
                self.file(i, cells);
            }
            return;
        }
        let cells = span(position, rotation, half);
        let i = self.entries.len();
        self.entries.push(Entry { id: id.to_string(), position, rotation, half, cells, seen: round });
        self.by_id.insert(id.to_string(), i);
        self.file(i, cells);
    }

    /// End an update: forget whatever was not told of since [`Self::begin`].
    /// When nothing has gone -- every frame but the one a thing is despawned
    /// in -- this is one comparison an object.
    pub fn end(&mut self) {
        let round = self.round;
        if self.entries.iter().all(|e| e.seen == round) {
            return;
        }
        let kept: Vec<Entry> = std::mem::take(&mut self.entries).into_iter().filter(|e| e.seen == round).collect();
        self.by_id.clear();
        self.cells.clear();
        self.big.clear();
        for e in kept {
            let (i, cells) = (self.entries.len(), e.cells);
            self.by_id.insert(e.id.clone(), i);
            self.entries.push(e);
            self.file(i, cells);
        }
    }

    fn file(&mut self, i: usize, cells: Option<(Cell, Cell)>) {
        self.refiled += 1;
        match cells {
            Some(range) => each_cell(range, |c| self.cells.entry(c).or_default().push(i)),
            None => self.big.push(i),
        }
    }

    fn unfile(&mut self, i: usize, cells: Option<(Cell, Cell)>) {
        match cells {
            Some(range) => each_cell(range, |c| {
                if let Some(list) = self.cells.get_mut(&c) {
                    list.retain(|&k| k != i);
                    if list.is_empty() {
                        self.cells.remove(&c);
                    }
                }
            }),
            None => self.big.retain(|&k| k != i),
        }
    }

    /// THE NEAREST OBJECT `allowed` LETS A HAND TAKE whose box is within
    /// `reach` of `point`: its id and how far its surface is (zero inside
    /// it). Only the cells within reach are read, and the big things.
    pub fn nearest(&self, point: Vec3, reach: f32, allowed: impl Fn(&str) -> bool) -> Option<(&str, f32)> {
        let mut best: Option<(usize, f32)> = None;
        let mut consider = |i: usize| {
            let e = &self.entries[i];
            if best.is_some_and(|(b, _)| b == i) || !allowed(&e.id) {
                return;
            }
            let d = distance_to_oriented_box(e.position, e.rotation, e.half, point);
            let better = match best {
                None => true,
                Some((b, bd)) => d < bd || (d == bd && e.id < self.entries[b].id),
            };
            if d <= reach && better {
                best = Some((i, d));
            }
        };
        each_cell((cell_of(point - Vec3::splat(reach)), cell_of(point + Vec3::splat(reach))), |c| {
            if let Some(list) = self.cells.get(&c) {
                for &i in list {
                consider(i);
            }
            }
        });
        self.big.iter().for_each(|&i| consider(i));
        best.map(|(i, d)| (self.entries[i].id.as_str(), d))
    }

    /// EVERY OBJECT WHOSE BOX IS WITHIN `reach` OF `point`, once each: its
    /// id, pose, half size and how far its surface is (zero inside it). The
    /// same cells [`Self::nearest`] reads, and the big things; for a hand's
    /// targets (`grab_target`), which weigh more than the distance.
    pub fn each_within(&self, point: Vec3, reach: f32, mut f: impl FnMut(&str, Vec3, Quat, Vec3, f32)) {
        let mut seen: Vec<usize> = Vec::new();
        let mut consider = |i: usize| {
            if seen.contains(&i) {
                return;
            }
            seen.push(i);
            let e = &self.entries[i];
            let d = distance_to_oriented_box(e.position, e.rotation, e.half, point);
            if d <= reach {
                f(&e.id, e.position, e.rotation, e.half, d);
            }
        };
        each_cell((cell_of(point - Vec3::splat(reach)), cell_of(point + Vec3::splat(reach))), |c| {
            if let Some(list) = self.cells.get(&c) {
                for &i in list {
                consider(i);
            }
            }
        });
        self.big.iter().for_each(|&i| consider(i));
    }

    /// How many entries a query at `point` with `reach` reads: what it costs.
    #[must_use]
    pub fn read_by(&self, point: Vec3, reach: f32) -> usize {
        let mut n = self.big.len();
        each_cell((cell_of(point - Vec3::splat(reach)), cell_of(point + Vec3::splat(reach))), |c| {
            n += self.cells.get(&c).map_or(0, Vec::len);
        });
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cheap deterministic scatter.
    fn mix(k: u32) -> f32 {
        let mut x = k.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2c1b_3c6d);
        x ^= x >> 12;
        (x & 0xffff) as f32 / 65535.0
    }

    fn level(count: u32) -> Vec<(String, Vec3, Quat, Vec3)> {
        let mut out: Vec<(String, Vec3, Quat, Vec3)> = (0..count)
            .map(|i| {
                let p = Vec3::new(mix(i * 3) * 200.0 - 100.0, mix(i * 3 + 1) * 3.0, mix(i * 3 + 2) * 200.0 - 100.0);
                let half = Vec3::new(0.02 + 0.2 * mix(i + 9000), 0.02 + 0.2 * mix(i + 9100), 0.02 + 0.2 * mix(i + 9200));
                (format!("thing_{i}"), p, Quat::from_rotation_y(mix(i + 7000) * 6.0) * Quat::from_rotation_x(mix(i + 7100)), half)
            })
            .collect();
        out.push(("hall_shell".into(), Vec3::new(0.0, 1.7, -6.0), Quat::IDENTITY, Vec3::new(3.0, 1.7, 10.0)));
        out.push(("sea".into(), Vec3::ZERO, Quat::IDENTITY, Vec3::new(80.0, 0.5, 80.0)));
        out
    }

    fn filled(objects: &[(String, Vec3, Quat, Vec3)]) -> GrabGrid {
        let mut g = GrabGrid::default();
        g.begin();
        for (id, p, r, h) in objects {
            g.set(id, *p, *r, *h);
        }
        g.end();
        g
    }

    fn brute(objects: &[(String, Vec3, Quat, Vec3)], point: Vec3, reach: f32, allowed: impl Fn(&str) -> bool) -> Option<(&str, f32)> {
        objects
            .iter()
            .filter(|o| allowed(&o.0))
            .map(|o| (o.0.as_str(), distance_to_oriented_box(o.1, o.2, o.3, point)))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)))
    }

    /// The grid's answer is the answer of asking every object, for hands
    /// beside things, between them and far from all of them.
    #[test]
    fn the_grid_finds_what_asking_every_object_finds() {
        let objects = level(4000);
        let g = filled(&objects);
        let loose = |id: &str| id.starts_with("thing_");
        let mut found = 0;
        for k in 0..3000u32 {
            let point = if k % 2 == 0 {
                let o = &objects[(k as usize * 7) % 4000];
                o.1 + Vec3::new(mix(k + 1) - 0.5, mix(k + 2) - 0.5, mix(k + 3) - 0.5) * 0.5
            } else {
                Vec3::new(mix(k * 5) * 200.0 - 100.0, mix(k * 5 + 1) * 3.0, mix(k * 5 + 2) * 200.0 - 100.0)
            };
            let (a, b) = (g.nearest(point, 0.15, loose), brute(&objects, point, 0.15, loose));
            assert_eq!(a.map(|x| x.0), b.map(|x| x.0), "at {point}");
            found += usize::from(a.is_some());
            assert_eq!(g.nearest(point, 0.15, |_| true).map(|x| x.0), brute(&objects, point, 0.15, |_| true).map(|x| x.0));
        }
        assert!(found > 500, "the test's hands found only {found} things");
    }

    /// `each_within` names every object asking every object names, once.
    #[test]
    fn each_within_finds_what_asking_every_object_finds_once() {
        let objects = level(4000);
        let g = filled(&objects);
        for k in 0..1500u32 {
            let o = &objects[(k as usize * 11) % 4000];
            let point = o.1 + Vec3::new(mix(k + 5) - 0.5, mix(k + 6) - 0.5, mix(k + 7) - 0.5) * 0.8;
            let mut got: Vec<String> = Vec::new();
            g.each_within(point, 0.6, |id, _, _, _, d| {
                assert!(d <= 0.6);
                got.push(id.to_string());
            });
            let n = got.len();
            got.sort();
            got.dedup();
            assert_eq!(got.len(), n, "an object was named twice at {point}");
            let mut want: Vec<String> = objects
                .iter()
                .filter(|o| distance_to_oriented_box(o.1, o.2, o.3, point) <= 0.6)
                .map(|o| o.0.clone())
                .collect();
            want.sort();
            assert_eq!(got, want, "at {point}");
        }
    }

    /// A press reads what is near the hand, not the level: with four
    /// thousand things across a 200 m level, a handful and the big ones.
    #[test]
    fn a_query_reads_what_is_near_the_hand() {
        let objects = level(4000);
        let g = filled(&objects);
        let worst = (0..500u32).map(|k| g.read_by(objects[(k * 8) as usize].1, 0.15)).max().unwrap();
        eprintln!("4,002 objects: a query reads {worst} entries at most");
        assert!(worst <= 40, "a query read {worst} of {} entries", g.len());
        let wide = (0..500u32).map(|k| g.read_by(objects[(k * 8) as usize].1, crate::grab_target::QUERY_REACH)).max().unwrap();
        eprintln!("4,002 objects: a hand's targets read {wide} entries at most");
        assert!(wide <= 200, "a target query read {wide} of {} entries", g.len());
        assert_eq!(g.len(), 4002);
    }

    /// A frame's update of things that have not moved files nothing; one
    /// that moved within its cells files nothing either; one that crossed
    /// into another cell is filed once; one no longer told of is forgotten.
    #[test]
    fn an_update_costs_only_what_moved() {
        let mut objects = level(500);
        let mut g = filled(&objects);
        let filed = g.refiled();
        let update = |g: &mut GrabGrid, objects: &[(String, Vec3, Quat, Vec3)]| {
            g.begin();
            for (id, p, r, h) in objects {
                g.set(id, *p, *r, *h);
            }
            g.end();
        };
        update(&mut g, &objects);
        assert_eq!(g.refiled(), filed, "a still level was filed again");
        objects[10].1 = Vec3::new(10.25, 1.25, 10.25);
        update(&mut g, &objects);
        let filed = g.refiled();
        objects[10].1 += Vec3::splat(0.001);
        objects[10].3 = Vec3::splat(0.05);
        update(&mut g, &objects);
        let after_resize = g.refiled();
        objects[10].1 += Vec3::splat(0.001);
        update(&mut g, &objects);
        assert_eq!(g.refiled(), after_resize, "a move inside its cells filed it again");
        assert!(after_resize <= filed + 1);
        assert_eq!(g.nearest(objects[10].1, 0.15, |id| id == "thing_10").map(|x| x.0), Some("thing_10"));
        let was = objects[10].1;
        objects[10].1 = Vec3::new(-70.0, 2.0, 33.0);
        update(&mut g, &objects);
        assert_eq!(g.refiled(), after_resize + 1);
        assert_eq!(g.nearest(objects[10].1, 0.15, |id| id == "thing_10").map(|x| x.0), Some("thing_10"));
        assert!(g.nearest(was, 0.15, |id| id == "thing_10").is_none());
        let gone = objects.remove(10);
        update(&mut g, &objects);
        assert_eq!(g.len(), objects.len());
        assert!(g.nearest(gone.1, 0.15, |id| id == "thing_10").is_none());
        let other = &objects[100];
        assert_eq!(g.nearest(other.1, 0.15, |id| id == other.0).map(|x| x.0), Some(other.0.as_str()));
    }
}
