//! Accumulated damage, and the structural changes it causes.
//!
//! Damage is RUNTIME state, deliberately not part of the scene document. The
//! scene says what a wall is made of and how it comes apart; how much of a
//! beating it has taken this match is not a property of the level, and writing
//! it back would mean a level file that differs depending on who played it.
//!
//! It is also why this resets per match rather than persisting. A persistent
//! world would change that -- and it is a real possibility here -- so the reset
//! is an explicit call rather than an assumption baked into the representation:
//! keeping the ledger and dropping the reset is the whole change.

use std::collections::HashMap;

use crate::scene::{distance_to_oriented_box, BreakableDef, GameObject};
use glam::Vec3;

/// A structure moving from one damage stage to another.
///
/// Reported only on a CHANGE, never per hit. A wall absorbing thirty rounds
/// crosses two thresholds; the server needs to act twice, not thirty times.
#[derive(Debug, Clone, PartialEq)]
pub struct StageChange {
    pub object_id: String,
    /// Index into the object's stage list, or `None` for undamaged.
    pub from: Option<usize>,
    pub to: Option<usize>,
    /// Whether the object still blocks movement and fire after this change.
    pub solid: bool,
}

impl StageChange {
    /// Whether this change opened the structure up -- the moment a breach
    /// happens, which is what gameplay and physics actually care about.
    pub fn breached(&self) -> bool {
        !self.solid
    }
}

/// How much damage every object has taken, this match.
#[derive(Debug, Default, Clone)]
pub struct DamageLedger {
    taken: HashMap<String, f32>,
}

impl DamageLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Damage an object has accumulated. Unknown objects have taken none.
    pub fn damage_for(&self, object_id: &str) -> f32 {
        self.taken.get(object_id).copied().unwrap_or(0.0)
    }

    /// Apply damage, returning a stage change only if one actually happened.
    ///
    /// Takes the object rather than just its id so the caller cannot apply
    /// damage against a stale or mismatched `BreakableDef` -- the definition and
    /// the thing being damaged arrive together or not at all.
    ///
    /// Negative amounts are ignored rather than healing. Repair is a real
    /// feature and a defensible one, but it is not this one, and letting a
    /// negative slip through would make a mis-signed damage value silently
    /// rebuild a wall.
    pub fn apply(&mut self, object: &GameObject, amount: f32) -> Option<StageChange> {
        let breakable = object.breakable.as_ref()?;
        if !(amount > 0.0) {
            return None;
        }

        let before = self.damage_for(&object.id);
        let after = before + amount;
        self.taken.insert(object.id.clone(), after);

        let from = stage_index(breakable, before);
        let to = stage_index(breakable, after);
        if from == to {
            return None;
        }

        Some(StageChange {
            object_id: object.id.clone(),
            from,
            to,
            solid: breakable.is_solid_at(after),
        })
    }

    /// The parts an object should be hiding right now.
    ///
    /// Returns the object's authored set when it has taken no damage or is not
    /// breakable, so this is safe to call for every object every frame.
    pub fn hidden_parts_for<'a>(&self, object: &'a GameObject) -> &'a [String] {
        match &object.breakable {
            Some(b) => b.hidden_parts_at(self.damage_for(&object.id), &object.hidden_parts),
            None => &object.hidden_parts,
        }
    }

    /// Whether an object has been destroyed outright and should not be drawn.
    ///
    /// Separate from `is_solid` because they are separate authored fields: a
    /// wall can be shot through and still standing, and a chunk can be gone.
    pub fn is_removed(&self, object: &GameObject) -> bool {
        match &object.breakable {
            Some(b) => b
                .stage_for(self.damage_for(&object.id))
                .is_some_and(|s| s.removed),
            None => false,
        }
    }

    /// Whether an object still blocks movement and fire.
    pub fn is_solid(&self, object: &GameObject) -> bool {
        match &object.breakable {
            Some(b) => b.is_solid_at(self.damage_for(&object.id)),
            None => true,
        }
    }

    /// Forget everything. Called when a match ends or a scene changes.
    ///
    /// A scene change resets too: object ids are only unique within a scene, so
    /// carrying damage across would apply one level's battering to whatever
    /// happens to share a name in the next.
    pub fn reset(&mut self) {
        self.taken.clear();
    }

    /// Objects that have taken any damage, for a late-joining client to be
    /// caught up with.
    pub fn damaged_objects(&self) -> impl Iterator<Item = (&str, f32)> {
        self.taken.iter().map(|(id, d)| (id.as_str(), *d))
    }

    pub fn is_empty(&self) -> bool {
        self.taken.is_empty()
    }
}

/// Which stage a damage total lands in, as an index rather than a reference.
///
/// An index because comparing two stages by value would call two structures
/// with identical thresholds and parts "the same stage", and they are not --
/// a wall with two visually identical stages should still fire a change when it
/// passes between them.
fn stage_index(breakable: &BreakableDef, damage: f32) -> Option<usize> {
    let target = breakable.stage_for(damage)?;
    breakable
        .stages
        .iter()
        .position(|s| std::ptr::eq(s, target))
}

/// One breakable an impact reached, and how hard.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactTarget {
    /// Index into `Scene::objects`.
    pub index: usize,
    /// Distance from the impact point to the object's surface; 0 inside it.
    pub distance: f32,
    /// Share of the impact's damage this object takes, in 0..=1.
    pub share: f32,
}

/// Which breakables an impact at `point` reaches, and how much each one takes.
///
/// THE ROUTING STEP. `GameRuntime::apply_damage` is deliberately id-based --
/// resolving what was hit is the shooter's job -- which is exactly right for a
/// single authored wall and leaves nothing to call for a FRACTURED one, where
/// "what was hit" is a question about geometry rather than about aim. A shooter
/// that had to answer it would be re-implementing this per weapon.
///
/// Pure, and separate from the runtime, because everything interesting about it
/// is geometry and falloff: a test can pose an impact against a wall of chunks
/// without a physics scene, a renderer or a headset.
///
/// # Falloff
///
/// Linear from full at the point to nothing at `radius`, measured to each
/// object's ORIENTED box rather than its centre. Centre distance would mean a
/// long wall shot at one end takes damage as though it were hit in the middle,
/// and a rotated chunk would be measured against a box that is not the one on
/// screen.
///
/// `radius <= 0` is the direct-hit case -- a rifle round, not a blast. Only
/// objects the point is actually inside take anything, and they take it in
/// full. That is the common case, and it must not divide by zero.
///
/// # Damage is not divided between chunks
///
/// Each object in range takes `amount * share`, not a slice of one budget.
/// Dividing would make a wall's toughness depend on how finely it was
/// fractured: the same grenade against the same wall would do less to each
/// piece the more pieces there were, so an author improving the look of a
/// breach would silently be armouring it. Chunk count is a visual decision and
/// must stay one.
pub fn impact_targets(objects: &[GameObject], point: Vec3, radius: f32) -> Vec<ImpactTarget> {
    let mut out = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        // Objects with no `breakable` are skipped rather than reported: there
        // is nothing to record damage against, and returning them would invite
        // a caller to think an indestructible wall had absorbed a grenade.
        if object.breakable.is_none() {
            continue;
        }
        let distance = distance_to_oriented_box(
            object.cuboid.position,
            object.cuboid.rotation,
            object.cuboid.half_size,
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

    /// A change is reported when a THRESHOLD is crossed, not per hit. Thirty
    /// rounds into a wall is two events for the server to act on, not thirty.
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

    /// Damage accumulates rather than replacing, or a rifle would be as
    /// destructive as whatever hit hardest and no more.
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

    /// A mis-signed damage value must not silently rebuild a wall.
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

    /// Object ids are only unique within a scene, so carrying damage across a
    /// scene change would batter whatever happens to share a name next.
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

    /// What a late joiner needs to be caught up with.
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

    /// Two stages that look identical are still different stages, so passing
    /// between them fires a change. Comparing by value would swallow it.
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

    /// A wall fractured into chunks: a row of 1m boxes along x, each its own
    /// breakable, which is what the editor's Voronoi fracture produces.
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
        // One long wall rather than chunks. Shot at its far end, a centre
        // measurement would read 4m away and score nothing; the surface is
        // right there.
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
        // A thin panel turned 90 degrees. Measured axis-aligned it would still
        // seem to reach 2m along x; turned, it reaches 0.1m.
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

    /// One 6m wall divided into `n` chunks, so a finer number really means
    /// smaller pieces of the same wall rather than a longer wall.
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
        // The same blast against the same wall, chunked two ways. Damage is
        // per-object and NOT divided between them, so the piece that was hit
        // takes the full amount either way -- otherwise an author making a
        // breach look better would be quietly making the wall tougher.
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
        // The two fields disagreeing is an invisible wall, so `removed` wins.
        let b = BreakableDef {
            health: 10.0,
            stages: vec![DamageStage { at: 0.5, solid: true, removed: true, ..Default::default() }],
        };
        assert!(b.is_solid_at(1.0), "below the threshold, untouched");
        assert!(!b.is_solid_at(9.0));
    }
}
