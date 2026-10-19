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

    #[serde(default)]
    pub button_releases: Vec<ButtonPress>,

    #[serde(default)]
    pub axes: InputAxes,

    #[serde(default)]
    pub part_blends: std::collections::HashMap<String, std::collections::HashMap<String, f32>>,

    #[serde(default)]
    pub part_transforms:
        std::collections::HashMap<String, std::collections::HashMap<String, ([f32; 3], [f32; 4])>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ButtonPress {
    pub button: String,
    pub object_id: Option<String>,

    #[serde(default)]
    pub hand: Option<Hand>,
}

impl ButtonPress {
    pub fn new(button: impl Into<String>, object_id: Option<String>, hand: Hand) -> Self {
        Self { button: button.into(), object_id, hand: Some(hand) }
    }
}

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


/// The name of the button press that asks for every loose prop to be put
/// back where it started (`GameRuntime::dispatch_input`): sent with no
/// object, by whatever control the headset gives it.
pub const RESPAWN_PROPS_BUTTON: &str = "respawn_props";
