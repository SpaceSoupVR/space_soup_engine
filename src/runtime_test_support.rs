#![cfg(test)]
use std::collections::HashMap;
use std::sync::Mutex;

use space_soup_protocol::PlayerId;

use crate::events::InputFrame;
use crate::locomotion::LocomotionInput;
use crate::rig::PlayerRig;
use crate::runtime::PlayerFrameInput;

// physx-sys's Foundation is a process-wide singleton -- any two rigid_physics
// tests running concurrently on different threads (the default for `cargo
// test`) abort with "Foundation object exists already". This lock is shared
// across every runtime_tests_* module specifically so they still serialize
// against each other, not just within their own file.
pub(crate) static PHYSX_TEST_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn frame(rig: PlayerRig, input: InputFrame) -> PlayerFrameInput {
    PlayerFrameInput {
        rig,
        input,
        locomotion_input: LocomotionInput::default(),
        teleport_target: None,
        client_offset: None,
        client_yaw: None,
    }
}

pub(crate) fn one_player(id: PlayerId, f: PlayerFrameInput) -> HashMap<PlayerId, PlayerFrameInput> {
    let mut m = HashMap::new();
    m.insert(id, f);
    m
}
