use glam::Vec3;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_center_half(center: Vec3, half: Vec3) -> Self {
        Self { min: center - half, max: center + half }
    }

    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x && self.max.x >= other.min.x &&
        self.min.y <= other.max.y && self.max.y >= other.min.y &&
        self.min.z <= other.max.z && self.max.z >= other.min.z
    }
}

#[derive(Debug, Default)]
pub struct CollisionTracker {
    active_pairs: HashSet<(String, String)>,
}

#[derive(Debug, Clone)]
pub enum CollisionEvent {
    Enter(String, String),
    Exit(String, String),
}

impl CollisionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bodies: &[(String, Aabb)]) -> Vec<CollisionEvent> {
        let mut current_pairs: HashSet<(String, String)> = HashSet::new();

        for i in 0..bodies.len() {
            for j in (i + 1)..bodies.len() {
                let (id_a, aabb_a) = &bodies[i];
                let (id_b, aabb_b) = &bodies[j];
                if aabb_a.overlaps(aabb_b) {
                    current_pairs.insert(ordered_pair(id_a, id_b));
                }
            }
        }

        let mut events = Vec::new();

        for pair in current_pairs.difference(&self.active_pairs) {
            events.push(CollisionEvent::Enter(pair.0.clone(), pair.1.clone()));
        }
        for pair in self.active_pairs.difference(&current_pairs) {
            events.push(CollisionEvent::Exit(pair.0.clone(), pair.1.clone()));
        }

        self.active_pairs = current_pairs;
        events
    }
}

fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}
