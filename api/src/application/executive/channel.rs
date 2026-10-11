//! How the customer came (owner decision E-5; design `executive-finance.md` §4.1, ruling G1 on #1071).
//!
//! The one place the booking, kiosk and referral conditions are written: EX-3's facts and the drill-down targets
//! `admin_transactions` and `admin_overview` (EX-11a #1103, EX-11b #1104) call [`Channel::condition`] rather than
//! repeating them.

/// The channel filter of A01. Wire values are exactly `WalkIn`, `Booking`, `Kiosk`, `Referred`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    WalkIn,
    Booking,
    Kiosk,
    Referred,
}

impl Channel {
    /// The channel named by its wire value; `None` for anything else.
    pub fn parse(value: &str) -> Option<Channel> {
        match value {
            "WalkIn" => Some(Channel::WalkIn),
            "Booking" => Some(Channel::Booking),
            "Kiosk" => Some(Channel::Kiosk),
            "Referred" => Some(Channel::Referred),
            _ => None,
        }
    }

    /// The wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::WalkIn => "WalkIn",
            Channel::Booking => "Booking",
            Channel::Kiosk => "Kiosk",
            Channel::Referred => "Referred",
        }
    }

    /// A boolean SQL expression over the transactions row `alias`, with no bind parameters.
    ///
    /// - `Booking`: `alias.booking_no` names a booking whose `transaction_ref` is `alias.document_ref`.
    /// - `Kiosk`: a `kiosk_requests` row's `document_ref` is `alias.document_ref`.
    /// - `Referred`: an ACTIVE `referral_attributions` row has `transaction_id = alias.id` (RA-1 #979).
    /// - `WalkIn`: none of the three. A referred booking matches both `Booking` and `Referred`.
    ///
    /// `alias` is a table alias the caller wrote, never request input.
    pub fn condition(self, alias: &str) -> String {
        match self {
            Channel::Booking => booked(alias),
            Channel::Kiosk => kiosk(alias),
            Channel::Referred => referred(alias),
            Channel::WalkIn => format!(
                "(NOT {} AND NOT {} AND NOT {})",
                booked(alias),
                kiosk(alias),
                referred(alias)
            ),
        }
    }
}

fn booked(alias: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM bookings ch_b \
         WHERE ch_b.organization_id = {alias}.organization_id AND ch_b.id = {alias}.booking_no \
         AND ch_b.transaction_ref = {alias}.document_ref)"
    )
}

fn kiosk(alias: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM kiosk_requests ch_k \
         WHERE ch_k.organization_id = {alias}.organization_id AND ch_k.document_ref = {alias}.document_ref)"
    )
}

fn referred(alias: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM referral_attributions ch_r \
         WHERE ch_r.transaction_id = {alias}.id AND ch_r.status = 'ACTIVE')"
    )
}
