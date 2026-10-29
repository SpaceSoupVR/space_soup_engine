//! THE GAME, FROM ITS LOGIC FILE. `logic/<scene>.logic.json` says who hunts the players
//! (`swarms`), what the players start with (`loadout`, `player_health`), which menus are up,
//! and the rules of the round (`rules`: on an event, do these actions). This module runs all of
//! it inside [`GameRuntime::update`], so the same file plays the same way in the editor's Play,
//! on the server and on the headset. Nothing about any one game is written here.
//!
//! The swarms are entities (`entity_core`): each NPC is an [`EntityRuntime`] walking the scene's
//! navmesh toward the nearest player, kept out of the walls by the physics world, told whether a
//! player is looking at it (for the ones that freeze when watched), and striking by its state's
//! `melee`. The host draws them from [`Npc`]: position, heading, state and animation time.

use std::collections::{HashMap, HashSet};

use entity_core::{EntityDef, EntityRuntime, Event, Movement, TickInput};
use glam::{Quat, Vec3};
use space_soup_protocol::PlayerId;

use crate::logic::{Action, LogicDef, Trigger, Zone};
use crate::rigid_physics_door::DoorDrive;
use crate::runtime::GameRuntime;
use crate::scene::{GameObject, Scene};

/// How far a player looks: within this many degrees of straight ahead counts as watching.
const WATCH_HALF_ANGLE_DEG: f32 = 32.0;
/// A creature's body for the walls: a sphere this wide at waist height.
const NPC_BODY_RADIUS_M: f32 = 0.35;
const NPC_WAIST_M: f32 = 0.9;
/// How often a hunting creature asks the navmesh for a fresh path, seconds.
const REPATH_S: f32 = 0.4;
/// A path corner this close is passed.
const CORNER_REACHED_M: f32 = 0.7;
/// A dead creature is drawn toppling for this long, then forgotten by the host.
pub const NPC_GONE_S: f32 = 2.5;
/// At most this many creatures in a scene, whatever the swarms ask.
const MAX_NPCS: usize = 64;
const EYE_ABOVE_FLOOR_M: f32 = 1.65;

/// ONE SWARM MEMBER, as the host draws it and as the rules count it.
#[derive(Clone, Debug)]
pub struct Npc {
    /// `<entity file stem>_<n>`, unique in the scene.
    pub id: String,
    /// The swarm it belongs to (an index into `LogicDef::swarms`).
    pub swarm: usize,
    /// Its entity file, from the game folder (`Swarm::entity`).
    pub entity: String,
    pub rt: EntityRuntime,
    pub health: f32,
    /// Seconds since it died, if it has.
    pub died_for: Option<f32>,
    path: Vec<Vec3>,
    path_age: f32,
}

impl Npc {
    #[must_use]
    pub fn alive(&self) -> bool {
        self.died_for.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    pub health: f32,
    pub full_health: f32,
}

impl PlayerState {
    #[must_use]
    pub fn dead(&self) -> bool {
        self.health <= 0.0
    }
}

/// Everything the logic file brought to life in this scene.
#[derive(Debug, Default)]
pub struct Gameplay {
    pub logic: LogicDef,
    /// The swarms' entity definitions, one per swarm (a swarm whose file failed to load has none).
    pub defs: Vec<Option<EntityDef>>,
    pub npcs: Vec<Npc>,
    pub players: HashMap<PlayerId, PlayerState>,
    /// The line the players are shown, and for how much longer.
    pub message: String,
    pub message_left: f32,
    /// The menus the logic has up right now.
    pub menus_open: Vec<String>,
    /// Seconds since the scene started.
    pub elapsed: f32,
    /// A `restart` action asked for the scene to be loaded again.
    pub restart: bool,
    /// Kills so far, for the host's counters.
    pub kills: u32,

    rule_fired: Vec<bool>,
    rule_inside: Vec<HashSet<PlayerId>>,
    rule_var_was: Vec<bool>,
    pending: Vec<(f32, Vec<Action>)>,
    was_all_dead: bool,
    any_dead_before: bool,
    deaths_seen: usize,
    started: bool,
    doors_held_open: HashSet<String>,
    doors_locked: HashMap<String, bool>,
    rng: u32,
    report_in: f32,
}

impl Gameplay {
    #[must_use]
    pub fn full_health(&self) -> f32 {
        self.logic.player_health.or_else(|| self.logic.swarms.first().map(|s| s.player_health)).unwrap_or(100.0)
    }

    #[must_use]
    pub fn alive(&self) -> usize {
        self.npcs.iter().filter(|n| n.alive()).count()
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 100_000) as f32 / 100_000.0
    }

    /// Whether `object` says the player at `head` is in it: the head, or the body a step
    /// below it, inside the object's box.
    fn inside(obj: &GameObject, head: Vec3) -> bool {
        let inv = obj.cuboid.rotation.inverse();
        let half = obj.cuboid.half_size + Vec3::splat(0.05);
        [head, head - Vec3::Y * 1.0].iter().any(|p| {
            let l = inv * (*p - obj.cuboid.position);
            l.x.abs() <= half.x && l.y.abs() <= half.y && l.z.abs() <= half.z
        })
    }
}

/// The length of an entity's animations, from its own moves (the only clips the engine knows
/// without opening the model). Anything else is taken as a second.
struct MoveClips<'a>(&'a EntityDef);

impl entity_core::Clips for MoveClips<'_> {
    fn duration(&self, anim: &str) -> Option<f32> {
        Some(self.0.moves.iter().find(|m| m.name == anim).map_or(1.0, |m| m.duration))
    }
}

fn spawn_pos(scene: &Scene, id: &str) -> Option<(Vec3, Quat)> {
    scene.find_object(id).filter(|o| o.spawn_point.is_some()).map(|o| (o.cuboid.position, o.cuboid.rotation))
}

/// Whether the ray from `origin` along `dir` meets a standing column at `base`, `height` tall
/// and `radius` wide: the distance along the ray and the point.
fn ray_column(origin: Vec3, dir: Vec3, base: Vec3, height: f32, radius: f32) -> Option<(f32, Vec3)> {
    let d = Vec3::new(dir.x, 0.0, dir.z);
    let o = Vec3::new(origin.x - base.x, 0.0, origin.z - base.z);
    let a = d.length_squared();
    if a < 1e-8 {
        return None;
    }
    let b = o.dot(d);
    let c = o.length_squared() - radius * radius;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let mut t = (-b - s) / a;
    if t < 0.0 {
        t = (-b + s) / a;
    }
    if t < 0.0 {
        return None;
    }
    let at = origin + dir * t;
    (at.y >= base.y && at.y <= base.y + height).then_some((t, at))
}

impl GameRuntime {
    /// Reads the scene's logic file and brings it to life: the swarms placed, the menus up,
    /// every player at `player_start` with full health.
    pub(crate) fn start_gameplay(&mut self) {
        let name = self.scene.name.clone();
        let logic = LogicDef::load(&self.game_dir, &name).unwrap_or_else(|_| LogicDef::new(&name));
        let mut gp = Gameplay { logic, rng: 0x2545_f491 ^ name.len() as u32, ..Default::default() };
        gp.menus_open = gp.logic.open_menus.clone();
        gp.rule_fired = vec![false; gp.logic.rules.len()];
        gp.rule_inside = (0..gp.logic.rules.len()).map(|_| HashSet::new()).collect();
        gp.rule_var_was = vec![false; gp.logic.rules.len()];

        let swarms = gp.logic.swarms.clone();
        for (si, swarm) in swarms.iter().enumerate() {
            let path = self.game_dir.join(&swarm.entity);
            let def = std::fs::read_to_string(&path).ok().and_then(|t| match EntityDef::from_json(&t) {
                Ok(d) => Some(d),
                Err(e) => {
                    log::warn!("gameplay: {} is not an entity file: {e}", path.display());
                    None
                }
            });
            if def.is_none() && !path.exists() {
                log::warn!("gameplay: swarm entity {} not found", path.display());
            }
            gp.defs.push(def);
            let Some(def) = gp.defs[si].as_ref().cloned() else { continue };
            let anchor = swarm
                .spawn
                .as_deref()
                .and_then(|id| spawn_pos(&self.scene, id))
                .or_else(|| self.scene.objects.iter().find(|o| o.spawn_point.is_some()).map(|o| (o.cuboid.position, o.cuboid.rotation)))
                .map_or(Vec3::ZERO, |(p, _)| p);
            let stem = std::path::Path::new(&swarm.entity)
                .file_name()
                .map(|s| s.to_string_lossy().trim_end_matches(".entity.json").trim_end_matches(".json").to_string())
                .unwrap_or_else(|| format!("swarm{si}"));
            let want = (swarm.count as usize).min(MAX_NPCS.saturating_sub(gp.npcs.len()));
            let mut tries = 0;
            let mut placed = 0;
            while placed < want && tries < want * 40 + 40 {
                tries += 1;
                let a = gp.rand() * std::f32::consts::TAU;
                let r = gp.rand() * swarm.spread_m.max(0.0);
                let mut at = anchor + Vec3::new(a.sin() * r, 0.0, a.cos() * r);
                if let Some(nav) = self.nav.as_ref() {
                    let Some(on) = nav.nearest(at) else { continue };
                    if Vec3::new(on.x - at.x, 0.0, on.z - at.z).length() > 2.0 {
                        continue;
                    }
                    at = on;
                }
                if let Some((hit, _)) = self.rigid_physics.raycast_down(at + Vec3::Y * 2.0, 6.0) {
                    at.y = hit.y;
                }
                let yaw = gp.rand() * std::f32::consts::TAU;
                let seed = (gp.rand() * 4.0e9) as u32;
                let rt = EntityRuntime::new(&def, at, yaw).with_seed(seed);
                gp.npcs.push(Npc { id: format!("{stem}_{placed}"), swarm: si, entity: swarm.entity.clone(), rt, health: swarm.health, died_for: None, path: Vec::new(), path_age: REPATH_S });
                placed += 1;
            }
            if placed < want {
                log::warn!("gameplay: placed {placed} of {want} {} (no floor found around the spawn)", def.name);
            }
        }
        let full = gp.full_health();
        for (player, state) in &mut gp.players {
            let _ = player;
            *state = PlayerState { health: full, full_health: full };
        }
        if let Some((pos, rot)) = gp.logic.player_start.as_deref().and_then(|id| spawn_pos(&self.scene, id)) {
            let yaw = Self::yaw_from_forward(rot * Vec3::NEG_Z);
            for l in self.locomotions.values_mut() {
                l.player_offset = pos;
                l.player_yaw = yaw;
            }
        }
        log::info!("gameplay: {}: {} creatures from {} swarm(s), {} rule(s), menus {:?}", name, gp.npcs.len(), gp.logic.swarms.len(), gp.logic.rules.len(), gp.menus_open);
        self.gameplay = gp;
    }

    pub fn gameplay(&self) -> &Gameplay {
        &self.gameplay
    }

    /// Every player put at the spawn point `spawn`, as the logic action would: for a host's
    /// own controls (the editor's test tools).
    pub fn teleport_players(&mut self, spawn: &str) {
        self.apply_actions(&[Action::TeleportPlayer { spawn: spawn.to_string() }]);
    }

    /// Sets a script variable, as the `set_var` action would: the keycards and switches the
    /// rules and locks read.
    pub fn set_var(&mut self, name: &str, value: &str) {
        self.apply_actions(&[Action::SetVar { name: name.to_string(), value: value.to_string() }]);
    }

    /// A player pressed `action` on `menu`: the logic's `menu_rules` for it follow.
    pub fn menu_action(&mut self, menu: &str, action: &str) {
        let acts = self.gameplay.logic.on_menu_action(menu, action);
        self.apply_actions(&acts);
    }

    /// A PLAYER'S SHOT at the creatures: the first one the ray meets within `range_m` takes the
    /// swarm's damage for `weapon` in the zone struck. The creature's id and whether it died.
    pub fn shoot_npcs(&mut self, origin: Vec3, dir: Vec3, range_m: f32, weapon: &str) -> Option<(String, Zone, bool)> {
        let mut best: Option<(usize, f32, Zone)> = None;
        for (i, n) in self.gameplay.npcs.iter().enumerate().filter(|(_, n)| n.alive()) {
            let Some(def) = self.gameplay.defs.get(n.swarm).and_then(|d| d.as_ref()) else { continue };
            let h = def.height_m;
            let Some((t, at)) = ray_column(origin, dir, n.rt.pos, h, (h * 0.3).max(0.25)) else { continue };
            if t > range_m || best.is_some_and(|(_, bt, _)| bt <= t) {
                continue;
            }
            let zone = if at.y > n.rt.pos.y + h * 0.72 { Zone::Head } else { Zone::Body };
            best = Some((i, t, zone));
        }
        let (i, t, zone) = best?;
        if let Some((hit, _)) = self.rigid_physics.raycast(origin, dir, t) {
            if (hit - origin).length() < t - 0.3 {
                return None;
            }
        }
        let swarm = self.gameplay.logic.swarms.get(self.gameplay.npcs[i].swarm)?.clone();
        let dmg = swarm.damage(weapon, zone)?;
        let n = &mut self.gameplay.npcs[i];
        n.health -= dmg;
        let killed = n.health <= 0.001;
        if killed {
            n.died_for = Some(0.0);
            self.gameplay.kills += 1;
        }
        Some((n.id.clone(), zone, killed))
    }

    /// One step of the game: the creatures move and strike, the rules fire, the doors are
    /// told what the game wants of them. Before the physics step, so what it asks is felt
    /// this frame.
    pub(crate) fn step_gameplay(&mut self, dt: f32) {
        let mut gp = std::mem::take(&mut self.gameplay);
        gp.elapsed += dt;
        gp.report_in -= dt;
        if gp.report_in <= 0.0 && !gp.npcs.is_empty() {
            gp.report_in = 10.0;
            let where_: Vec<String> = gp.npcs.iter().map(|n| format!("{} ({:.1}, {:.1}) {}{}", n.id, n.rt.pos.x, n.rt.pos.z, n.rt.state, if n.alive() { "" } else { " dead" })).collect();
            let players: Vec<String> = self.rigs.iter().map(|(p, r)| { let h = r.head().position; format!("{:?} at ({:.1}, {:.1}, {:.1}) hp {:.0}", p, h.x, h.y, h.z, gp.players.get(p).map_or(0.0, |s| s.health)) }).collect();
            log::info!("gameplay: {} | {}", players.join("; "), where_.join("; "));
        }
        if gp.message_left > 0.0 {
            gp.message_left -= dt;
            if gp.message_left <= 0.0 {
                gp.message.clear();
            }
        }
        let full = gp.full_health();
        let ids: Vec<PlayerId> = self.rigs.keys().copied().collect();
        for p in &ids {
            gp.players.entry(*p).or_insert(PlayerState { health: full, full_health: full });
        }
        gp.players.retain(|p, _| ids.contains(p));

        // The players as the creatures see them: eye, forward, and the floor under them.
        let heads: Vec<(PlayerId, Vec3, Vec3)> = ids
            .iter()
            .map(|&p| {
                let h = self.rigs[&p].head();
                (p, h.position, (h.rotation * Vec3::NEG_Z).normalize_or_zero())
            })
            .collect();
        let living: Vec<(PlayerId, Vec3, Vec3)> = heads.iter().copied().filter(|(p, _, _)| gp.players.get(p).is_none_or(|s| !s.dead())).collect();

        let defs = std::mem::take(&mut gp.defs);
        let mut damage: HashMap<PlayerId, f32> = HashMap::new();
        for n in &mut gp.npcs {
            let Some(def) = defs.get(n.swarm).and_then(|d| d.as_ref()) else { continue };
            if let Some(t) = n.died_for.as_mut() {
                *t += dt;
                continue;
            }
            let chest = n.rt.pos + Vec3::Y * (def.height_m * 0.6);
            let nearest = living.iter().min_by(|a, b| (a.1 - chest).length_squared().total_cmp(&(b.1 - chest).length_squared())).copied();
            let watched = living.iter().any(|(_, eye, fwd)| {
                let to = chest - *eye;
                let d = to.length();
                if d < 1e-3 {
                    return true;
                }
                let v = to / d;
                if fwd.dot(v) < WATCH_HALF_ANGLE_DEG.to_radians().cos() {
                    return false;
                }
                self.rigid_physics.raycast(*eye, v, d).is_none_or(|(hit, _)| (hit - *eye).length() >= d - 0.6)
            });
            let player_floor = nearest.map(|(_, eye, _)| Vec3::new(eye.x, eye.y - EYE_ABOVE_FLOOR_M, eye.z));
            let hunting = matches!(def.state(&n.rt.state).map(|s| &s.movement), Some(Movement::TowardPlayer { .. }));
            let mut steer = None;
            if let (Some(nav), Some(goal), true) = (self.nav.as_ref(), player_floor, hunting) {
                n.path_age += dt;
                if n.path_age >= REPATH_S || n.path.is_empty() {
                    n.path_age = 0.0;
                    n.path = nav.find_path(n.rt.pos, goal).unwrap_or_default();
                }
                while n.path.first().is_some_and(|c| Vec3::new(c.x - n.rt.pos.x, 0.0, c.z - n.rt.pos.z).length() < CORNER_REACHED_M) {
                    n.path.remove(0);
                }
                steer = n.path.first().copied();
            }
            n.rt.tick_with(def, dt, &TickInput { player: player_floor, steer, watched }, &MoveClips(def));
            // The floor under it, and out of the walls.
            if let Some((hit, _)) = self.rigid_physics.raycast_down(n.rt.pos + Vec3::Y * 1.5, 4.0) {
                n.rt.pos.y = hit.y;
            }
            let waist = n.rt.pos + Vec3::Y * NPC_WAIST_M;
            for c in self.rigid_physics.sphere_contacts(waist, NPC_BODY_RADIUS_M) {
                if !c.fixed {
                    continue;
                }
                let depth = NPC_BODY_RADIUS_M - c.distance;
                if depth <= 0.0 {
                    continue;
                }
                let nrm = if c.normal.length_squared() > 0.0 { Vec3::new(c.normal.x, 0.0, c.normal.z).normalize_or_zero() } else { Vec3::ZERO };
                n.rt.pos += nrm * depth;
            }
            for ev in std::mem::take(&mut n.rt.events) {
                match ev {
                    Event::Struck { damage: d } => {
                        if let Some((p, _, _)) = nearest {
                            *damage.entry(p).or_default() += d;
                        }
                    }
                    Event::Fired { hit: true, damage: d } => {
                        if let Some((p, _, _)) = nearest {
                            *damage.entry(p).or_default() += d;
                        }
                    }
                    _ => {}
                }
            }
        }
        gp.defs = defs;
        for (p, d) in damage {
            if let Some(s) = gp.players.get_mut(&p) {
                s.health = (s.health - d).max(0.0);
            }
        }

        // The rules.
        let mut fire: Vec<Vec<Action>> = Vec::new();
        let any_dead = gp.players.values().any(PlayerState::dead);
        let all_dead = !gp.npcs.is_empty() && gp.npcs.iter().all(|n| !n.alive());
        let deaths = gp.npcs.iter().filter(|n| !n.alive()).count();
        let rules = gp.logic.rules.clone();
        for (i, rule) in rules.iter().enumerate() {
            if rule.once && gp.rule_fired[i] {
                continue;
            }
            let fired = match &rule.on {
                Trigger::Start => !gp.started,
                Trigger::After { s } => gp.elapsed >= *s,
                Trigger::Enter { object } | Trigger::Exit { object } => {
                    let entering = matches!(rule.on, Trigger::Enter { .. });
                    let Some(obj) = self.scene.find_object(object) else { continue };
                    let mut hit = false;
                    for (p, head, _) in &heads {
                        let now = Gameplay::inside(obj, *head);
                        let was = gp.rule_inside[i].contains(p);
                        if now && !was {
                            gp.rule_inside[i].insert(*p);
                            hit |= entering;
                        } else if !now && was {
                            gp.rule_inside[i].remove(p);
                            hit |= !entering;
                        }
                    }
                    hit
                }
                Trigger::Var { name, is } => {
                    let now = self.script_host.var_string(name).as_deref() == Some(is.as_str());
                    let was = gp.rule_var_was[i];
                    gp.rule_var_was[i] = now;
                    now && !was
                }
                Trigger::PlayerDead => any_dead && !gp.any_dead_before,
                Trigger::AllDead => all_dead && !gp.was_all_dead,
                Trigger::NpcDead => deaths > gp.deaths_seen,
            };
            if fired {
                gp.rule_fired[i] = true;
                if rule.delay_s > 0.0 {
                    gp.pending.push((gp.elapsed + rule.delay_s, rule.then.clone()));
                } else {
                    fire.push(rule.then.clone());
                }
            }
        }
        gp.started = true;
        gp.any_dead_before = any_dead;
        gp.was_all_dead = all_dead;
        gp.deaths_seen = deaths;
        let due: Vec<Vec<Action>> = {
            let now = gp.elapsed;
            let (ready, later): (Vec<_>, Vec<_>) = std::mem::take(&mut gp.pending).into_iter().partition(|(t, _)| *t <= now);
            gp.pending = later;
            ready.into_iter().map(|(_, a)| a).collect()
        };

        // The doors: automatic ones open for whoever is near, locked ones stay shut.
        let npc_spots: Vec<Vec3> = gp.npcs.iter().filter(|n| n.alive()).map(|n| n.rt.pos + Vec3::Y * NPC_WAIST_M).collect();
        let mut drives: HashMap<String, DoorDrive> = HashMap::new();
        for (id, geom) in self.rigid_physics.doors() {
            let Some(def) = self.scene.find_object(id).and_then(|o| o.door.as_ref()) else { continue };
            let mut drive = DoorDrive::default();
            let locked = gp.doors_locked.get(id).copied().unwrap_or_else(|| {
                def.locked.as_ref().is_some_and(|l| self.script_host.var_string(&l.var).as_deref() != Some(l.is.as_str()))
            });
            drive.locked = locked;
            let centre = geom.closed_pos;
            if let Some(auto) = def.auto.as_ref() {
                let flat = |p: Vec3| Vec3::new(p.x - centre.x, 0.0, p.z - centre.z).length();
                let nearest = heads
                    .iter()
                    .filter(|_| auto.players)
                    .map(|(_, h, _)| *h)
                    .chain(npc_spots.iter().filter(|_| auto.npcs).copied())
                    .filter(|p| flat(*p) <= auto.range_m)
                    .min_by(|a, b| flat(*a).total_cmp(&flat(*b)));
                if let (Some(_), false, true) = (nearest, locked, geom.slides()) {
                    // A slider has no away: it runs to its open end, at its own speed.
                    drive.open_to = Some(if geom.slide > 0.0 { geom.hi } else { geom.lo });
                    drive.speed_rad_s = auto.speed_m_s;
                } else if let (Some(body), false) = (nearest, locked) {
                    // Which way is away from them: the free edge of a left-hung leaf moves to
                    // the leaf's -z when its angle grows, a right-hung one's to +z.
                    let local_z = (geom.closed_rot.inverse() * (body - centre)).z;
                    let grows_away = (def.hinge == crate::door::HingeSide::Left) == (local_z > 0.0);
                    let far = if grows_away { geom.hi } else { geom.lo };
                    let target = auto.open_deg.map_or(far, |d| if grows_away { d.to_radians().min(geom.hi) } else { (-d.to_radians()).max(geom.lo) });
                    drive.open_to = Some(target);
                    drive.speed_rad_s = auto.speed_deg_s.to_radians();
                }
            }
            if gp.doors_held_open.contains(id) && !locked {
                drive.open_to = Some(if geom.hi.abs() >= geom.lo.abs() { geom.hi } else { geom.lo });
                drive.speed_rad_s = drive.speed_rad_s.max(2.0);
            }
            if drive.locked || drive.open_to.is_some() {
                drives.insert(id.to_string(), drive);
            }
        }
        self.rigid_physics.set_door_drives(drives);

        self.gameplay = gp;
        for acts in fire.into_iter().chain(due) {
            self.apply_actions(&acts);
        }
    }

    pub(crate) fn apply_actions(&mut self, actions: &[Action]) {
        for a in actions {
            match a {
                Action::TeleportPlayer { spawn } => {
                    if let Some((pos, rot)) = spawn_pos(&self.scene, spawn) {
                        let yaw = Self::yaw_from_forward(rot * Vec3::NEG_Z);
                        for l in self.locomotions.values_mut() {
                            l.player_offset = pos;
                            l.player_yaw = yaw;
                        }
                    } else {
                        log::warn!("gameplay: no spawn point '{spawn}' to teleport to");
                    }
                }
                Action::ShowMenu { menu } => {
                    if !self.gameplay.menus_open.contains(menu) {
                        self.gameplay.menus_open.push(menu.clone());
                    }
                }
                Action::HideMenu { menu } => self.gameplay.menus_open.retain(|m| m != menu),
                Action::HideObject { object } => {
                    if let Some(o) = self.scene.find_object_mut(object) {
                        o.hidden = true;
                    }
                }
                Action::ShowObject { object } => {
                    if let Some(o) = self.scene.find_object_mut(object) {
                        o.hidden = false;
                    }
                }
                Action::PlaySound { object } => {
                    self.sound_play_requests.insert(object.clone());
                }
                Action::SwitchTeam => {}
                Action::ChangeScene { scene } => self.pending_scene_change = Some(scene.clone()),
                Action::Restart => self.gameplay.restart = true,
                Action::SetVar { name, value } => self.script_host.set_var(name, value),
                Action::Message { text, seconds } => {
                    self.gameplay.message = text.clone();
                    self.gameplay.message_left = *seconds;
                }
                Action::OpenDoor { object } => {
                    self.gameplay.doors_held_open.insert(object.clone());
                }
                Action::CloseDoor { object } => {
                    self.gameplay.doors_held_open.remove(object);
                }
                Action::LockDoor { object, locked } => {
                    self.gameplay.doors_locked.insert(object.clone(), *locked);
                }
                Action::Heal { amount } => {
                    for s in self.gameplay.players.values_mut() {
                        s.health = (s.health + amount).min(s.full_health);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_at_the_chest_hits_and_one_over_the_head_misses() {
        let base = Vec3::new(0.0, 0.0, -10.0);
        assert!(ray_column(Vec3::new(0.0, 1.6, 0.0), Vec3::NEG_Z, base, 2.0, 0.5).is_some());
        assert!(ray_column(Vec3::new(0.0, 3.0, 0.0), Vec3::NEG_Z, base, 2.0, 0.5).is_none());
        assert!(ray_column(Vec3::new(2.0, 1.6, 0.0), Vec3::NEG_Z, base, 2.0, 0.5).is_none());
    }

    #[test]
    fn inside_counts_the_head_and_the_body_below_it() {
        let mut o = GameObject::default();
        o.cuboid.position = Vec3::new(0.0, 0.5, 0.0);
        o.cuboid.half_size = Vec3::new(1.0, 0.5, 1.0);
        assert!(Gameplay::inside(&o, Vec3::new(0.2, 1.7, 0.2)), "a standing player's head is over the box, their body in it");
        assert!(!Gameplay::inside(&o, Vec3::new(3.0, 1.7, 0.0)));
    }
}
