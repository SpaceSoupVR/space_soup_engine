#![cfg(test)]
use std::collections::HashMap;
use std::sync::Mutex;

use space_soup_protocol::PlayerId;

use crate::events::InputFrame;
use crate::locomotion::LocomotionInput;
use crate::rig::PlayerRig;
use crate::runtime::PlayerFrameInput;

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
