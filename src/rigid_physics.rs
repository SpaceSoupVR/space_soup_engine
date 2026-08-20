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

    pub(crate) hand_anchors: HashMap<(PlayerId, Hand), *mut PxRigidDynamic>,
    pub(crate) grabs: HashMap<(PlayerId, String, Hand), GrabState>,
    pub(crate) scratch: ScratchBuffer,

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
            hand_anchors: HashMap::new(),
            grabs: HashMap::new(),

            scratch: unsafe { ScratchBuffer::new(4) },
            terrain_meshes: Vec::new(),
        }
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

        self.grabs.clear();
        self.hand_anchors.clear();
        self.scene = new_px_scene(&mut self.foundation);

        for obj in &scene.objects {
            let Some(def) = &obj.rigid_body else { continue };
            self.spawn_actor(obj, def);
        }

        for obj in &scene.objects {
            let Some(def) = &obj.slider_joint else {
                continue;
            };
            self.spawn_slider_joint(obj, def);
        }

        for obj in &scene.objects {
            let Some(def) = &obj.terrain_collider else {
                continue;
            };
            self.spawn_terrain_colliders(obj, def, game_dir);
        }

        if let Some(def) = &scene.terrain {
            self.spawn_scene_terrain(def, game_dir);
        }
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}
