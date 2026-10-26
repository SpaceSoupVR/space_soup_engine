use std::collections::HashMap;
use std::path::Path;

use glam::{Quat, Vec3};
use physx::prelude::*;

use physx::scene::Scene as _;

use physx::height_field::HeightField;
use physx::scene::SceneFlags;
use physx::triangle_mesh::TriangleMesh;

use space_soup_protocol::PlayerId;

use crate::events::Hand;
use crate::scene::Scene;

pub(crate) fn to_px_transform(pos: Vec3, rot: Quat) -> PxTransform {
    PxTransform::from_translation_rotation(
        &PxVec3::new(pos.x, pos.y, pos.z),
        &PxQuat::new(rot.x, rot.y, rot.z, rot.w),
    )
}

pub(crate) fn from_px_transform(t: &PxTransform) -> (Vec3, Quat) {
    let p = t.translation();
    let r = t.rotation();
    (
        Vec3::new(p.x(), p.y(), p.z()),
        Quat::from_xyzw(r.x(), r.y(), r.z(), r.w()),
    )
}

pub(crate) fn to_px_vec3(v: [f32; 3]) -> PxVec3 {
    PxVec3::new(v[0], v[1], v[2])
}

pub(crate) const DEFAULT_DENSITY: f32 = 500.0;

/// The mass physics gives a box of `half_size` with none authored: what a
/// client that is told only a body's size takes it to weigh.
#[must_use]
pub fn default_mass(half_size: Vec3) -> f32 {
    calculated_mass(half_size, DEFAULT_DENSITY)
}

pub(crate) fn calculated_mass(half_size: Vec3, density: f32) -> f32 {
    let volume = (half_size.x * 2.0) * (half_size.y * 2.0) * (half_size.z * 2.0);
    (volume * density).max(0.001)
}

pub(crate) fn to_raw_transform(t: PxTransform) -> physx_sys::PxTransform {
    t.into()
}

pub(crate) type PxMaterial = physx::material::PxMaterial<()>;
pub(crate) type PxShape = physx::shape::PxShape<(), PxMaterial>;
pub(crate) type PxArticulationLink = physx::articulation_link::PxArticulationLink<(), PxShape>;
pub(crate) type PxRigidStatic = physx::rigid_static::PxRigidStatic<(), PxShape>;
pub(crate) type PxRigidDynamic = physx::rigid_dynamic::PxRigidDynamic<(), PxShape>;
pub(crate) type PxArticulationReducedCoordinate =
    physx::articulation_reduced_coordinate::PxArticulationReducedCoordinate<(), PxArticulationLink>;

pub(crate) type PxScene = physx::scene::PxScene<
    u64,
    PxArticulationLink,
    PxRigidStatic,
    PxRigidDynamic,
    PxArticulationReducedCoordinate,
    OnCollision,
    OnTrigger,
    OnConstraintBreak,
    OnWakeSleep,
    OnAdvance,
>;
pub(crate) type PxFoundation = PhysicsFoundation<physx::foundation::DefaultAllocator, PxShape>;

pub(crate) struct OnCollision;
impl CollisionCallback for OnCollision {
    fn on_collision(
        &mut self,
        _header: &physx_sys::PxContactPairHeader,
        _pairs: &[physx_sys::PxContactPair],
    ) {
    }
}
pub(crate) struct OnTrigger;
impl TriggerCallback for OnTrigger {
    fn on_trigger(&mut self, _pairs: &[physx_sys::PxTriggerPair]) {}
}
pub(crate) struct OnConstraintBreak;
impl ConstraintBreakCallback for OnConstraintBreak {
    fn on_constraint_break(&mut self, _constraints: &[physx_sys::PxConstraintInfo]) {}
}
pub(crate) struct OnWakeSleep;
impl WakeSleepCallback<PxArticulationLink, PxRigidStatic, PxRigidDynamic> for OnWakeSleep {
    fn on_wake_sleep(
        &mut self,
        _actors: &[&physx::actor::ActorMap<PxArticulationLink, PxRigidStatic, PxRigidDynamic>],
        _is_waking: bool,
    ) {
    }
}
pub(crate) struct OnAdvance;
impl AdvanceCallback<PxArticulationLink, PxRigidDynamic> for OnAdvance {
    fn on_advance(
        &self,
        _actors: &[&physx::rigid_body::RigidBodyMap<PxArticulationLink, PxRigidDynamic>],
        _transforms: &[PxTransform],
    ) {
    }
}

pub(crate) fn gravity() -> PxVec3 {
    PxVec3::new(0.0, -9.81, 0.0)
}

/// The highest a level's kill height is ever put: the height every level had
/// when it was a constant, kept as the ceiling so a level whose floor is at
/// the origin respawns its fallen props exactly where it always did.
pub(crate) const KILL_Y_HIGHEST: f32 = -15.0;

/// How far under the lowest thing in the level a body's centre has to be to
/// have left it. Nothing can rest below the lowest collider, so any margin is
/// right; this one is room for a large body's own half height and for a dip
/// under the floor that the solver takes back.
pub(crate) const KILL_MARGIN: f32 = 5.0;

/// HOW THE SCENE'S SCULPTED GROUND IS GIVEN TO PHYSX
/// (`PhysicsWorld::spawn_scene_terrain`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainCollider {
    /// A `PhysX` height field: four bytes a sample, built in one pass over the
    /// samples with nothing to cook. Its holes are the terrain's, its cells
    /// are split along the diagonals the mesher draws, and the ground the
    /// caves put back -- triangles that are not on the grid -- is small
    /// triangle meshes beside it. The default.
    HeightField,
    /// One triangle mesh of the whole ground at every sample, as it was until
    /// 2026-10-13. Kept to compare against, and for a source that is not a
    /// grid. Measured on the stress levels at 257 / 625 / 1025 / 2049 samples
    /// a side (`print_what_the_physics_rebuild_costs`, an Apple M4, the
    /// ground's share of `rebuild`): 36 / 241 / 684 / 3,262 ms and 35 / 222 /
    /// 655 / 2,650 MB of process memory, where the height field takes 3 / 13
    /// / 54 / 162 ms and 1 / 3 / 15 / 88 MB (most of the last the 48 caves'
    /// patches and the files read for them). At 2049 the mesh is 8.9 million
    /// triangles, and it was not whole: 62 of 100 rays cast straight down on
    /// it met nothing.
    TriangleMesh,
}

impl TerrainCollider {
    /// The process's choice: `SS_TERRAIN_COLLIDER=mesh` asks for the triangle
    /// mesh; anything else, or nothing, the height field. What
    /// [`PhysicsWorld::new`] starts with; `set_terrain_collider` overrides it
    /// for one world.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("SS_TERRAIN_COLLIDER").as_deref() {
            Ok("mesh" | "trimesh" | "triangle_mesh") => Self::TriangleMesh,
            _ => Self::HeightField,
        }
    }
}

pub(crate) struct DynamicActor {
    pub(crate) ptr: *mut PxRigidDynamic,
    pub(crate) spawn_pos: Vec3,
    pub(crate) spawn_rot: Quat,
    pub(crate) respawn_interval: Option<f32>,
    pub(crate) elapsed: f32,
    /// Its collider's half extents in its own frame (a sphere's or a hull's:
    /// the box round it), and the volume of water it pushes aside when under,
    /// cubic metres. What the water bears it up by (`float_bodies`) and what a
    /// free grab holds it at the surface of (`grab_free`).
    pub(crate) half: Vec3,
    pub(crate) volume: f32,
    /// Where its object was in `Scene::objects` when last looked for: checked
    /// before any search, so writing a body's pose back is not a search of the level.
    pub(crate) object: usize,
    /// Asleep when last stepped: its pose has been written back already.
    pub(crate) asleep: bool,
    /// Its going in and out of the water (`water::WaterState`).
    pub(crate) water: crate::water::WaterState,
    /// Put back where it started after resting under water this long,
    /// seconds (`RigidBodyDef::respawn_sunk`), and how long it has.
    pub(crate) respawn_sunk: Option<f32>,
    pub(crate) sunk_for: f32,
    /// The world's clock when its turn to be looked at for a respawn last
    /// came (`PhysicsWorld::respawn_round`).
    pub(crate) checked_at: f64,
}

impl DynamicActor {
    /// A body as it starts, with nothing yet known of it but where.
    pub(crate) fn new(ptr: *mut PxRigidDynamic, spawn_pos: Vec3, spawn_rot: Quat, half: Vec3, volume: f32) -> Self {
        Self {
            ptr,
            spawn_pos,
            spawn_rot,
            respawn_interval: None,
            elapsed: 0.0,
            half,
            volume,
            object: 0,
            asleep: false,
            water: crate::water::WaterState::default(),
            respawn_sunk: None,
            sunk_for: 0.0,
            checked_at: 0.0,
        }
    }
}

pub(crate) struct GrabState {
    pub(crate) joint: *mut physx_sys::PxJoint,
    pub(crate) point_name: String,
    /// Where the hand holds it, in the body's own frame: a free grab's (and a
    /// door handle's) own point; none for an authored grip point's.
    pub(crate) local: Option<Vec3>,
}

/// A hold just let go of: its joint, with nothing left driving it, kept
/// until the body has left the hand (`PhysicsWorld::release`).
pub(crate) struct Parting {
    pub(crate) joint: *mut physx_sys::PxJoint,
    pub(crate) player: PlayerId,
    pub(crate) hand: Hand,
    pub(crate) id: String,
    pub(crate) local: Vec3,
    pub(crate) since: f64,
}

#[derive(Clone, Copy)]
pub(crate) struct Drive {
    pub(crate) stiffness: f32,
    pub(crate) damping: f32,
}

pub struct PhysicsWorld {
    pub(crate) scene: Owner<PxScene>,
    pub(crate) materials: Vec<Owner<PxMaterial>>,
    pub(crate) dynamic: HashMap<String, DynamicActor>,
    pub(crate) kinematic: HashMap<String, *mut PxRigidDynamic>,

    pub(crate) statics: HashMap<String, *mut PxRigidStatic>,

    pub(crate) hand_anchors: HashMap<(PlayerId, Hand), *mut PxRigidDynamic>,
    pub(crate) grabs: HashMap<(PlayerId, String, Hand), GrabState>,
    /// The holds just let go of, until hand and body have parted.
    pub(crate) parting: Vec<Parting>,
    pub(crate) scratch: ScratchBuffer,

    pub(crate) water: Vec<crate::water::WaterFootprint>,
    /// The bodies that went in since they were last asked for
    /// (`take_water_entries`): what the splashes are raised from. Which
    /// bodies are in it is each body's own (`DynamicActor::water`).
    pub(crate) water_entries: Vec<crate::water::WaterEntry>,

    pub(crate) doors: HashMap<String, crate::door::DoorGeom>,

    pub(crate) door_hinges: HashMap<String, (f32, f32)>,
    /// Each latching door's latch and handle (`door::DoorLatch`), and whether
    /// its bolt is home.
    pub(crate) door_latches: HashMap<String, crate::rigid_physics_door::LatchState>,

    /// Seconds this world has been stepped for.
    pub(crate) clock: f64,
    /// Whose turn it is to be looked at for a respawn: the body this many
    /// into `dynamic` (`respawn_round`).
    pub(crate) respawn_cursor: usize,
    /// Asked to put every prop back where it started, at the next step
    /// (`respawn_props`).
    pub(crate) respawn_all: bool,
    /// What the last step cost and saw (`StepStats`).
    pub(crate) stats: StepStats,
    /// Where model files are read from, for a hull collider (`rebuild`).
    pub(crate) game_dir: std::path::PathBuf,
    /// The hulls cooked so far, by model file and scale: an index into
    /// `hull_meshes`, its box's centre and half extents and its volume.
    pub(crate) hulls: HashMap<String, (usize, Vec3, Vec3, f32)>,

    /// THE LEVEL'S FLOOR: the lowest point of anything a body can rest on or
    /// start at -- the ground's lowest sample and the caves' patches, every
    /// collider, every door, the still water (so a body is seen going in
    /// before it is taken back). Lowered by each spawner as it builds its
    /// collider (`note_floor`), at load and for an object spawned later; never
    /// read or scanned in a tick. `f32::INFINITY` in a world with nothing in it.
    pub(crate) level_floor: f32,
    /// The height under which a dynamic body has fallen out of the world and
    /// is put back where it started (`step`): [`KILL_MARGIN`] under
    /// `level_floor`, and never above [`KILL_Y_HIGHEST`]. It was the constant
    /// -15 for every level, which respawned every prop of a level whose
    /// valley or cave floor lay deeper than that, each tick, for ever.
    pub(crate) kill_y: f32,

    /// How the next `rebuild` gives the ground to `PhysX`.
    pub(crate) terrain_collider: TerrainCollider,

    pub(crate) terrain_meshes: Vec<Owner<TriangleMesh>>,
    pub(crate) terrain_height_fields: Vec<Owner<HeightField>>,
    /// Cooked convex hulls (`ColliderShape::Hull`): owned by the physics
    /// object as a triangle mesh is, so released before the foundation too.
    pub(crate) hull_meshes: Vec<Owner<physx::convex_mesh::ConvexMesh>>,
    pub(crate) foundation: PxFoundation,
}

/// WHAT ONE STEP DID, for a bench and for anyone watching the cost: how many
/// dynamic bodies there are, how many were awake (the only ones a step reads,
/// floats or watches at the water), how many of those were in water, and how
/// many went in -- seen, and told of after the cap (`water::keep_most_energetic`).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct StepStats {
    pub bodies: usize,
    pub awake: usize,
    pub in_water: usize,
    pub entries_seen: usize,
    pub entries_told: usize,
    pub respawned: usize,
}

pub(crate) fn new_px_scene(foundation: &mut PxFoundation) -> Owner<PxScene> {
    foundation
        .create(SceneDescriptor {
            gravity: gravity(),
            thread_count: 1,

            flags: SceneFlags::EnablePcm | SceneFlags::EnableCcd,
            ..SceneDescriptor::new(0u64)
        })
        .expect("space_soup_engine: failed to create PxScene")
}

/// Where a hand's anchor is made, before its hand is known: far under any
/// level.
pub(crate) const HAND_ANCHOR_START_Y: f32 = -1.0e4;

pub(crate) fn create_hand_anchor(
    foundation: &mut PxFoundation,
    scene: &mut Owner<PxScene>,
    materials: &mut Vec<Owner<PxMaterial>>,
    player: PlayerId,
    hand: Hand,
) -> Option<*mut PxRigidDynamic> {
    let Some(mut material) = foundation.create_material(0.0, 0.0, 0.0, ()) else {
        log::warn!("rigid_physics: failed to create hand-anchor material for {player:?}/{hand:?}");
        return None;
    };
    let geo = PxSphereGeometry::new(0.035);
    let Some(mut actor) = foundation.create_rigid_dynamic(
        PxTransform::from_translation(&PxVec3::new(0.0, HAND_ANCHOR_START_Y, 0.0)),
        &geo,
        material.as_mut(),
        1.0,
        PxTransform::default(),
        (),
    ) else {
        log::warn!("rigid_physics: failed to create hand anchor for {player:?}/{hand:?}");
        return None;
    };
    actor.set_rigid_body_flag(RigidBodyFlag::Kinematic, true);
    for shape in actor.get_shapes_mut() {
        shape.set_flag(ShapeFlag::SceneQueryShape, false);
    }
    let ptr: *mut PxRigidDynamic = &raw mut *actor;
    scene.add_dynamic_actor(actor);
    materials.push(material);
    Some(ptr)
}

impl PhysicsWorld {
    #[must_use]
    pub fn new() -> Self {
        let mut foundation: PxFoundation = PhysicsFoundation::default();
        let scene = new_px_scene(&mut foundation);
        Self {
            foundation,
            scene,
            materials: Vec::new(),
            dynamic: HashMap::new(),
            kinematic: HashMap::new(),
            statics: HashMap::new(),
            hand_anchors: HashMap::new(),
            grabs: HashMap::new(),
            parting: Vec::new(),

            scratch: unsafe { ScratchBuffer::new(4) },
            water: Vec::new(),
            water_entries: Vec::new(),
            doors: HashMap::new(),
            door_hinges: HashMap::new(),
            door_latches: HashMap::new(),
            clock: 0.0,
            respawn_cursor: 0,
            respawn_all: false,
            stats: StepStats::default(),
            game_dir: std::path::PathBuf::new(),
            hulls: HashMap::new(),
            level_floor: f32::INFINITY,
            kill_y: KILL_Y_HIGHEST,
            terrain_collider: TerrainCollider::from_env(),
            terrain_meshes: Vec::new(),
            terrain_height_fields: Vec::new(),
            hull_meshes: Vec::new(),
        }
    }

    /// Choose how this world's ground is given to `PhysX`, from the next
    /// `rebuild` on.
    pub fn set_terrain_collider(&mut self, kind: TerrainCollider) {
        self.terrain_collider = kind;
    }

    #[must_use]
    pub fn terrain_collider(&self) -> TerrainCollider {
        self.terrain_collider
    }

    /// Something solid, or a body's start, reaches down to world height `y`:
    /// the level's floor is at least that low, and the kill height follows it.
    /// One comparison a collider, where the collider is built.
    pub(crate) fn note_floor(&mut self, y: f32) {
        if y.is_finite() && y < self.level_floor {
            self.level_floor = y;
            self.kill_y = KILL_Y_HIGHEST.min(y - KILL_MARGIN);
        }
    }

    /// The height under which a dynamic body is taken to have fallen out of
    /// the level and is put back where it started: under the level's own
    /// floor, wherever that is (`level_floor`).
    #[must_use]
    pub fn kill_height(&self) -> f32 {
        self.kill_y
    }

    /// How deep standing water is over ground at `ground_y`, at a world x/z:
    /// the highest surface standing there, less the ground. `None` where no
    /// water stands; negative where the ground is above the surface, as on a
    /// beach the waves run up.
    pub fn water_depth_at(&self, x: f32, z: f32, ground_y: f32) -> Option<f32> {
        self.water.iter().filter_map(|w| w.surface_at(x, z)).reduce(f32::max).map(|surface| surface - ground_y)
    }

    /// The still surface of the highest water standing over a world x/z.
    pub fn water_surface_at(&self, x: f32, z: f32) -> Option<f32> {
        self.water.iter().filter_map(|w| w.surface_at(x, z)).reduce(f32::max)
    }

    /// THE BODIES THAT WENT INTO THE WATER since this was last called: each
    /// as it met the surface -- where, how fast and at what angle, how wide,
    /// how heavy -- which is what its splash is drawn from
    /// (`space_soup::renderer::effects::Splash`). Seen in [`Self::step`].
    pub fn take_water_entries(&mut self) -> Vec<crate::water::WaterEntry> {
        std::mem::take(&mut self.water_entries)
    }

    /// WHERE A SHOT MEETS WATER: the point where the ray from `origin` along
    /// `dir` first crosses a still water surface within `max_distance`, if
    /// nothing solid stops it first, and the water's depth there. What a
    /// firearm asks before it raises a bullet's splash
    /// (`effects::Splash::projectile`).
    #[must_use]
    pub fn water_hit(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<(Vec3, f32)> {
        let dir = dir.try_normalize()?;
        if dir.y >= -1e-5 {
            return None;
        }
        let solid = self.raycast(origin, dir, max_distance).map_or(max_distance, |(hit, _)| origin.distance(hit));
        let mut heights: Vec<f32> = Vec::new();
        for w in &self.water {
            let Some(h) = w.surface_height() else { continue };
            if !heights.contains(&h) {
                heights.push(h);
            }
        }
        heights
            .into_iter()
            .filter_map(|h| {
                let t = (h - origin.y) / dir.y;
                let p = origin + dir * t;
                (t > 0.0 && t <= solid && self.water_surface_at(p.x, p.z).is_some_and(|s| (s - h).abs() < 1e-4)).then_some((t, p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| {
                let bed = self.raycast_down(p - Vec3::Y * 0.01, 60.0).map_or(30.0, |(hit, _)| p.y - hit.y);
                (p, bed.max(0.0))
            })
    }

    /// How far the water over a world x/z on ground at `ground_y` is deeper
    /// than its own wading limit (`WaterDef::max_wade_depth`), metres; the
    /// most of any water standing there. Negative where it can be waded.
    pub fn water_wade_excess_at(&self, x: f32, z: f32, ground_y: f32) -> Option<f32> {
        self.water.iter().filter_map(|w| w.surface_at(x, z).map(|s| s - ground_y - w.wade_depth())).reduce(f32::max)
    }

    pub fn ensure_player(&mut self, player: PlayerId) {
        for hand in [Hand::Left, Hand::Right] {
            if self.hand_anchors.contains_key(&(player, hand)) {
                continue;
            }
            if let Some(ptr) = create_hand_anchor(
                &mut self.foundation,
                &mut self.scene,
                &mut self.materials,
                player,
                hand,
            ) {
                self.hand_anchors.insert((player, hand), ptr);
            }
        }
    }

    pub fn remove_player(&mut self, player: PlayerId) {
        let held_by_player: Vec<(PlayerId, String, Hand)> = self
            .grabs
            .keys()
            .filter(|(p, _, _)| *p == player)
            .cloned()
            .collect();
        for key in held_by_player {
            if let Some(state) = self.grabs.remove(&key) {
                unsafe { physx_sys::PxJoint_release_mut(state.joint) };
            }
        }
        self.parting.retain(|p| {
            if p.player == player {
                unsafe { physx_sys::PxJoint_release_mut(p.joint) };
            }
            p.player != player
        });

        for hand in [Hand::Left, Hand::Right] {
            let Some(ptr) = self.hand_anchors.remove(&(player, hand)) else {
                continue;
            };
            if ptr.is_null() {
                continue;
            }
            unsafe {
                self.scene.remove_actor(&mut *ptr, false);
                physx_sys::PxActor_release_mut(ptr.cast::<physx_sys::PxActor>());
            }
        }
    }

    pub fn rebuild(&mut self, scene: &Scene, game_dir: &Path) {
        self.dynamic.clear();
        self.kinematic.clear();
        self.statics.clear();
        self.materials.clear();
        self.terrain_meshes.clear();
        self.terrain_height_fields.clear();
        self.doors.clear();
        self.door_hinges.clear();
        self.door_latches.clear();
        self.respawn_cursor = 0;
        self.respawn_all = false;
        self.game_dir = game_dir.to_path_buf();

        self.grabs.clear();
        self.parting.clear();
        self.hand_anchors.clear();
        self.scene = new_px_scene(&mut self.foundation);
        self.hulls.clear();
        self.hull_meshes.clear();

        self.level_floor = f32::INFINITY;
        self.kill_y = KILL_Y_HIGHEST;

        for entity in &scene.objects {
            let Some(def) = &entity.rigid_body else { continue };
            self.spawn_actor(entity, def);
        }

        for entity in &scene.objects {
            let Some(def) = &entity.brush else { continue };
            if entity.is_solid_brush() {
                self.spawn_brush_collider(entity, def);
            }
        }

        for entity in &scene.objects {
            let Some(def) = &entity.slider_joint else {
                continue;
            };
            self.spawn_slider_joint(entity, def);
        }

        for entity in &scene.objects {
            let Some(def) = &entity.door else {
                continue;
            };
            self.spawn_door(entity, def);
        }

        for entity in &scene.objects {
            let Some(def) = &entity.terrain_collider else {
                continue;
            };
            self.spawn_terrain_colliders(entity, def, game_dir);
        }

        let terrain = scene.terrain.as_ref().and_then(|def| match crate::terrain::load(def, game_dir) {
            Ok(source) => Some(source),
            Err(e) => {
                log::warn!("rigid_physics: scene terrain not loaded: {e}");
                None
            }
        });
        if let Some(source) = &terrain {
            self.spawn_scene_terrain(source.as_ref());
            self.note_floor(source.lowest_height());
        }

        match &terrain {
            Some(source) => self.spawn_scatter_trunks(scene, &SourceGround(source.as_ref())),
            None => self.spawn_scatter_trunks(scene, &crate::scatter::FlatGround(0.0)),
        }

        for entity in &scene.objects {
            let Some(def) = &entity.walk_area else { continue };
            self.spawn_walk_area(entity, def, terrain.as_deref());
        }

        self.water = scene
            .water
            .iter()
            .filter_map(|def| {
                crate::water::WaterFootprint::new(def, |x, z| terrain.as_ref().and_then(|t| t.height_at(x, z)))
            })
            .collect();
        for height in self.water.iter().filter_map(super::water::WaterFootprint::surface_height).collect::<Vec<_>>() {
            self.note_floor(height);
        }
    }
}

struct SourceGround<'a>(&'a dyn crate::terrain::TerrainSource);

impl crate::scatter::Ground for SourceGround<'_> {
    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.0.height_at(x, z)
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}
