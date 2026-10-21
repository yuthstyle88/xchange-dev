//! `TicketPrintJob` (design `kiosk-shared-phone.md` §4.2 D9, §4.4 B10): the one ESC/POS renderer
//! for the queue ticket. `issueByToken` answers its bytes; the Flutter queue machine writes them
//! unchanged to its own printer and has no renderer of its own.

use chrono::{DateTime, FixedOffset};
use pos_printer::{EscposBuilder, PaperWidth, PrinterCharset, QrEcLevel, Weight};

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
        let mut b = EscposBuilder::new(PrinterCharset::PASSTHROUGH, PaperWidth::Mm80);
        b.center_line(&self.branch_name, Weight::Regular)
            .text_size(0x11)
            .center_line(&self.display_number, Weight::Bold)
            .text_size(0x00)
            .center_line(
                &self.issued_at.format("%Y-%m-%d %H:%M").to_string(),
                Weight::Regular,
            )
            .align_center()
            .qr_native(&self.qr_url, 6, QrEcLevel::M)
            .feed(2);
        b.cut(false)
    }
}
