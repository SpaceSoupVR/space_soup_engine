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
}

#[derive(Debug, Clone, Default)]
pub struct InputFrame {
    pub pointed: Vec<(String, Hand)>,

    pub grabbed: Vec<(String, Hand, String)>,
    pub released: Vec<(String, Hand)>,
    pub button_presses: Vec<ButtonPress>,
}

#[derive(Debug, Clone)]
pub struct ButtonPress {
    pub button: String,
    pub object_id: Option<String>,
}
