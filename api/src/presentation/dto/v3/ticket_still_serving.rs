//! Refusal details for `ticketStillServing` (`docs/design/blueprint/queue-tickets.md` §4.7; `xchange-v3-web.md` §4.3).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_queue::domain::ticket::live_ticket::LiveTicket;

/// `error.details` on a 409 `ticketStillServing` refusal: the CALLED and SERVING tickets blocking the shift close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ticket_still_serving.ts")]
pub struct TicketStillServingDetails {
    pub tickets: Vec<LiveTicketDto>,
}

/// One live ticket blocking the shift close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ticket_still_serving.ts")]
pub struct LiveTicketDto {
    pub number: String,
    pub status: String,
    pub teller_id: String,
    pub teller_name: Option<String>,
}

impl From<LiveTicket> for LiveTicketDto {
    fn from(t: LiveTicket) -> Self {
        Self {
            number: t.number,
            status: t.status,
            teller_id: t.teller_id,
            teller_name: t.teller_name,
        }
    }
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TicketStillServingDetails::export_all(&cfg)?;
    LiveTicketDto::export_all(&cfg)?;
    Ok(())
}
