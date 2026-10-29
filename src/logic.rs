use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Body,
    Head,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {

    TeleportPlayer { spawn: String },
    ShowMenu { menu: String },
    HideMenu { menu: String },

    HideObject { object: String },

    /// Plays the scene's audio object `object` (an object whose sound names a
    /// saved sound) where it stands.
    PlaySound { object: String },

    SwitchTeam,

    /// Loads another scene of the game, every player placed at its start.
    ChangeScene { scene: String },
    /// Loads this scene again from its file: the round over, everything as it was.
    Restart,
    /// Sets a script variable (the same ones `trigger_volume` and scripts read).
    SetVar { name: String, value: String },
    /// Shows an object hidden earlier (or authored hidden).
    ShowObject { object: String },
    /// A line for the players to read (the host shows it for `seconds`).
    Message { text: String, #[serde(default = "default_message_seconds")] seconds: f32 },
    /// Swings a door open and holds it there until `lock_door` or a `close_door`.
    OpenDoor { object: String },
    CloseDoor { object: String },
    /// Bolts a door shut (or frees it), whatever its own `locked` says.
    LockDoor { object: String, #[serde(default = "default_true")] locked: bool },
    /// Gives every player back `amount` health, up to their full health.
    Heal { amount: f32 },
}

fn default_message_seconds() -> f32 {
    6.0
}

/// WHAT A RULE WAITS FOR. `event` names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Trigger {
    /// The scene has just started.
    Start,
    /// `s` seconds into the scene.
    After { s: f32 },
    /// A player steps into the object's box.
    Enter { object: String },
    /// A player steps out of the object's box.
    Exit { object: String },
    /// A script variable takes the value `is` (set by a `trigger_volume`, a script or `set_var`).
    Var { name: String, #[serde(default = "default_var_is")] is: String },
    /// A player's health reaches zero.
    PlayerDead,
    /// Every swarm member is dead (and there was at least one).
    AllDead,
    /// Any swarm member dies.
    NpcDead,
}

fn default_var_is() -> String {
    "1".into()
}

/// A RULE: when `on` happens, `then` follows, `delay_s` later. `once` (the default) retires the
/// rule after its first firing; false fires it every time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub on: Trigger,
    #[serde(default)]
    pub then: Vec<Action>,
    #[serde(default = "default_true")]
    pub once: bool,
    #[serde(default)]
    pub delay_s: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TeamDef {
    pub name: String,
    pub spawns: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScoreRow {
    pub player: String,
    pub team: String,
    pub kills: u32,
    pub deaths: u32,
    pub ping_ms: u32,
}

impl ScoreRow {

    #[must_use]
    pub fn kd(&self) -> f32 {
        if self.deaths == 0 { self.kills as f32 } else { self.kills as f32 / self.deaths as f32 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MenuRule {
    pub menu: String,
    pub action: String,
    #[serde(default)]
    pub then: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeaponHits {

    pub weapon: String,
    pub body_shots: u32,
    pub head_shots: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcRule {

    pub object: String,
    #[serde(default = "default_health")]
    pub health: f32,
    #[serde(default)]
    pub hits: Vec<WeaponHits>,

    #[serde(default)]
    pub on_death: Vec<Action>,
}

fn default_health() -> f32 {
    100.0
}

fn default_true() -> bool {
    true
}

impl NpcRule {

    #[must_use]
    pub fn damage(&self, weapon: &str, zone: Zone) -> Option<f32> {
        let w = self.hits.iter().find(|h| h.weapon == weapon)?;
        let shots = match zone {
            Zone::Body => w.body_shots,
            Zone::Head => w.head_shots,
        };
        (shots > 0).then(|| self.health / shots as f32)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swarm {

    pub entity: String,
    #[serde(default = "default_swarm_count")]
    pub count: u32,

    #[serde(default = "default_swarm_spread")]
    pub spread_m: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn: Option<String>,
    #[serde(default = "default_health")]
    pub health: f32,

    #[serde(default)]
    pub hits: Vec<WeaponHits>,

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

    #[must_use]
    pub fn damage(&self, weapon: &str, zone: Zone) -> Option<f32> {
        NpcRule { object: String::new(), health: self.health, hits: self.hits.clone(), on_death: Vec::new() }.damage(weapon, zone)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LogicDef {
    pub scene: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_start: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_menus: Vec<String>,
    #[serde(default)]
    pub menu_rules: Vec<MenuRule>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TeamDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_button: Option<String>,

    #[serde(default = "default_true")]
    pub hand_menu: bool,
    #[serde(default)]
    pub npcs: Vec<NpcRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub swarms: Vec<Swarm>,

    /// The game's rules: what happens on what (`Rule`). This is where a round, a win and a
    /// loss are written, so none of it lives in code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Each player's health at the start. Absent: the first swarm's `player_health`, else 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_health: Option<f32>,
    /// A prop every player starts holding, by its `.prop.json` path from the game folder. Its
    /// name is the weapon the swarms' `hits` are counted in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loadout: Option<String>,
}

#[must_use]
pub fn logic_path(game_dir: &Path, scene: &str) -> PathBuf {
    game_dir.join("logic").join(format!("{scene}.logic.json"))
}

impl LogicDef {
    #[must_use]
    pub fn new(scene: &str) -> Self {
        LogicDef { scene: scene.into(), ..Default::default() }
    }

    pub fn load(game_dir: &Path, scene: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(logic_path(game_dir, scene)).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn next_team(&self, current: &str) -> Option<&TeamDef> {
        if self.teams.is_empty() {
            return None;
        }
        let i = self.teams.iter().position(|t| t.name == current).map_or(0, |i| (i + 1) % self.teams.len());
        self.teams.get(i)
    }

    #[must_use]
    pub fn team_for_new_player<'a>(&'a self, counts: &HashMap<String, usize>) -> Option<&'a TeamDef> {
        self.teams.iter().min_by_key(|t| counts.get(&t.name).copied().unwrap_or(0))
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

    #[must_use]
    pub fn on_menu_action(&self, menu: &str, action: &str) -> Vec<Action> {
        self.menu_rules
            .iter()
            .filter(|r| r.menu == menu && r.action == action)
            .flat_map(|r| r.then.iter().cloned())
            .collect()
    }

    #[must_use]
    pub fn npc(&self, object: &str) -> Option<&NpcRule> {
        self.npcs.iter().find(|n| n.object == object)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HitOutcome {

    Ignored,
    Hurt { health_left: f32 },

    Killed { then: Vec<Action> },

    AlreadyDead,
}

#[derive(Debug, Default)]
pub struct NpcHealth {
    left: HashMap<String, f32>,
}

impl NpcHealth {

    pub fn hit(&mut self, logic: &LogicDef, object: &str, weapon: &str, zone: Zone) -> HitOutcome {
        let Some(rule) = logic.npc(object) else { return HitOutcome::Ignored };
        let Some(dmg) = rule.damage(weapon, zone) else { return HitOutcome::Ignored };
        let left = self.left.entry(object.to_string()).or_insert(rule.health);
        if *left <= 0.0 {
            return HitOutcome::AlreadyDead;
        }

        *left = ((*left - dmg) * 1000.0).round() / 1000.0;
        if *left <= 0.0 {
            *left = 0.0;
            HitOutcome::Killed { then: rule.on_death.clone() }
        } else {
            HitOutcome::Hurt { health_left: *left }
        }
    }

    #[must_use]
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
            teams: Vec::new(),
            menu_button: None,
            hand_menu: true,
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
            rules: Vec::new(),
            player_health: None,
            loadout: None,
        }
    }

    #[test]
    fn rules_and_their_triggers_read_from_json() {
        let text = r#"{"scene":"x","rules":[
            {"on":{"event":"enter","object":"exit_zone"},"then":[{"do":"message","text":"You escaped"},{"do":"change_scene","scene":"next"}]},
            {"on":{"event":"player_dead"},"then":[{"do":"restart"}],"delay_s":2.5},
            {"on":{"event":"var","name":"keycard"},"then":[{"do":"lock_door","object":"exit_door","locked":false}]},
            {"on":{"event":"all_dead"},"once":false,"then":[{"do":"heal","amount":50}]}
        ]}"#;
        let l: LogicDef = serde_json::from_str(text).unwrap();
        assert_eq!(l.rules.len(), 4);
        assert_eq!(l.rules[0].on, Trigger::Enter { object: "exit_zone".into() });
        assert_eq!(l.rules[0].then[0], Action::Message { text: "You escaped".into(), seconds: 6.0 });
        assert!(l.rules[0].once && l.rules[1].delay_s == 2.5 && !l.rules[3].once);
        assert_eq!(l.rules[2].on, Trigger::Var { name: "keycard".into(), is: "1".into() });
        assert_eq!(l.rules[2].then[0], Action::LockDoor { object: "exit_door".into(), locked: false });
        let back: LogicDef = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
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

#[cfg(test)]
mod team_tests {
    use super::*;

    fn two_teams() -> LogicDef {
        LogicDef {
            scene: "t".into(),
            teams: vec![
                TeamDef { name: "north".into(), spawns: vec!["spawn_north_1".into(), "spawn_north_2".into()] },
                TeamDef { name: "south".into(), spawns: vec!["spawn_south_1".into()] },
            ],
            menu_button: Some("team".into()),
            hand_menu: false,
            ..Default::default()
        }
    }

    #[test]
    fn switching_cycles_through_the_teams() {
        let d = two_teams();
        assert_eq!(d.next_team("north").unwrap().name, "south");
        assert_eq!(d.next_team("south").unwrap().name, "north");
        assert_eq!(d.next_team("nobody").unwrap().name, "north");
        assert!(LogicDef::default().next_team("north").is_none());
    }

    #[test]
    fn a_new_player_joins_the_smaller_team() {
        let d = two_teams();
        let mut counts = HashMap::new();
        counts.insert("north".to_string(), 3);
        counts.insert("south".to_string(), 1);
        assert_eq!(d.team_for_new_player(&counts).unwrap().name, "south");
        assert_eq!(d.team_for_new_player(&HashMap::new()).unwrap().name, "north");
    }

    #[test]
    fn teams_and_the_menu_button_round_trip_and_old_files_still_load() {
        let d = two_teams();
        let text = serde_json::to_string(&d).unwrap();
        let back: LogicDef = serde_json::from_str(&text).unwrap();
        assert_eq!(back, d);
        let old: LogicDef = serde_json::from_str(r#"{"scene":"w","menu_rules":[]}"#).unwrap();
        assert!(old.teams.is_empty() && old.menu_button.is_none() && old.hand_menu);
        let act: Action = serde_json::from_str(r#"{"do":"switch_team"}"#).unwrap();
        assert_eq!(act, Action::SwitchTeam);
    }

    #[test]
    fn kd_counts_kills_alone_until_the_first_death() {
        let r = ScoreRow { kills: 3, deaths: 0, ..Default::default() };
        assert_eq!(r.kd(), 3.0);
        let r = ScoreRow { kills: 3, deaths: 2, ..Default::default() };
        assert_eq!(r.kd(), 1.5);
    }
}
