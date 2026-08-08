use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hand {
    Left,
    #[default]
    Right,
}

impl Hand {
    pub fn as_str(self) -> &'static str {
        match self {
            Hand::Left => "left",
            Hand::Right => "right",
        }
    }

    pub fn other(self) -> Hand {
        match self {
            Hand::Left => Hand::Right,
            Hand::Right => Hand::Left,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputFrame {
    pub pointed: Vec<(String, Hand)>,

    pub grabbed: Vec<(String, Hand, String)>,
    pub released: Vec<(String, Hand)>,
    pub button_presses: Vec<ButtonPress>,

    /// Button-up edges. Separate from `button_presses` rather than a flag on it
    /// so older wire data still deserializes into an empty release list.
    #[serde(default)]
    pub button_releases: Vec<ButtonPress>,

    /// Continuous controller values, refreshed every frame.
    #[serde(default)]
    pub axes: InputAxes,

    /// Every part-animation blend the client computed this frame:
    /// object id -> clip name -> 0..1.
    ///
    /// The client owns these because it is the only side that can compute them:
    /// a HandPull blend comes from where the player's hand is relative to the
    /// part, which needs the skinned pose. The engine needs them anyway, because
    /// blend-threshold triggers spawn objects and apply impulses -- authoritative
    /// work that cannot live on a headset.
    #[serde(default)]
    pub part_blends: std::collections::HashMap<String, std::collections::HashMap<String, f32>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ButtonPress {
    pub button: String,
    pub object_id: Option<String>,

    /// Which controller produced it. `None` on wire data written before this
    /// existed. A two-handed weapon cannot tell the support hand from the
    /// firing hand without it.
    #[serde(default)]
    pub hand: Option<Hand>,
}

impl ButtonPress {
    pub fn new(button: impl Into<String>, object_id: Option<String>, hand: Hand) -> Self {
        Self { button: button.into(), object_id, hand: Some(hand) }
    }
}

/// Continuous controller inputs.
///
/// Deliberately polled rather than delivered as events: an axis changes every
/// frame, and a script that wants "how hard is the trigger held" wants to ask in
/// `on_update`, not to be woken sixty times a second. Edges get events, levels
/// get getters.
///
/// This is also what makes a held-button behaviour writable at all -- button
/// presses are edge-triggered, so before this a script could see the trigger go
/// down but had nothing to tell it the trigger was still down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct InputAxes {
    pub l_trigger: f32,
    pub r_trigger: f32,
    pub l_grip: f32,
    pub r_grip: f32,
    pub l_stick: [f32; 2],
    pub r_stick: [f32; 2],
}

impl InputAxes {
    pub fn trigger(&self, hand: Hand) -> f32 {
        match hand {
            Hand::Left => self.l_trigger,
            Hand::Right => self.r_trigger,
        }
    }

    pub fn grip(&self, hand: Hand) -> f32 {
        match hand {
            Hand::Left => self.l_grip,
            Hand::Right => self.r_grip,
        }
    }

    pub fn stick(&self, hand: Hand) -> [f32; 2] {
        match hand {
            Hand::Left => self.l_stick,
            Hand::Right => self.r_stick,
        }
    }
}

