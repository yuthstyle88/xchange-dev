//! `LoyaltyCustomerRef` (design D-2): the customer reference v3 sends to Loyalty as its
//! `customer_id` soft reference.

use uuid::Uuid;

/// The UUIDv5 namespace of every xchange customer reference sent to Loyalty.
///
/// **Never change this value.** It was chosen once (a random v4, lane controller claude/ad05bf8d's
/// ruling on #1381) and every `customerRef` Loyalty stores is derived from it; a new value would
/// orphan every member reference already sent. The LY-1 tests pin it.
pub const NS_XC_LOYALTY: Uuid = Uuid::from_u128(0x4d7a67a3_4dc9_48d1_8163_f21a31438408);

pub struct LoyaltyCustomerRef;

impl LoyaltyCustomerRef {
    /// `UUIDv5(NS_XC_LOYALTY, "xchange:" ‖ organization_id ‖ ":" ‖ customers.id)` (D-2), with the
    /// organisation in its hyphenated lowercase form.
    pub fn of(organization_id: Uuid, customer_id: &str) -> Uuid {
        let name = format!("xchange:{organization_id}:{customer_id}");
        Uuid::new_v5(&NS_XC_LOYALTY, name.as_bytes())
    }
}
