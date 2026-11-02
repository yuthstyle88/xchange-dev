//! `LiveTicket`: a CALLED or SERVING ticket that blocks a shift close (`queue-tickets.md` §4.7).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTicket {
    pub number: String,
    pub status: String,
    pub teller_id: String,
    pub teller_name: Option<String>,
}

impl LiveTicket {
    pub const MARKER: &'static str = "; tickets=";

    pub fn refusal(tickets: &[Self]) -> String {
        let json = serde_json::to_string(tickets).expect("serialize live tickets");
        format!("ticketStillServing{}{json}", Self::MARKER)
    }

    pub fn from_refusal(message: &str) -> Vec<Self> {
        let Some((_, tail)) = message.split_once(Self::MARKER) else {
            return Vec::new();
        };
        serde_json::from_str(tail).unwrap_or_default()
    }
}
