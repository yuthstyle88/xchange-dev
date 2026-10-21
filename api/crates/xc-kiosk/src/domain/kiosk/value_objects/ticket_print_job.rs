//! `TicketPrintJob` (design `kiosk-shared-phone.md` §4.2 D9, §4.4 B10): the one ESC/POS renderer
//! for the queue ticket. `issueByToken` answers its bytes; the Flutter queue machine writes them
//! unchanged to its own printer and has no renderer of its own.

use chrono::{DateTime, FixedOffset};

/// What the queue ticket prints. Nothing else from the request is printed (no amount, no name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketPrintJob {
    /// `W001`, … (`IssuedTicket::display_number`).
    pub display_number: String,
    /// `branches.name`.
    pub branch_name: String,
    /// When the ticket was issued, in the branch's own offset.
    pub issued_at: DateTime<FixedOffset>,
    /// `<origin>/q#<token>`: the request QR, for the desk scan (D7).
    pub qr_url: String,
}

impl TicketPrintJob {
    /// The ticket as ESC/POS bytes for an 80 mm printer, built with `pos-printer`: the branch,
    /// the number (large), the time `YYYY-MM-DD HH:MM`, the request QR, then a cut.
    /// The same job always renders the same bytes (B7: a repeat scan reprints byte for byte).
    pub fn render(&self) -> Vec<u8> {
        todo!("KWQ-A3: render with pos_printer::escpos::EscposBuilder")
    }
}
