use std::collections::HashMap;
use std::path::Path;

use glam::{Quat, Vec3};
use physx::prelude::*;

use physx::scene::Scene as _;

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

pub(crate) struct DynamicActor {
    pub(crate) ptr: *mut PxRigidDynamic,
    pub(crate) spawn_pos: Vec3,
    pub(crate) spawn_rot: Quat,
    pub(crate) respawn_interval: Option<f32>,
    pub(crate) elapsed: f32,
}

pub(crate) struct GrabState {
    pub(crate) joint: *mut physx_sys::PxJoint,
    pub(crate) point_name: String,
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
    /// Static actors, kept so a breached wall can stop colliding.
    ///
    /// These used to be created and handed straight to the scene with no
    /// handle kept, which made "remove this wall's collider" unrepresentable --
    /// the reason a breached structure was visibly broken and still solid.
    ///
    /// Raw pointers, like `kinematic`: the scene owns the actors, so this map
    /// borrows rather than owns and its position among the fields does not
    /// affect the drop order documented below.
    pub(crate) statics: HashMap<String, *mut PxRigidStatic>,

    pub(crate) hand_anchors: HashMap<(PlayerId, Hand), *mut PxRigidDynamic>,
    pub(crate) grabs: HashMap<(PlayerId, String, Hand), GrabState>,
    pub(crate) scratch: ScratchBuffer,

    /// Where standing water lies. Not colliders -- water is waded through --
    /// but how deep it stands, so a walk stops before it is out of its depth.
    pub(crate) water: Vec<crate::water::WaterFootprint>,
    /// Which bodies are in it, and those that went in since they were last
    /// asked for (`take_water_entries`): what the splashes are raised from.
    pub(crate) water_watch: crate::water::WaterWatch,
    pub(crate) water_entries: Vec<crate::water::WaterEntry>,

    /// The doors, by object id: each one's leaf as it hangs. Their bodies are
    /// in `dynamic` like any other; this is the hinge they turn on, for the
    /// bodies' pushes (`push_doors`) and for anyone asking a door's angle.
    pub(crate) doors: HashMap<String, crate::door::DoorGeom>,
    /// Each door's closer and hinge friction (`DoorDef::closer`,
    /// `DoorDef::friction`), applied as torques by `push_doors`.
    pub(crate) door_hinges: HashMap<String, (f32, f32)>,

    // ORDER MATTERS, and it is load-bearing rather than stylistic. Rust drops
    // fields in declaration order, and a cooked TriangleMesh is owned by the
    // physics object inside the foundation: releasing one after the foundation
    // has gone is a use-after-free, and it segfaults at teardown rather than
    // where the mistake is.
    //
    // So the sequence has to be: `scene` first (it releases the actors that
    // reference the meshes), then `terrain_meshes`, then `foundation` last.
    //
    // This was already wrong -- terrain_meshes sat after foundation -- and it
    // had never been caught because nothing exercised a cooked terrain collider
    // in a test. The glTF terrain_collider path had the same fault.
    pub(crate) terrain_meshes: Vec<Owner<TriangleMesh>>,
    pub(crate) foundation: PxFoundation,
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
        PxTransform::default(),
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
    let ptr: *mut PxRigidDynamic = &mut *actor as *mut PxRigidDynamic;
    scene.add_dynamic_actor(actor);
    materials.push(material);
    Some(ptr)
}

impl PhysicsWorld {
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

            scratch: unsafe { ScratchBuffer::new(4) },
            water: Vec::new(),
            water_watch: crate::water::WaterWatch::default(),
            water_entries: Vec::new(),
            doors: HashMap::new(),
            door_hinges: HashMap::new(),
            terrain_meshes: Vec::new(),
        }
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
    pub fn water_hit(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<(Vec3, f32)> {
        let dir = dir.try_normalize()?;
        if dir.y >= -1e-5 {
            return None;
        }
        let solid = self.raycast(origin, dir, max_distance).map_or(max_distance, |(hit, _)| origin.distance(hit));
        // Each body's own plane: the nearest crossing that lies on its water.
        let mut heights: Vec<f32> = Vec::new();
        for w in &self.water {
            // A footprint's surface is one height wherever it stands.
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

        for hand in [Hand::Left, Hand::Right] {
            let Some(ptr) = self.hand_anchors.remove(&(player, hand)) else {
                continue;
            };
            if ptr.is_null() {
                continue;
            }
            unsafe {
                self.scene.remove_actor(&mut *ptr, false);
                physx_sys::PxActor_release_mut(ptr as *mut physx_sys::PxActor);
            }
        }
    }

    pub fn rebuild(&mut self, scene: &Scene, game_dir: &Path) {
        self.dynamic.clear();
        self.kinematic.clear();
        self.materials.clear();
        self.terrain_meshes.clear();
        self.doors.clear();
        self.door_hinges.clear();

        self.grabs.clear();
        self.hand_anchors.clear();
        self.scene = new_px_scene(&mut self.foundation);

        for obj in &scene.objects {
            let Some(def) = &obj.rigid_body else { continue };
            self.spawn_actor(obj, def);
        }

        // After rigid bodies, so an authored `rigid_body` on a brush keeps the
        // shape its author chose. See `GameObject::solid`.
        for obj in &scene.objects {
            let Some(def) = &obj.brush else { continue };
            if obj.is_solid_brush() {
                self.spawn_brush_collider(obj, def);
            }
        }

        for obj in &scene.objects {
            let Some(def) = &obj.slider_joint else {
                continue;
            };
            self.spawn_slider_joint(obj, def);
        }

        for obj in &scene.objects {
            let Some(def) = &obj.door else {
                continue;
            };
            self.spawn_door(obj, def);
        }

        for obj in &scene.objects {
            let Some(def) = &obj.terrain_collider else {
                continue;
            };
            self.spawn_terrain_colliders(obj, def, game_dir);
        }

        // The ground's heights, loaded once for its collider and for the water
        // measured against it. A failure warns and leaves the scene without
        // ground rather than aborting the load: a level that opens with
        // nothing to stand on is diagnosable, and a runtime that refuses to
        // start is not.
        let terrain = scene.terrain.as_ref().and_then(|def| match crate::terrain::load(def, game_dir) {
            Ok(source) => Some(source),
            Err(e) => {
                log::warn!("rigid_physics: scene terrain not loaded: {e}");
                None
            }
        });
        if let Some(source) = &terrain {
            self.spawn_scene_terrain(source.as_ref());
        }

        // By the same heightfield the ground collider was cooked from, and by
        // the drawn surface's own rule (see `WaterFootprint`).
        self.water = scene
            .water
            .iter()
            .filter_map(|def| {
                crate::water::WaterFootprint::new(def, |x, z| terrain.as_ref().and_then(|t| t.height_at(x, z)))
            })
            .collect();
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}
