//! `Channel` — `notification_deliveries.channel`. Only `InApp` is written at launch (owner C3).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    InApp,
    Email,
    Push,
    Sms,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::InApp => "IN_APP",
            Channel::Email => "EMAIL",
            Channel::Push => "PUSH",
            Channel::Sms => "SMS",
        }
    }
}
