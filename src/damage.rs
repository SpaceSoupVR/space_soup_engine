use std::collections::HashMap;

use crate::scene::{distance_to_oriented_box, BreakableDef, GameObject};
use glam::Vec3;

#[derive(Debug, Clone, PartialEq)]
pub struct StageChange {
    pub object_id: String,

    pub from: Option<usize>,
    pub to: Option<usize>,

    pub solid: bool,
}

impl StageChange {

    #[must_use]
    pub fn breached(&self) -> bool {
        !self.solid
    }
}

#[derive(Debug, Default, Clone)]
pub struct DamageLedger {
    taken: HashMap<String, f32>,
}

impl DamageLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn damage_for(&self, object_id: &str) -> f32 {
        self.taken.get(object_id).copied().unwrap_or(0.0)
    }

    pub fn apply(&mut self, entity: &GameObject, amount: f32) -> Option<StageChange> {
        let breakable = entity.breakable.as_ref()?;
        if amount.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return None;
        }

        let before = self.damage_for(&entity.id);
        let after = before + amount;
        self.taken.insert(entity.id.clone(), after);

        let from = stage_index(breakable, before);
        let to = stage_index(breakable, after);
        if from == to {
            return None;
        }

        Some(StageChange {
            object_id: entity.id.clone(),
            from,
            to,
            solid: breakable.is_solid_at(after),
        })
    }

    #[must_use]
    pub fn hidden_parts_for<'a>(&self, entity: &'a GameObject) -> &'a [String] {
        match &entity.breakable {
            Some(b) => b.hidden_parts_at(self.damage_for(&entity.id), &entity.hidden_parts),
            None => &entity.hidden_parts,
        }
    }

    #[must_use]
    pub fn is_removed(&self, entity: &GameObject) -> bool {
        match &entity.breakable {
            Some(b) => b
                .stage_for(self.damage_for(&entity.id))
                .is_some_and(|s| s.removed),
            None => false,
        }
    }

    #[must_use]
    pub fn is_solid(&self, entity: &GameObject) -> bool {
        match &entity.breakable {
            Some(b) => b.is_solid_at(self.damage_for(&entity.id)),
            None => true,
        }
    }

    pub fn reset(&mut self) {
        self.taken.clear();
    }

    pub fn damaged_objects(&self) -> impl Iterator<Item = (&str, f32)> {
        self.taken.iter().map(|(id, d)| (id.as_str(), *d))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.taken.is_empty()
    }
}

fn stage_index(breakable: &BreakableDef, damage: f32) -> Option<usize> {
    let target = breakable.stage_for(damage)?;
    breakable
        .stages
        .iter()
        .position(|s| std::ptr::eq(s, target))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImpactTarget {

    pub index: usize,

    pub distance: f32,

    pub share: f32,
}

#[must_use]
pub fn impact_targets(objects: &[GameObject], point: Vec3, radius: f32) -> Vec<ImpactTarget> {
    let mut out = Vec::new();
    for (index, entity) in objects.iter().enumerate() {

        if entity.breakable.is_none() {
            continue;
        }
        let distance = distance_to_oriented_box(
            entity.cuboid.position,
            entity.cuboid.rotation,
            entity.cuboid.half_size,
            point,
        );
        let share = if radius > 0.0 {
            1.0 - distance / radius
        } else if distance <= 0.0 {
            1.0
        } else {
            0.0
        };
        if share > 0.0 {
            out.push(ImpactTarget {
                index,
                distance,
                share,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::DamageStage;

    fn wall() -> GameObject {
        GameObject {
            id: "compound_wall".into(),
            hidden_parts: vec!["scaffold".into()],
            breakable: Some(BreakableDef {
                health: 100.0,
                stages: vec![
                    DamageStage {
                        at: 0.5,
                        hidden_parts: vec!["intact".into()],
                        solid: true,
                        ..Default::default()
                    },
                    DamageStage {
                        at: 0.9,
                        hidden_parts: vec!["intact".into(), "cracked".into()],
                        solid: false,
                        ..Default::default()
                    },
                ],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn an_untouched_object_keeps_its_authored_parts_and_stays_solid() {
        let ledger = DamageLedger::new();
        let w = wall();
        assert_eq!(ledger.damage_for("compound_wall"), 0.0);
        assert_eq!(ledger.hidden_parts_for(&w), ["scaffold"]);
        assert!(ledger.is_solid(&w));
    }

    #[test]
    fn a_change_is_reported_only_when_a_stage_actually_changes() {
        let mut ledger = DamageLedger::new();
        let w = wall();

        assert!(ledger.apply(&w, 10.0).is_none(), "still undamaged");
        assert!(ledger.apply(&w, 10.0).is_none());
        assert!(ledger.apply(&w, 10.0).is_none());

        let change = ledger.apply(&w, 25.0).expect("crossing 50 changes stage");
        assert_eq!(change.from, None);
        assert_eq!(change.to, Some(0));
        assert!(change.solid);

        assert!(ledger.apply(&w, 5.0).is_none(), "still inside stage 0");

        let breach = ledger.apply(&w, 40.0).expect("crossing 90 breaches");
        assert_eq!(breach.to, Some(1));
        assert!(breach.breached(), "the wall should now be passable");
    }

    #[test]
    fn hidden_parts_follow_the_current_stage() {
        let mut ledger = DamageLedger::new();
        let w = wall();

        ledger.apply(&w, 60.0);
        assert_eq!(ledger.hidden_parts_for(&w), ["intact"]);
        assert!(ledger.is_solid(&w));

        ledger.apply(&w, 40.0);
        assert_eq!(ledger.hidden_parts_for(&w), ["intact", "cracked"]);
        assert!(!ledger.is_solid(&w));
    }

    #[test]
    fn damage_accumulates_across_hits() {
        let mut ledger = DamageLedger::new();
        let w = wall();
        for _ in 0..10 {
            ledger.apply(&w, 6.0);
        }
        assert_eq!(ledger.damage_for("compound_wall"), 60.0);
        assert_eq!(ledger.hidden_parts_for(&w), ["intact"]);
    }

    #[test]
    fn negative_damage_is_ignored_rather_than_healing() {
        let mut ledger = DamageLedger::new();
        let w = wall();
        ledger.apply(&w, 95.0);
        assert!(!ledger.is_solid(&w));

        assert!(ledger.apply(&w, -90.0).is_none());
        assert_eq!(ledger.damage_for("compound_wall"), 95.0);
        assert!(!ledger.is_solid(&w), "a negative must not un-breach it");
    }

    #[test]
    fn a_non_breakable_object_takes_no_damage_and_reports_nothing() {
        let mut ledger = DamageLedger::new();
        let plain = GameObject { id: "rock".into(), ..Default::default() };
        assert!(ledger.apply(&plain, 500.0).is_none());
        assert_eq!(ledger.damage_for("rock"), 0.0);
        assert!(ledger.is_solid(&plain));
        assert!(ledger.hidden_parts_for(&plain).is_empty());
    }

    #[test]
    fn reset_clears_everything() {
        let mut ledger = DamageLedger::new();
        let w = wall();
        ledger.apply(&w, 95.0);
        assert!(!ledger.is_empty());

        ledger.reset();
        assert!(ledger.is_empty());
        assert_eq!(ledger.damage_for("compound_wall"), 0.0);
        assert!(ledger.is_solid(&w), "a reset wall is whole again");
        assert_eq!(ledger.hidden_parts_for(&w), ["scaffold"]);
    }

    #[test]
    fn damaged_objects_lists_what_has_been_hit() {
        let mut ledger = DamageLedger::new();
        let w = wall();
        let other = GameObject { id: "door".into(), ..wall() };
        ledger.apply(&w, 20.0);
        ledger.apply(&other, 55.0);

        let mut listed: Vec<(String, f32)> = ledger
            .damaged_objects()
            .map(|(id, d)| (id.to_string(), d))
            .collect();
        listed.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(listed, vec![("compound_wall".into(), 20.0), ("door".into(), 55.0)]);
    }

    #[test]
    fn visually_identical_stages_are_still_distinct() {
        let mut ledger = DamageLedger::new();
        let twin = GameObject {
            id: "twin".into(),
            breakable: Some(BreakableDef {
                health: 100.0,
                stages: vec![
                    DamageStage { at: 0.3, hidden_parts: vec!["a".into()], solid: true, ..Default::default() },
                    DamageStage { at: 0.6, hidden_parts: vec!["a".into()], solid: true, ..Default::default() },
                ],
            }),
            ..Default::default()
        };

        assert_eq!(ledger.apply(&twin, 35.0).unwrap().to, Some(0));
        let second = ledger.apply(&twin, 30.0).expect("crossing into the twin stage is a change");
        assert_eq!(second.from, Some(0));
        assert_eq!(second.to, Some(1));
    }

    #[test]
    fn a_single_overwhelming_hit_reports_one_change_at_the_final_stage() {
        let mut ledger = DamageLedger::new();
        let w = wall();
        let change = ledger.apply(&w, 10_000.0).expect("one shell breaches outright");
        assert_eq!(change.from, None);
        assert_eq!(change.to, Some(1));
        assert!(change.breached());
    }

    fn chunk_row(n: usize) -> Vec<GameObject> {
        (0..n)
            .map(|i| GameObject {
                id: format!("wall_chunk_{i}"),
                cuboid: crate::scene::CuboidDef {
                    position: Vec3::new(i as f32, 0.0, 0.0),
                    half_size: Vec3::splat(0.5),
                    ..Default::default()
                },
                breakable: Some(BreakableDef {
                    health: 100.0,
                    stages: vec![DamageStage {
                        at: 1.0,
                        removed: true,
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn a_bullet_reaches_only_the_chunk_it_is_inside() {
        let row = chunk_row(5);
        let hit = impact_targets(&row, Vec3::new(2.1, 0.0, 0.0), 0.0);
        assert_eq!(hit.len(), 1, "a round is a point, not a blast");
        assert_eq!(hit[0].index, 2);
        assert_eq!(hit[0].share, 1.0, "a direct hit is not reduced");
    }

    #[test]
    fn a_bullet_that_misses_every_chunk_damages_nothing() {
        let row = chunk_row(5);
        assert!(impact_targets(&row, Vec3::new(2.0, 4.0, 0.0), 0.0).is_empty());
    }

    #[test]
    fn a_blast_reaches_several_chunks_and_falls_off_with_distance() {
        let row = chunk_row(7);
        let hit = impact_targets(&row, Vec3::new(3.0, 0.0, 0.0), 2.0);
        let ids: Vec<usize> = hit.iter().map(|t| t.index).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5], "everything within 2m of the centre");

        let centre = hit.iter().find(|t| t.index == 3).unwrap();
        let near = hit.iter().find(|t| t.index == 4).unwrap();
        let far = hit.iter().find(|t| t.index == 5).unwrap();
        assert_eq!(centre.share, 1.0);
        assert!(
            centre.share > near.share && near.share > far.share,
            "the chunk that was hit must come off worst: {centre:?} {near:?} {far:?}"
        );
    }

    #[test]
    fn falloff_is_measured_to_the_surface_not_the_centre() {

        let wall = vec![GameObject {
            id: "long_wall".into(),
            cuboid: crate::scene::CuboidDef {
                position: Vec3::ZERO,
                half_size: Vec3::new(5.0, 1.5, 0.15),
                ..Default::default()
            },
            breakable: Some(BreakableDef { health: 100.0, stages: vec![] }),
            ..Default::default()
        }];
        let hit = impact_targets(&wall, Vec3::new(4.9, 0.0, 0.0), 1.0);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].share, 1.0, "the point is inside the wall");
    }

    #[test]
    fn an_oriented_chunk_is_measured_against_the_box_that_is_on_screen() {

        let turned = vec![GameObject {
            id: "panel".into(),
            cuboid: crate::scene::CuboidDef {
                position: Vec3::ZERO,
                half_size: Vec3::new(2.0, 1.0, 0.1),
                rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                ..Default::default()
            },
            breakable: Some(BreakableDef { health: 100.0, stages: vec![] }),
            ..Default::default()
        }];
        assert!(
            impact_targets(&turned, Vec3::new(1.5, 0.0, 0.0), 0.0).is_empty(),
            "1.5m along x is outside a panel that was turned to face x"
        );
        assert_eq!(
            impact_targets(&turned, Vec3::new(0.0, 0.0, 1.5), 0.0).len(),
            1,
            "and inside it along z"
        );
    }

    #[test]
    fn objects_with_no_breakable_are_never_reported() {
        let mut row = chunk_row(3);
        row[1].breakable = None;
        let hit = impact_targets(&row, Vec3::new(1.0, 0.0, 0.0), 3.0);
        assert!(
            hit.iter().all(|t| t.index != 1),
            "an indestructible object cannot absorb a grenade"
        );
    }

    fn wall_of(n: usize) -> Vec<GameObject> {
        let width = 6.0 / n as f32;
        (0..n)
            .map(|i| GameObject {
                id: format!("wall_{i}"),
                cuboid: crate::scene::CuboidDef {
                    position: Vec3::new(width * (i as f32 + 0.5), 0.0, 0.0),
                    half_size: Vec3::new(width / 2.0, 1.5, 0.15),
                    ..Default::default()
                },
                breakable: Some(BreakableDef { health: 100.0, stages: vec![] }),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn finer_fracture_does_not_secretly_armour_a_wall() {

        let at = Vec3::new(3.0, 0.0, 0.0);
        let coarse = impact_targets(&wall_of(3), at, 1.5);
        let fine = impact_targets(&wall_of(12), at, 1.5);

        let best = |v: &[ImpactTarget]| v.iter().map(|t| t.share).fold(0.0_f32, f32::max);
        assert_eq!(best(&coarse), 1.0);
        assert_eq!(best(&fine), 1.0, "the piece that was hit still takes it all");
        assert!(
            fine.len() > coarse.len(),
            "and the finer wall really does put more pieces in range: {} vs {}",
            fine.len(),
            coarse.len()
        );
    }

    #[test]
    fn a_removed_stage_stops_the_object_being_drawn_and_being_solid() {
        let mut ledger = DamageLedger::new();
        let chunk = chunk_row(1).remove(0);
        assert!(!ledger.is_removed(&chunk), "intact until it is shot");
        assert!(ledger.is_solid(&chunk));

        ledger.apply(&chunk, 100.0);
        assert!(ledger.is_removed(&chunk));
        assert!(
            !ledger.is_solid(&chunk),
            "an object that is not drawn must not still block fire"
        );
    }

    #[test]
    fn a_removed_stage_is_not_solid_even_when_authored_solid() {

        let b = BreakableDef {
            health: 10.0,
            stages: vec![DamageStage { at: 0.5, solid: true, removed: true, ..Default::default() }],
        };
        assert!(b.is_solid_at(1.0), "below the threshold, untouched");
        assert!(!b.is_solid_at(9.0));
    }
}
