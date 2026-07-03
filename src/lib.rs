pub mod manifest;
pub mod scene;
pub mod animation;
pub mod physics;
pub mod events;
pub mod script;
pub mod rig;
pub mod attach;
pub mod locomotion;
pub mod hands;
pub mod runtime;
pub mod rigid_physics;
pub mod debug_protocol;

pub use manifest::Manifest;
pub use scene::{
    Scene, GameObject, Animation, Keyframe, Easing,
    CuboidDef, Color3, CuboidStyle, MeshRef,
    RigAttachmentDef, GripPoseDef,
    BodyMode, ColliderShape, RigidBodyDef,
    GripKind, GripPointDef,
};
pub use events::{InputFrame, Hand, ButtonPress};
pub use rig::{PlayerRig, JointId, FingerJoint, Transform};
pub use attach::{Attachment, AttachmentTable};
pub use locomotion::{Locomotion, LocomotionMode, LocomotionInput, TeleportTarget};
pub use hands::{spawn_hand_rig, spawn_both_hand_rigs, despawn_hand_rig};
pub use runtime::{GameRuntime, RenderCuboid, RenderMesh};
pub use debug_protocol::{
    DebugPacket, Pose, HandSample, JointSample, LocomotionSample, SceneSample, TimingSample,
    sender as debug_sender, receiver as debug_receiver,
};
