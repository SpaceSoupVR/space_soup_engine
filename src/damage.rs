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

use crate::scene::{BreakableDef, GameObject};

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
                    },
                    DamageStage {
                        at: 0.9,
                        hidden_parts: vec!["intact".into(), "cracked".into()],
                        solid: false,
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
                    DamageStage { at: 0.3, hidden_parts: vec!["a".into()], solid: true },
                    DamageStage { at: 0.6, hidden_parts: vec!["a".into()], solid: true },
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
}
