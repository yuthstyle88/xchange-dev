//! Mode value object (LIVE or DEMO).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Live,
    Demo,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "LIVE",
            Self::Demo => "DEMO",
        }
    }
}
