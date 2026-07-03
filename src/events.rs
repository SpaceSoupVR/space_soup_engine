#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

impl Default for Hand {
    fn default() -> Self { Hand::Right }
}

impl Hand {
    pub fn as_str(self) -> &'static str {
        match self {
            Hand::Left  => "left",
            Hand::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InputFrame {
    pub pointed: Vec<(String, Hand)>,
    /// object id, hand, named grip point (empty string if the grab wasn't
    /// on a named point — e.g. an object with no `grip_points` defined).
    pub grabbed: Vec<(String, Hand, String)>,
    pub released: Vec<(String, Hand)>,
    pub button_presses: Vec<ButtonPress>,
}

#[derive(Debug, Clone)]
pub struct ButtonPress {
    pub button:    String,
    pub object_id: Option<String>,
}
