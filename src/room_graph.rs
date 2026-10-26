use glam::Vec3;

pub const ADJACENCY_SLACK: f32 = 0.6;

#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub id: String,
    pub min: Vec3,
    pub max: Vec3,

    pub zone: usize,
}

impl Room {
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    pub fn centre(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn volume(&self) -> f32 {
        let d = (self.max - self.min).abs();
        d.x * d.y * d.z
    }

    fn touches(&self, other: &Room) -> bool {
        let slack = Vec3::splat(ADJACENCY_SLACK);
        (self.min - slack).cmple(other.max).all() && (self.max + slack).cmpge(other.min).all()
    }
}

#[derive(Clone, Debug, Default)]
pub struct RoomGraph {
    pub rooms: Vec<Room>,

    neighbours: Vec<Vec<usize>>,
}

impl RoomGraph {

    pub fn new(rooms: Vec<Room>) -> Self {
        let mut neighbours = vec![Vec::new(); rooms.len()];
        for i in 0..rooms.len() {
            for j in (i + 1)..rooms.len() {
                if rooms[i].touches(&rooms[j]) {
                    neighbours[i].push(j);
                    neighbours[j].push(i);
                }
            }
        }
        Self { rooms, neighbours }
    }

    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rooms.len()
    }

    pub fn neighbours(&self, room: usize) -> &[usize] {
        self.neighbours.get(room).map(std::vec::Vec::as_slice).unwrap_or(&[])
    }

    pub fn room_at(&self, p: Vec3) -> Option<usize> {
        self.rooms
            .iter()
            .enumerate()
            .filter(|(_, r)| r.contains(p))
            .min_by(|(_, a), (_, b)| {
                a.volume().partial_cmp(&b.volume()).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    }

    pub fn visible_from(
        &self,
        player: Vec3,
        max_depth: u32,
        visible: impl Fn(Vec3, Vec3) -> bool,
    ) -> Vec<usize> {
        let Some(start) = self.room_at(player) else {

            return (0..self.rooms.len())
                .filter(|&i| visible(self.rooms[i].min, self.rooms[i].max))
                .collect();
        };

        let mut seen = vec![false; self.rooms.len()];
        let mut out = Vec::new();
        let mut frontier = Vec::new();

        let zone = self.rooms[start].zone;
        for (i, r) in self.rooms.iter().enumerate() {
            if r.zone == zone {
                seen[i] = true;
                out.push(i);
                frontier.push((i, 0u32));
            }
        }
        while let Some((room, depth)) = frontier.pop() {
            if depth >= max_depth {
                continue;
            }
            for &n in self.neighbours(room) {
                if seen[n] {
                    continue;
                }
                seen[n] = true;
                if visible(self.rooms[n].min, self.rooms[n].max) {
                    out.push(n);
                    frontier.push((n, depth + 1));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str, min: [f32; 3], max: [f32; 3]) -> Room {
        Room { id: id.to_string(), min: Vec3::from(min), max: Vec3::from(max), zone: 0 }
    }

    fn room_z(id: &str, min: [f32; 3], max: [f32; 3], zone: usize) -> Room {
        Room { zone, ..room(id, min, max) }
    }

    #[test]
    fn adjacency_follows_which_volumes_touch() {
        let g = RoomGraph::new(vec![
            room("a", [0.0, 0.0, 0.0], [4.0, 3.0, 4.0]),

            room("b", [4.3, 0.0, 0.0], [8.0, 3.0, 4.0]),

            room("c", [50.0, 0.0, 0.0], [54.0, 3.0, 4.0]),
        ]);
        assert_eq!(g.neighbours(0), &[1]);
        assert_eq!(g.neighbours(1), &[0]);
        assert!(g.neighbours(2).is_empty(), "a distant room was joined to the level");
    }

    #[test]
    fn a_wide_gap_is_not_adjacency() {
        let g = RoomGraph::new(vec![
            room("a", [0.0, 0.0, 0.0], [4.0, 3.0, 4.0]),
            room("b", [4.0 + ADJACENCY_SLACK + 0.1, 0.0, 0.0], [8.0, 3.0, 4.0]),
        ]);
        assert!(g.neighbours(0).is_empty());
    }

    #[test]
    fn the_smallest_containing_volume_wins() {
        let g = RoomGraph::new(vec![
            room("hall", [0.0, 0.0, 0.0], [10.0, 3.0, 10.0]),
            room("alcove", [1.0, 0.0, 1.0], [3.0, 3.0, 3.0]),
        ]);
        assert_eq!(g.room_at(Vec3::new(2.0, 1.0, 2.0)), Some(1));
        assert_eq!(g.room_at(Vec3::new(8.0, 1.0, 8.0)), Some(0));
        assert_eq!(g.room_at(Vec3::new(-5.0, 1.0, 0.0)), None);
    }

    #[test]
    fn the_players_own_room_survives_a_frustum_that_rejects_everything() {
        let g = RoomGraph::new(vec![
            room_z("a", [0.0, 0.0, 0.0], [4.0, 3.0, 4.0], 0),
            room_z("b", [4.3, 0.0, 0.0], [8.0, 3.0, 4.0], 1),
        ]);
        let got = g.visible_from(Vec3::new(2.0, 1.0, 2.0), 2, |_, _| false);
        assert_eq!(got, vec![0], "the room the player is standing in was culled");
    }

    #[test]
    fn a_neighbour_is_included_only_when_the_frustum_accepts_it() {
        let g = RoomGraph::new(vec![
            room_z("a", [0.0, 0.0, 0.0], [4.0, 3.0, 4.0], 0),
            room_z("b", [4.3, 0.0, 0.0], [8.0, 3.0, 4.0], 1),
        ]);
        let here = Vec3::new(2.0, 1.0, 2.0);

        let east = |min: Vec3, _max: Vec3| min.x > 4.0;
        let mut got = g.visible_from(here, 2, east);
        got.sort_unstable();
        assert_eq!(got, vec![0, 1]);

        let west = |_min: Vec3, max: Vec3| max.x < 0.0;
        assert_eq!(g.visible_from(here, 2, west), vec![0]);
    }

    #[test]
    fn the_fill_stops_at_max_depth() {
        let rooms: Vec<Room> = (0..6)
            .map(|i| {
                let x = i as f32 * 4.3;
                room_z(&format!("r{i}"), [x, 0.0, 0.0], [x + 4.0, 3.0, 4.0], i as usize)
            })
            .collect();
        let g = RoomGraph::new(rooms);
        let here = Vec3::new(2.0, 1.0, 2.0);
        assert_eq!(g.visible_from(here, 1, |_, _| true).len(), 2, "depth 1 is here plus neighbours");
        assert_eq!(g.visible_from(here, 2, |_, _| true).len(), 3);
        assert_eq!(g.visible_from(here, 99, |_, _| true).len(), 6);
    }

    #[test]
    fn standing_outside_every_room_still_sees_the_level() {
        let g = RoomGraph::new(vec![
            room("a", [0.0, 0.0, 0.0], [4.0, 3.0, 4.0]),
            room("b", [50.0, 0.0, 0.0], [54.0, 3.0, 4.0]),
        ]);
        let outside = Vec3::new(-20.0, 1.0, 0.0);
        assert_eq!(
            g.visible_from(outside, 2, |_, _| true).len(),
            2,
            "stepping outdoors culled the whole level",
        );

        let near = |min: Vec3, _: Vec3| min.x < 10.0;
        assert_eq!(g.visible_from(outside, 2, near), vec![0]);
    }

    #[test]
    fn every_part_of_a_multi_brush_room_stays_visible() {
        let g = RoomGraph::new(vec![
            room_z("hall_a", [0.0, 0.0, 0.0], [4.0, 3.0, 12.0], 0),
            room_z("hall_b", [4.0, 0.0, 0.0], [12.0, 3.0, 4.0], 0),
            room_z("other", [40.0, 0.0, 0.0], [44.0, 3.0, 4.0], 1),
        ]);
        let mut got = g.visible_from(Vec3::new(2.0, 1.0, 10.0), 2, |_, _| false);
        got.sort_unstable();
        assert_eq!(got, vec![0, 1], "the far arm of an L-shaped room was culled");
        assert!(!got.contains(&2), "an unrelated room came along for the ride");
    }

    #[test]
    fn zones_group_carves_that_share_space() {
        use crate::brush::{BrushDef, BrushSolid};
        fn boxy(min: Vec3, max: Vec3) -> BrushSolid {
            let faces = [
                (Vec3::X, max.x), (-Vec3::X, -min.x),
                (Vec3::Y, max.y), (-Vec3::Y, -min.y),
                (Vec3::Z, max.z), (-Vec3::Z, -min.z),
            ]
            .into_iter()
            .map(|(n, d): (Vec3, f32)| {
                serde_json::from_value(serde_json::json!({
                    "plane": [n.x as f64, n.y as f64, n.z as f64, d as f64],
                }))
                .unwrap()
            })
            .collect();
            BrushSolid { faces }
        }
        let brush = BrushDef {
            solids: vec![boxy(Vec3::splat(-50.0), Vec3::splat(50.0))],
            subtract: vec![

                boxy(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 3.0, 12.0)),
                boxy(Vec3::new(4.0, 0.0, 0.0), Vec3::new(12.0, 3.0, 4.0)),

                boxy(Vec3::new(0.0, 0.0, 13.0), Vec3::new(4.0, 3.0, 20.0)),
            ],
            ..Default::default()
        };
        let (rooms, _) = rooms_from_scene(&[("hall", &brush)]);
        assert_eq!(rooms.len(), 3);
        assert_eq!(rooms[0].zone, rooms[1].zone, "the two arms of one room split into two zones");
        assert_ne!(rooms[0].zone, rooms[2].zone, "a room across a wall was merged in");
    }

    #[test]
    fn an_empty_graph_is_harmless() {
        let g = RoomGraph::new(Vec::new());
        assert!(g.is_empty());
        assert_eq!(g.room_at(Vec3::ZERO), None);
        assert!(g.visible_from(Vec3::ZERO, 2, |_, _| true).is_empty());
    }
}

pub const MIN_ROOM_EXTENT: f32 = 1.4;

pub const MIN_PORTAL_EXTENT: f32 = 0.5;

fn overlaps(a: &Room, b: &Room) -> bool {
    const EPS: f32 = 0.05;
    let e = Vec3::splat(EPS);
    (a.min - e).cmple(b.max).all() && (a.max + e).cmpge(b.min).all()
}

fn v3(p: crate::brush::Vec3) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

pub fn rooms_from_scene(
    brushes: &[(&str, &crate::brush::BrushDef)],
) -> (Vec<Room>, Vec<(usize, usize)>) {
    let mut rooms = Vec::new();
    let mut portals = Vec::new();
    for (id, brush) in brushes {
        for (i, carve) in brush.subtract.iter().enumerate() {
            let Some((lo, hi)) = crate::brush::solid_bounds(carve) else { continue };

            let (min, max) = (v3(lo), v3(hi));
            let extent = (max - min).abs();
            if extent.min_element() >= MIN_ROOM_EXTENT {

                rooms.push(Room { id: format!("{id}#{i}"), min, max, zone: 0 });
            } else if extent.min_element() >= MIN_PORTAL_EXTENT {
                portals.push((min, max));
            }
        }
    }

    let mut parent: Vec<usize> = (0..rooms.len()).collect();
    fn find(parent: &mut Vec<usize>, mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..rooms.len() {
        for j in (i + 1)..rooms.len() {
            if overlaps(&rooms[i], &rooms[j]) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut zone_of = std::collections::HashMap::new();
    for i in 0..rooms.len() {
        let root = find(&mut parent, i);
        let next = zone_of.len();
        rooms[i].zone = *zone_of.entry(root).or_insert(next);
    }

    let mut links = Vec::new();
    for (pmin, pmax) in &portals {
        let touching: Vec<usize> = rooms
            .iter()
            .enumerate()
            .filter(|(_, r)| doorway_touches(*pmin, *pmax, r.min, r.max))
            .map(|(i, _)| i)
            .collect();
        for a in 0..touching.len() {
            for b in (a + 1)..touching.len() {
                links.push((touching[a], touching[b]));
            }
        }
    }
    links.sort_unstable();
    links.dedup();
    (rooms, links)
}

const WALL_SAMPLE_STEP: f32 = 0.1;
const WALL_SAMPLE_OFFSET: f32 = 0.01;

pub fn closed_room_boxes(
    brushes: &[(&str, &crate::brush::BrushDef)],
    doorways: &[(Vec3, Vec3)],
) -> Vec<(Vec3, Vec3)> {
    use crate::brush::BrushSolid;
    let bounds = |s: &BrushSolid| crate::brush::solid_bounds(s).map(|(lo, hi)| (v3(lo), v3(hi)));
    let inside = |s: &BrushSolid, p: Vec3| {
        s.faces.iter().all(|f| {
            Vec3::new(f.plane[0] as f32, f.plane[1] as f32, f.plane[2] as f32).dot(p) <= f.plane[3] as f32 + 1e-4
        })
    };
    let overlap = |a: (Vec3, Vec3), b: (Vec3, Vec3)| a.0.cmplt(b.1).all() && b.0.cmplt(a.1).all();
    let solids: Vec<&BrushSolid> = brushes.iter().flat_map(|(_, b)| b.solids.iter()).collect();
    let carves: Vec<(&BrushSolid, (Vec3, Vec3))> = brushes
        .iter()
        .flat_map(|(_, b)| b.subtract.iter())
        .filter_map(|c| bounds(c).map(|b| (c, b)))
        .collect();

    let known_door = |k: (Vec3, Vec3)| {
        let thinnest = (k.1 - k.0).min_element();
        (MIN_PORTAL_EXTENT..MIN_ROOM_EXTENT).contains(&thinnest) && doorways.iter().any(|&d| overlap(k, d))
    };
    let mut closed = Vec::new();
    for &(carve, room) in &carves {
        if (room.1 - room.0).min_element() < MIN_ROOM_EXTENT {
            continue;
        }

        let reach = (room.0 - Vec3::splat(0.02), room.1 + Vec3::splat(0.02));
        let foreign = carves
            .iter()
            .any(|&(other, k)| !std::ptr::eq(other, carve) && overlap(k, reach) && !known_door(k));
        if foreign {
            continue;
        }

        let walled = |p: Vec3| {
            solids.iter().any(|s| inside(s, p))
                || carves.iter().any(|&(k, kb)| known_door(kb) && inside(k, p))
        };
        let mut open = false;
        'faces: for axis in 0..3 {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            let steps = |lo: f32, hi: f32| ((hi - lo) / WALL_SAMPLE_STEP).ceil().max(1.0) as usize;
            let (na, nb) = (steps(room.0[a], room.1[a]), steps(room.0[b], room.1[b]));
            for side in [room.0[axis] - WALL_SAMPLE_OFFSET, room.1[axis] + WALL_SAMPLE_OFFSET] {
                for i in 0..=na {
                    for j in 0..=nb {
                        let mut p = Vec3::ZERO;
                        p[axis] = side;
                        p[a] = room.0[a] + (room.1[a] - room.0[a]) * i as f32 / na as f32;
                        p[b] = room.0[b] + (room.1[b] - room.0[b]) * j as f32 / nb as f32;
                        if !walled(p) {
                            open = true;
                            break 'faces;
                        }
                    }
                }
            }
        }
        if !open {
            closed.push(room);
        }
    }
    closed
}

pub fn doorway_touches(pmin: Vec3, pmax: Vec3, min: Vec3, max: Vec3) -> bool {
    let slack = Vec3::splat(ADJACENCY_SLACK);
    (min - slack).cmple(pmax).all() && (max + slack).cmpge(pmin).all()
}

pub fn doorways_from_scene(brushes: &[(&str, &crate::brush::BrushDef)]) -> Vec<(Vec3, Vec3)> {
    let mut out = Vec::new();
    for (_, brush) in brushes {
        for carve in &brush.subtract {
            let Some((lo, hi)) = crate::brush::solid_bounds(carve) else { continue };
            let (min, max) = (v3(lo), v3(hi));
            let thin = (max - min).abs().min_element();
            if (MIN_PORTAL_EXTENT..MIN_ROOM_EXTENT).contains(&thin) {
                out.push((min, max));
            }
        }
    }
    out
}

impl RoomGraph {

    pub fn from_derived(rooms: Vec<Room>, portals: &[(usize, usize)]) -> Self {
        let mut g = Self::new(rooms);
        for &(a, b) in portals {
            if a >= g.rooms.len() || b >= g.rooms.len() || a == b {
                continue;
            }
            if !g.neighbours[a].contains(&b) {
                g.neighbours[a].push(b);
                g.neighbours[b].push(a);
            }
        }
        g
    }
}

#[cfg(test)]
mod derivation_tests {
    use super::*;
    use crate::brush::{BrushDef, BrushFace, BrushSolid};

    fn boxy(min: Vec3, max: Vec3) -> BrushSolid {
        let faces = [
            (Vec3::X, max.x),
            (-Vec3::X, -min.x),
            (Vec3::Y, max.y),
            (-Vec3::Y, -min.y),
            (Vec3::Z, max.z),
            (-Vec3::Z, -min.z),
        ]
        .into_iter()
        .map(|(n, d): (Vec3, f32)| {

            serde_json::from_value::<BrushFace>(serde_json::json!({
                "plane": [n.x as f64, n.y as f64, n.z as f64, d as f64],
            }))
            .expect("a plane is the only required field of a face")
        })
        .collect();
        BrushSolid { faces }
    }

    #[test]
    fn a_room_is_closed_only_when_walled_all_round_but_its_known_doorways() {
        let shell = boxy(Vec3::new(-3.0, -0.3, -10.0), Vec3::new(3.0, 3.4, 4.0));
        let interior = boxy(Vec3::new(-2.7, 0.0, -9.7), Vec3::new(2.7, 3.1, 3.7));
        let doorway = (Vec3::new(-0.8, 0.0, 3.5), Vec3::new(0.8, 2.2, 4.3));
        let door_carve = boxy(doorway.0, doorway.1);
        let room = (Vec3::new(-2.7, 0.0, -9.7), Vec3::new(2.7, 3.1, 3.7));
        let hall = |subtract: Vec<BrushSolid>| BrushDef { solids: vec![shell.clone()], subtract };

        let b = hall(vec![interior.clone(), door_carve.clone()]);
        assert_eq!(closed_room_boxes(&[("hall", &b)], &[doorway]), vec![room]);

        assert!(closed_room_boxes(&[("hall", &b)], &[]).is_empty());

        let roofless = BrushDef {
            solids: vec![shell.clone()],
            subtract: vec![boxy(Vec3::new(-2.7, 0.0, -9.7), Vec3::new(2.7, 3.6, 3.7))],
        };
        assert!(closed_room_boxes(&[("hall", &roofless)], &[]).is_empty());

        let slit = boxy(Vec3::new(2.5, 1.0, -2.0), Vec3::new(3.3, 1.2, 0.0));
        let b = hall(vec![interior.clone(), door_carve.clone(), slit]);
        assert!(closed_room_boxes(&[("hall", &b)], &[doorway]).is_empty());

        let niche = boxy(Vec3::new(2.5, 1.0, -2.0), Vec3::new(2.9, 1.3, 0.0));
        let b = hall(vec![interior.clone(), door_carve.clone(), niche]);
        assert!(closed_room_boxes(&[("hall", &b)], &[doorway]).is_empty());

        let hall_b = hall(vec![interior, door_carve]);
        let hallway_room = (Vec3::new(-0.8, 0.0, 4.0), Vec3::new(0.8, 2.2, 9.0));
        let hallway = |start: f32| BrushDef {
            solids: vec![boxy(Vec3::new(-1.1, -0.3, start), Vec3::new(1.1, 2.5, 9.3))],
            subtract: vec![boxy(Vec3::new(-0.8, 0.0, start), Vec3::new(0.8, 2.2, 9.0))],
        };
        let joined = hallway(4.0);
        let closed = closed_room_boxes(&[("hall", &hall_b), ("hallway", &joined)], &[doorway]);
        assert!(closed.contains(&hallway_room), "{closed:?}");
        let gapped = hallway(4.5);
        let closed = closed_room_boxes(&[("hall", &hall_b), ("hallway", &gapped)], &[doorway]);
        assert!(!closed.iter().any(|r| r.0.z == 4.5), "{closed:?}");
    }

    #[test]
    fn a_carved_space_becomes_a_room() {
        let brush = BrushDef {
            solids: vec![boxy(Vec3::splat(-6.0), Vec3::splat(6.0))],
            subtract: vec![boxy(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 3.0, 5.0))],
            ..Default::default()
        };
        let (rooms, _) = rooms_from_scene(&[("hall", &brush)]);
        assert_eq!(rooms.len(), 1, "the carved interior did not become a room");
        assert_eq!(rooms[0].min, Vec3::new(-5.0, 0.0, -5.0));
        assert_eq!(rooms[0].max, Vec3::new(5.0, 3.0, 5.0));
    }

    #[test]
    fn a_doorway_is_a_portal_not_a_room() {
        let brush = BrushDef {
            solids: vec![boxy(Vec3::splat(-6.0), Vec3::splat(6.0))],
            subtract: vec![
                boxy(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 3.0, 5.0)),

                boxy(Vec3::new(-0.6, 0.0, 4.9), Vec3::new(0.6, 2.1, 5.2)),
            ],
            ..Default::default()
        };
        let (rooms, _) = rooms_from_scene(&[("hall", &brush)]);
        assert_eq!(rooms.len(), 1, "the doorway was counted as a room to stand in");
    }

    #[test]
    fn a_doorway_joins_the_rooms_it_opens_between() {
        let a = BrushDef {
            solids: vec![boxy(Vec3::new(-6.0, -1.0, -6.0), Vec3::new(6.0, 4.0, 0.0))],
            subtract: vec![boxy(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 3.0, -0.4))],
            ..Default::default()
        };
        let b = BrushDef {
            solids: vec![boxy(Vec3::new(-6.0, -1.0, 0.0), Vec3::new(6.0, 4.0, 6.0))],
            subtract: vec![
                boxy(Vec3::new(-5.0, 0.0, 0.4), Vec3::new(5.0, 3.0, 5.0)),

                boxy(Vec3::new(-0.6, 0.0, -0.6), Vec3::new(0.6, 2.1, 0.6)),
            ],
            ..Default::default()
        };
        let (rooms, portals) = rooms_from_scene(&[("north", &a), ("south", &b)]);
        assert_eq!(rooms.len(), 2);
        assert_eq!(portals, vec![(0, 1)], "the doorway did not join the two rooms");

        let g = RoomGraph::from_derived(rooms, &portals);
        assert_eq!(g.neighbours(0), &[1]);
    }

    #[test]
    fn a_shallow_recess_is_neither_a_room_nor_a_portal() {
        let brush = BrushDef {
            solids: vec![boxy(Vec3::splat(-6.0), Vec3::splat(6.0))],
            subtract: vec![boxy(Vec3::new(-0.3, 1.0, 4.9), Vec3::new(0.3, 1.4, 5.1))],
            ..Default::default()
        };
        let (rooms, portals) = rooms_from_scene(&[("hall", &brush)]);
        assert!(rooms.is_empty() && portals.is_empty());
    }

    #[test]
    fn doorways_are_the_thin_carves() {
        let brush = BrushDef {
            solids: vec![boxy(Vec3::splat(-6.0), Vec3::splat(6.0))],
            subtract: vec![
                boxy(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 3.0, 5.0)),
                boxy(Vec3::new(-0.6, 0.0, 4.9), Vec3::new(0.6, 2.1, 5.6)),
            ],
            ..Default::default()
        };
        let doors = doorways_from_scene(&[("hall", &brush)]);
        assert_eq!(doors.len(), 1);
        assert_eq!(doors[0].0, Vec3::new(-0.6, 0.0, 4.9));
    }

    #[test]
    fn a_solid_block_derives_no_rooms() {
        let brush = BrushDef {
            solids: vec![boxy(Vec3::splat(-1.0), Vec3::splat(1.0))],
            ..Default::default()
        };
        let (rooms, portals) = rooms_from_scene(&[("pillar", &brush)]);
        assert!(rooms.is_empty() && portals.is_empty());
    }
}
