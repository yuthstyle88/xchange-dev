//! `CustomerQuote` (design `kiosk-shared-phone.md` §4.2 D2): what makes a quote a web request —
//! the signed-in member and the request token the caller minted (UUID v4). Only the token's
//! SHA-256 is stored, in `kiosk_requests.cancel_token_hash`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerQuote {
    /// `customers.id` of the signed-in member.
    pub customer_id: String,
    /// The request token in the clear; the caller returns it once and never stores it.
    pub token: String,
}
