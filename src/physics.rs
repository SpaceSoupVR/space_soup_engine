use glam::{Quat, Vec3};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_center_half(center: Vec3, half: Vec3) -> Self {
        Self {
            min: center - half,
            max: center + half,
        }
    }

    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
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

fn safe_div(v: f32) -> f32 {
    if v == 0.0 {
        1e-12
    } else {
        v
    }
}

/// Is a point inside a rotated box?
///
/// Rotated, unlike `Aabb::overlaps`, because a trigger zone is usually turned
/// to face something: an axis-aligned test on a 45-degree volume claims corners
/// the author can see are outside it, and the symptom is a door that opens
/// while you are still in the corridor.
pub fn point_in_obb(p: Vec3, center: Vec3, half_size: Vec3, rotation: Quat) -> bool {
    let local = rotation.conjugate() * (p - center);
    local.x.abs() <= half_size.x && local.y.abs() <= half_size.y && local.z.abs() <= half_size.z
}

pub fn ray_intersect_obb(
    origin: Vec3,
    dir: Vec3,
    center: Vec3,
    half_size: Vec3,
    rotation: Quat,
    max_dist: f32,
) -> Option<f32> {
    let inv_rot = rotation.conjugate();
    let local_origin = inv_rot * (origin - center);
    let local_dir = inv_rot * dir;

    let inv_dir = Vec3::new(
        1.0 / safe_div(local_dir.x),
        1.0 / safe_div(local_dir.y),
        1.0 / safe_div(local_dir.z),
    );
    let t1 = (-half_size - local_origin) * inv_dir;
    let t2 = (half_size - local_origin) * inv_dir;
    let tmin = t1.min(t2);
    let tmax = t1.max(t2);
    let t_enter = tmin.max_element();
    let t_exit = tmax.min_element();

    if t_enter > t_exit || t_exit <= 1e-4 || t_enter >= max_dist {
        return None;
    }
    Some(t_enter.max(1e-4))
}

#[cfg(test)]
mod ray_intersect_obb_test {
    use super::*;

    #[test]
    fn hits_a_box_straight_ahead() {
        let dist = ray_intersect_obb(
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::X,
            Vec3::ZERO,
            Vec3::splat(0.5),
            Quat::IDENTITY,
            100.0,
        );
        assert!(dist.is_some());
        assert!((dist.unwrap() - 4.5).abs() < 1e-4);
    }

    #[test]
    fn misses_a_box_the_ray_never_crosses() {
        let dist = ray_intersect_obb(
            Vec3::new(-5.0, 5.0, 0.0),
            Vec3::X,
            Vec3::ZERO,
            Vec3::splat(0.5),
            Quat::IDENTITY,
            100.0,
        );
        assert!(dist.is_none());
    }

    #[test]
    fn a_box_entirely_behind_the_origin_does_not_count_as_a_hit() {
        let dist = ray_intersect_obb(
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::X,
            Vec3::ZERO,
            Vec3::splat(0.5),
            Quat::IDENTITY,
            100.0,
        );
        assert!(dist.is_none());
    }

    #[test]
    fn respects_max_dist() {
        let origin = Vec3::new(-5.0, 0.0, 0.0);
        let dir = Vec3::X;
        let center = Vec3::ZERO;
        let half_size = Vec3::splat(0.5);
        assert!(ray_intersect_obb(origin, dir, center, half_size, Quat::IDENTITY, 4.0).is_none());
        assert!(ray_intersect_obb(origin, dir, center, half_size, Quat::IDENTITY, 5.0).is_some());
    }

    #[test]
    fn hits_a_rotated_box_along_its_true_orientation() {
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let dist = ray_intersect_obb(
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::X,
            Vec3::ZERO,
            Vec3::splat(0.5),
            rotation,
            100.0,
        );
        assert!(dist.is_some());
        let unrotated_dist = 4.5;
        assert!(
            dist.unwrap() < unrotated_dist,
            "expected the rotated box's diamond corner to be hit sooner than the unrotated face, got {:?}",
            dist
        );
    }
}

