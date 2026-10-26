use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {

    Full,

    Millimetre,

    Centimetre,
}

impl Precision {
    #[must_use]
    pub fn decimals(self) -> Option<i32> {
        match self {
            Precision::Full => None,
            Precision::Millimetre => Some(3),
            Precision::Centimetre => Some(2),
        }
    }

    #[must_use]
    pub fn round(self, v: f32) -> f32 {
        match self.decimals() {
            Some(d) => {
                let k = 10f32.powi(d);
                (v * k).round() / k
            }
            None => v,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FingerMode {

    None,

    Curl,

    All,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoseRule {
    pub enabled: bool,

    pub hz: f32,

    pub only_if_moved_cm: f32,

    pub only_if_turned_deg: f32,
    pub head: bool,
    pub left_hand: bool,
    pub right_hand: bool,
    pub floor: bool,
    pub flashlight: bool,
    pub precision: Precision,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputRule {
    pub enabled: bool,
    pub hz: f32,

    pub only_if_changed: bool,
    pub buttons_and_sticks: bool,
    pub movement: bool,
    pub teleport_target: bool,
    pub body_joints: bool,
    pub fingers: FingerMode,
    pub precision: Precision,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldRule {

    pub tick_hz: f32,

    pub only_changes: bool,

    pub only_within_m: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayersRule {

    pub hz: f32,
    pub only_within_m: f32,

    pub max_players: u32,
    pub flashlight: bool,
    pub precision: Precision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {

    Pose,

    Input,

    World,

    OtherPlayers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Head,
    LeftHand,
    RightHand,
    Floor,
    Flashlight,
    ButtonsAndSticks,
    Movement,
    TeleportTarget,
    BodyJoints,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "if", rename_all = "snake_case")]
pub enum Cond {

    StandingStill,

    Moving,
    HandsTracked,
    HandsNotTracked,

    PlayersMoreThan { n: u32 },
    PlayersFewerThan { n: u32 },

    FartherThan { m: f32 },
    InScene { scene: String },
    MenuOpen { menu: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Act {

    Skip,

    LimitRate { hz: f32 },
    SetPrecision { precision: Precision },

    Drop { field: Field },
    FingersAs { mode: FingerMode },

    OnlyChanges,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomRule {
    pub name: String,
    pub enabled: bool,
    pub message: Message,
    #[serde(default)]
    pub only_if: Vec<Cond>,
    #[serde(default)]
    pub then: Vec<Act>,
}

impl CustomRule {
    #[must_use]
    pub fn new(message: Message) -> Self {
        CustomRule { name: "New rule".into(), enabled: true, message, only_if: Vec::new(), then: Vec::new() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    pub standing_still: bool,
    pub hands_tracked: bool,
    pub players: u32,

    pub distance_m: Option<f32>,
    pub scene: String,
    pub open_menus: Vec<String>,
}

impl Cond {
    #[must_use]
    pub fn holds(&self, c: &Context) -> bool {
        match self {
            Cond::StandingStill => c.standing_still,
            Cond::Moving => !c.standing_still,
            Cond::HandsTracked => c.hands_tracked,
            Cond::HandsNotTracked => !c.hands_tracked,
            Cond::PlayersMoreThan { n } => c.players > *n,
            Cond::PlayersFewerThan { n } => c.players < *n,
            Cond::FartherThan { m } => c.distance_m.is_some_and(|d| d > *m),
            Cond::InScene { scene } => c.scene == *scene,
            Cond::MenuOpen { menu } => c.open_menus.contains(menu),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub skip: bool,

    pub hz_cap: Option<f32>,
    pub precision: Option<Precision>,
    pub dropped: Vec<Field>,
    pub fingers: Option<FingerMode>,
    pub only_changes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetRules {
    pub pose: PoseRule,
    pub input: InputRule,
    pub world: WorldRule,
    pub players: PlayersRule,

    #[serde(default)]
    pub rules: Vec<CustomRule>,
}

impl Default for NetRules {

    fn default() -> Self {
        NetRules {
            pose: PoseRule {
                enabled: true,
                hz: 0.0,
                only_if_moved_cm: 0.0,
                only_if_turned_deg: 0.0,
                head: true,
                left_hand: true,
                right_hand: true,
                floor: true,
                flashlight: true,
                precision: Precision::Full,
            },
            input: InputRule {
                enabled: true,
                hz: 0.0,
                only_if_changed: false,
                buttons_and_sticks: true,
                movement: true,
                teleport_target: true,
                body_joints: true,
                fingers: FingerMode::All,
                precision: Precision::Full,
            },
            world: WorldRule { tick_hz: 30.0, only_changes: false, only_within_m: 0.0 },
            players: PlayersRule { hz: 0.0, only_within_m: 0.0, max_players: 0, flashlight: true, precision: Precision::Full },
            rules: Vec::new(),
        }
    }
}

#[must_use]
pub fn rules_path(game_dir: &Path) -> PathBuf {
    game_dir.join("netrules.json")
}

impl NetRules {

    #[must_use]
    pub fn resolve(&self, message: Message, ctx: &Context) -> Resolved {
        let mut out = Resolved::default();
        for r in self.rules.iter().filter(|r| r.enabled && r.message == message && r.only_if.iter().all(|c| c.holds(ctx))) {
            for a in &r.then {
                match a {
                    Act::Skip => out.skip = true,
                    Act::LimitRate { hz } => out.hz_cap = Some(out.hz_cap.map_or(*hz, |h| h.min(*hz))),
                    Act::SetPrecision { precision } => out.precision = Some(*precision),
                    Act::Drop { field } => {
                        if !out.dropped.contains(field) {
                            out.dropped.push(*field);
                        }
                    }
                    Act::FingersAs { mode } => out.fingers = Some(*mode),
                    Act::OnlyChanges => out.only_changes = true,
                }
            }
        }
        out
    }

    #[must_use]
    pub fn with_rule_applied(&self, rule: &CustomRule) -> NetRules {
        let mut r = self.clone();
        r.rules.clear();
        for a in &rule.then {
            match (rule.message, a) {
                (Message::Pose, Act::Skip) => r.pose.enabled = false,
                (Message::Input, Act::Skip) => r.input.enabled = false,
                (Message::Pose, Act::LimitRate { hz }) => r.pose.hz = *hz,
                (Message::Input, Act::LimitRate { hz }) => r.input.hz = *hz,
                (Message::World, Act::LimitRate { hz }) => r.world.tick_hz = *hz,
                (Message::OtherPlayers, Act::LimitRate { hz }) => r.players.hz = *hz,
                (Message::Pose, Act::SetPrecision { precision }) => r.pose.precision = *precision,
                (Message::Input, Act::SetPrecision { precision }) => r.input.precision = *precision,
                (Message::OtherPlayers, Act::SetPrecision { precision }) => r.players.precision = *precision,
                (Message::Input, Act::FingersAs { mode }) => r.input.fingers = *mode,
                (Message::World, Act::OnlyChanges) => r.world.only_changes = true,
                (Message::Pose, Act::Drop { field }) => match field {
                    Field::Head => r.pose.head = false,
                    Field::LeftHand => r.pose.left_hand = false,
                    Field::RightHand => r.pose.right_hand = false,
                    Field::Floor => r.pose.floor = false,
                    Field::Flashlight => r.pose.flashlight = false,
                    _ => {}
                },
                (Message::Input, Act::Drop { field }) => match field {
                    Field::ButtonsAndSticks => r.input.buttons_and_sticks = false,
                    Field::Movement => r.input.movement = false,
                    Field::TeleportTarget => r.input.teleport_target = false,
                    Field::BodyJoints => r.input.body_joints = false,
                    _ => {}
                },
                (Message::OtherPlayers, Act::Drop { field: Field::Flashlight }) => r.players.flashlight = false,
                _ => {}
            }
        }
        r
    }

    #[must_use]
    pub fn load(game_dir: &Path) -> Self {
        std::fs::read_to_string(rules_path(game_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, game_dir: &Path) -> Result<PathBuf, String> {
        let path = rules_path(game_dir);
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

#[derive(Debug, Default)]
pub struct SendGate {
    last_t: Option<f32>,
    last_points: Vec<[f32; 3]>,
    last_yaw: f32,
}

impl SendGate {

    pub fn should_send(&mut self, rule: &PoseRule, t: f32, points: &[[f32; 3]], yaw_deg: f32) -> bool {
        if !rule.enabled {
            return false;
        }
        if rule.hz > 0.0 {
            if let Some(last) = self.last_t {
                if t - last < 1.0 / rule.hz {
                    return false;
                }
            }
        }
        let gated = rule.only_if_moved_cm > 0.0 || rule.only_if_turned_deg > 0.0;
        if gated && self.last_t.is_some() {
            let moved = points.iter().zip(&self.last_points).any(|(a, b)| {
                let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                rule.only_if_moved_cm > 0.0 && d * 100.0 >= rule.only_if_moved_cm
            }) || points.len() != self.last_points.len();
            let mut dy = (yaw_deg - self.last_yaw).abs() % 360.0;
            if dy > 180.0 {
                dy = 360.0 - dy;
            }
            let turned = rule.only_if_turned_deg > 0.0 && dy >= rule.only_if_turned_deg;
            if !moved && !turned {
                return false;
            }
        }
        self.last_t = Some(t);
        self.last_points = points.to_vec();
        self.last_yaw = yaw_deg;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> PoseRule {
        NetRules::default().pose
    }

    #[test]
    fn the_defaults_send_every_frame() {
        let mut g = SendGate::default();
        let r = rule();
        for i in 0..5 {
            assert!(g.should_send(&r, i as f32 / 72.0, &[[0.0; 3]], 0.0));
        }
    }

    #[test]
    fn a_rate_cap_spaces_the_sends() {
        let mut g = SendGate::default();
        let r = PoseRule { hz: 10.0, ..rule() };
        let sent: Vec<bool> = (0..20).map(|i| g.should_send(&r, i as f32 / 72.0, &[[0.0; 3]], 0.0)).collect();

        assert_eq!(sent.iter().filter(|s| **s).count(), 3);
    }

    #[test]
    fn standing_still_sends_nothing_after_the_first() {
        let mut g = SendGate::default();
        let r = PoseRule { only_if_moved_cm: 1.0, only_if_turned_deg: 2.0, ..rule() };
        assert!(g.should_send(&r, 0.0, &[[0.0; 3], [1.0, 1.0, 0.0]], 0.0));
        for i in 1..50 {
            assert!(!g.should_send(&r, i as f32 / 72.0, &[[0.0; 3], [1.0, 1.0, 0.0]], 0.0));
        }
    }

    #[test]
    fn movement_or_turning_sends_again() {
        let mut g = SendGate::default();
        let r = PoseRule { only_if_moved_cm: 1.0, only_if_turned_deg: 2.0, ..rule() };
        assert!(g.should_send(&r, 0.0, &[[0.0; 3]], 0.0));
        assert!(!g.should_send(&r, 0.1, &[[0.004, 0.0, 0.0]], 0.0));
        assert!(g.should_send(&r, 0.2, &[[0.02, 0.0, 0.0]], 0.0));
        assert!(g.should_send(&r, 0.3, &[[0.02, 0.0, 0.0]], 5.0));

        assert!(g.should_send(&r, 0.4, &[[0.02, 0.0, 0.0]], 359.0));
        assert!(g.should_send(&r, 0.5, &[[0.02, 0.0, 0.0]], 1.0));
        assert!(!g.should_send(&r, 0.6, &[[0.02, 0.0, 0.0]], 2.0));
    }

    #[test]
    fn disabled_means_never() {
        let mut g = SendGate::default();
        let r = PoseRule { enabled: false, ..rule() };
        assert!(!g.should_send(&r, 0.0, &[[0.0; 3]], 0.0));
    }

    #[test]
    fn precision_rounds() {
        assert_eq!(Precision::Centimetre.round(1.23456), 1.23);
        assert_eq!(Precision::Millimetre.round(1.23456), 1.235);
        assert_eq!(Precision::Full.round(1.23456), 1.23456);
    }

    #[test]
    fn custom_rules_apply_only_when_their_conditions_hold() {
        let mut r = NetRules::default();
        r.rules.push(CustomRule {
            name: "Idle players cost nothing".into(),
            enabled: true,
            message: Message::Pose,
            only_if: vec![Cond::StandingStill, Cond::PlayersMoreThan { n: 4 }],
            then: vec![Act::LimitRate { hz: 5.0 }, Act::Drop { field: Field::Flashlight }],
        });
        let still = Context { standing_still: true, players: 8, ..Default::default() };
        let got = r.resolve(Message::Pose, &still);
        assert_eq!(got.hz_cap, Some(5.0));
        assert_eq!(got.dropped, vec![Field::Flashlight]);

        assert_eq!(r.resolve(Message::Pose, &Context { players: 8, ..Default::default() }), Resolved::default());
        assert_eq!(r.resolve(Message::Pose, &Context { standing_still: true, players: 2, ..Default::default() }), Resolved::default());
        assert_eq!(r.resolve(Message::World, &still), Resolved::default());
    }

    #[test]
    fn disabled_rules_do_nothing_and_the_slowest_rate_wins() {
        let mut r = NetRules::default();
        let mut a = CustomRule::new(Message::World);
        a.then = vec![Act::LimitRate { hz: 20.0 }];
        let mut b = CustomRule::new(Message::World);
        b.then = vec![Act::LimitRate { hz: 10.0 }];
        let mut c = CustomRule::new(Message::World);
        c.enabled = false;
        c.then = vec![Act::Skip];
        r.rules = vec![a, b, c];
        let got = r.resolve(Message::World, &Context::default());
        assert_eq!(got.hz_cap, Some(10.0));
        assert!(!got.skip);
    }

    #[test]
    fn a_rule_can_be_folded_into_the_base_to_see_its_worth() {
        let r = NetRules::default();
        let mut rule = CustomRule::new(Message::Input);
        rule.then = vec![Act::FingersAs { mode: FingerMode::Curl }, Act::Drop { field: Field::BodyJoints }];
        let applied = r.with_rule_applied(&rule);
        assert_eq!(applied.input.fingers, FingerMode::Curl);
        assert!(!applied.input.body_joints);
        assert!(applied.pose.head);
    }

    #[test]
    fn the_file_round_trips() {
        let mut r = NetRules::default();
        r.input.fingers = FingerMode::Curl;
        r.world.only_changes = true;
        let text = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<NetRules>(&text).unwrap(), r);
    }
}
