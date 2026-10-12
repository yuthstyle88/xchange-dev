//! The customer account's row (CI-7 #654, design `customer-identity-consent.md` §4.4).
//!
//! `consentRecord`, `profileUpdate` and `my_profile` inherit `customerSelf`. `Policy::authorize` targets the actor's
//! own customer, so the row passes a signed-in customer for their own account only, and answers `outOfScope` to a
//! CUSTOMER user with no linked customer. Staff and the owner hold no `CustomerSelf`: `roleNotAllowedHere`.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "customerSelf",
    need: Need::Permission(Permission::CustomerSelf),
}];
