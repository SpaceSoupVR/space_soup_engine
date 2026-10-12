//! Game rules authored in SSStudio's Logic tab: where the player starts,
//! what a menu button does, and how much punishment an NPC takes from each
//! weapon. One `game/logic/<scene>.logic.json` per scene.
//!
//! Data and decisions only: the host (headset game, editor) feeds it events
//! -- "this action was pressed", "this bullet hit that NPC's head" -- and
//! gets back what to do. Nothing here touches physics or rendering.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where on an NPC a hit landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Body,
    Head,
}

/// One thing the game does in response to a rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {
    /// Put the player at a spawn point (an object id in the scene).
    TeleportPlayer { spawn: String },
    ShowMenu { menu: String },
    HideMenu { menu: String },
    /// Remove an object (an NPC that died, a door that opened...).
    HideObject { object: String },
}

/// "When this menu's button with this action name is pressed, do these."
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MenuRule {
    pub menu: String,
    pub action: String,
    #[serde(default)]
    pub then: Vec<Action>,
}

/// How many hits of one weapon kill an NPC, per zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeaponHits {
    /// The weapon's prop name (`pistol`, `m4a1`).
    pub weapon: String,
    pub body_shots: u32,
    pub head_shots: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcRule {
    /// The scene object that is this NPC.
    pub object: String,
    #[serde(default = "default_health")]
    pub health: f32,
    #[serde(default)]
    pub hits: Vec<WeaponHits>,
    /// What happens when its health runs out.
    #[serde(default)]
    pub on_death: Vec<Action>,
}

fn default_health() -> f32 {
    100.0
}

impl NpcRule {
    /// Damage one hit from `weapon` to `zone` deals, or `None` for a weapon
    /// the rule doesn't list (it can't hurt this NPC). A zone that needs 0
    /// shots is treated as "can't hurt".
    pub fn damage(&self, weapon: &str, zone: Zone) -> Option<f32> {
        let w = self.hits.iter().find(|h| h.weapon == weapon)?;
        let shots = match zone {
            Zone::Body => w.body_shots,
            Zone::Head => w.head_shots,
        };
        (shots > 0).then(|| self.health / shots as f32)
    }
}

/// A crowd of one entity (`.entity.json`) roaming the scene and hunting the
/// player. Once every one is dead the round restarts at `player_start`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swarm {
    /// The entity file, relative to the game folder.
    pub entity: String,
    #[serde(default = "default_swarm_count")]
    pub count: u32,
    /// How far from the player's terrain spawn they are scattered, metres.
    #[serde(default = "default_swarm_spread")]
    pub spread_m: f32,
    #[serde(default = "default_health")]
    pub health: f32,
    /// What each weapon does to one of them (same meaning as `NpcRule::hits`).
    #[serde(default)]
    pub hits: Vec<WeaponHits>,
    /// What one landed attack takes off the player, and how much they have.
    #[serde(default = "default_attack_damage")]
    pub attack_damage: f32,
    #[serde(default = "default_player_health")]
    pub player_health: f32,
}

fn default_attack_damage() -> f32 {
    15.0
}

fn default_player_health() -> f32 {
    50.0
}

fn default_swarm_count() -> u32 {
    30
}

fn default_swarm_spread() -> f32 {
    150.0
}

impl Swarm {
    /// Damage one hit from `weapon` to `zone` deals (see `NpcRule::damage`).
    pub fn damage(&self, weapon: &str, zone: Zone) -> Option<f32> {
        NpcRule { object: String::new(), health: self.health, hits: self.hits.clone(), on_death: Vec::new() }.damage(weapon, zone)
    }
}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LogicDef {
    pub scene: String,
    /// Spawn point object the player starts at when the scene loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_start: Option<String>,
    /// Menus on screen when the scene loads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_menus: Vec<String>,
    #[serde(default)]
    pub menu_rules: Vec<MenuRule>,
    #[serde(default)]
    pub npcs: Vec<NpcRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub swarms: Vec<Swarm>,
}

pub fn logic_path(game_dir: &Path, scene: &str) -> PathBuf {
    game_dir.join("logic").join(format!("{scene}.logic.json"))
}

impl LogicDef {
    pub fn new(scene: &str) -> Self {
        LogicDef { scene: scene.into(), ..Default::default() }
    }

    pub fn load(game_dir: &Path, scene: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(logic_path(game_dir, scene)).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn save(&self, game_dir: &Path) -> Result<PathBuf, String> {
        let path = logic_path(game_dir, &self.scene);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// What a pressed menu button does.
    pub fn on_menu_action(&self, menu: &str, action: &str) -> Vec<Action> {
        self.menu_rules
            .iter()
            .filter(|r| r.menu == menu && r.action == action)
            .flat_map(|r| r.then.iter().cloned())
            .collect()
    }

    pub fn npc(&self, object: &str) -> Option<&NpcRule> {
        self.npcs.iter().find(|n| n.object == object)
    }
}

/// What a hit did.
#[derive(Clone, Debug, PartialEq)]
pub enum HitOutcome {
    /// This weapon can't hurt this NPC (or the NPC has no rule).
    Ignored,
    Hurt { health_left: f32 },
    /// Health ran out on this hit; carry out `on_death`.
    Killed { then: Vec<Action> },
    /// Already dead: nothing more happens.
    AlreadyDead,
}

/// Live health of every NPC in the running scene.
#[derive(Debug, Default)]
pub struct NpcHealth {
    left: HashMap<String, f32>,
}

impl NpcHealth {
    /// Apply one hit to `object` using the scene's rules.
    pub fn hit(&mut self, logic: &LogicDef, object: &str, weapon: &str, zone: Zone) -> HitOutcome {
        let Some(rule) = logic.npc(object) else { return HitOutcome::Ignored };
        let Some(dmg) = rule.damage(weapon, zone) else { return HitOutcome::Ignored };
        let left = self.left.entry(object.to_string()).or_insert(rule.health);
        if *left <= 0.0 {
            return HitOutcome::AlreadyDead;
        }
        // Rounded to a thousandth so five 20-point hits land on exactly zero.
        *left = ((*left - dmg) * 1000.0).round() / 1000.0;
        if *left <= 0.0 {
            *left = 0.0;
            HitOutcome::Killed { then: rule.on_death.clone() }
        } else {
            HitOutcome::Hurt { health_left: *left }
        }
    }

    pub fn is_dead(&self, object: &str) -> bool {
        self.left.get(object).is_some_and(|h| *h <= 0.0)
    }

    pub fn reset(&mut self) {
        self.left.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soldier() -> LogicDef {
        LogicDef {
            scene: "warehouse".into(),
            player_start: Some("spawn_point_tent".into()),
            open_menus: vec!["start".into()],
            menu_rules: vec![MenuRule {
                menu: "start".into(),
                action: "start_game".into(),
                then: vec![
                    Action::HideMenu { menu: "start".into() },
                    Action::TeleportPlayer { spawn: "spawn_point_terrain".into() },
                ],
            }],
            npcs: vec![NpcRule {
                object: "soldier".into(),
                health: 100.0,
                hits: vec![
                    WeaponHits { weapon: "pistol".into(), body_shots: 5, head_shots: 1 },
                    WeaponHits { weapon: "m4a1".into(), body_shots: 2, head_shots: 1 },
                ],
                on_death: vec![Action::HideObject { object: "soldier".into() }],
            }],
            swarms: Vec::new(),
        }
    }

    #[test]
    fn the_start_button_leaves_the_tent() {
        let l = soldier();
        let acts = l.on_menu_action("start", "start_game");
        assert_eq!(acts[1], Action::TeleportPlayer { spawn: "spawn_point_terrain".into() });
        assert!(l.on_menu_action("start", "nope").is_empty());
        assert!(l.on_menu_action("other", "start_game").is_empty());
    }

    #[test]
    fn five_pistol_body_shots_kill() {
        let l = soldier();
        let mut hp = NpcHealth::default();
        for _ in 0..4 {
            assert!(matches!(hp.hit(&l, "soldier", "pistol", Zone::Body), HitOutcome::Hurt { .. }));
        }
        assert!(matches!(hp.hit(&l, "soldier", "pistol", Zone::Body), HitOutcome::Killed { .. }));
        assert!(hp.is_dead("soldier"));
        assert_eq!(hp.hit(&l, "soldier", "pistol", Zone::Body), HitOutcome::AlreadyDead);
    }

    #[test]
    fn one_pistol_headshot_kills() {
        let l = soldier();
        let mut hp = NpcHealth::default();
        assert!(matches!(hp.hit(&l, "soldier", "pistol", Zone::Head), HitOutcome::Killed { .. }));
    }

    #[test]
    fn two_rifle_body_shots_or_one_headshot_kill() {
        let l = soldier();
        let mut hp = NpcHealth::default();
        assert!(matches!(hp.hit(&l, "soldier", "m4a1", Zone::Body), HitOutcome::Hurt { .. }));
        assert!(matches!(hp.hit(&l, "soldier", "m4a1", Zone::Body), HitOutcome::Killed { .. }));
        let mut hp = NpcHealth::default();
        assert!(matches!(hp.hit(&l, "soldier", "m4a1", Zone::Head), HitOutcome::Killed { .. }));
    }

    #[test]
    fn mixed_weapons_add_up() {
        let l = soldier();
        let mut hp = NpcHealth::default();
        // Rifle body = 50, pistol body = 20 each: 50 + 20 + 20 + 20 = 110.
        hp.hit(&l, "soldier", "m4a1", Zone::Body);
        hp.hit(&l, "soldier", "pistol", Zone::Body);
        hp.hit(&l, "soldier", "pistol", Zone::Body);
        assert!(matches!(hp.hit(&l, "soldier", "pistol", Zone::Body), HitOutcome::Killed { .. }));
    }

    #[test]
    fn unknown_weapons_and_npcs_are_ignored() {
        let l = soldier();
        let mut hp = NpcHealth::default();
        assert_eq!(hp.hit(&l, "soldier", "knife", Zone::Body), HitOutcome::Ignored);
        assert_eq!(hp.hit(&l, "tree", "pistol", Zone::Body), HitOutcome::Ignored);
    }

    #[test]
    fn the_file_round_trips() {
        let l = soldier();
        let text = serde_json::to_string_pretty(&l).unwrap();
        assert_eq!(serde_json::from_str::<LogicDef>(&text).unwrap(), l);
    }
}
