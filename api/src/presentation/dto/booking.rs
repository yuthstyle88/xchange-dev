//! Booking presentation DTOs and JSON converters.

use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};
use xc_exchange::application::booking::list::{BookingList, BookingView};
use xc_exchange::domain::booking::entities::booking::{Booking, CompatRate};
use xc_exchange::domain::booking::entities::member::Member;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

/// Formats a decimal string into a shortest JSON number with no trailing fractional zeros.
fn shortest_number(raw: &str) -> Value {
    if let Ok(dec) = Decimal::from_str(raw) {
        let norm = dec.normalize().to_string();
        if let Ok(num) = Number::from_str(&norm) {
            return Value::Number(num);
        }
    }
    Value::Null
}

/// The 13 fields of a booking as printed:
/// `{id, branchId, name, phone, member: bool, type: "BUY"|"SELL", currency, amount: number, rate: number, status, createdAt, expiresAt: string|null, transactionRef: string|null}`
pub fn booking_json(booking: &Booking, now: DateTime<Utc>) -> Value {
    let side_str = match booking.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };
    let amount_num = shortest_number(&booking.amount.to_string());
    let per_one = CompatRate::per_one(booking.rate, booking.unit_size);
    let rate_num = shortest_number(&per_one.to_string());
    let created_at = booking
        .created_at
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let expires_at = booking
        .expires_at
        .map(|t| Value::String(t.to_rfc3339_opts(SecondsFormat::Millis, true)))
        .unwrap_or(Value::Null);
    let transaction_ref = booking
        .transaction_ref
        .as_ref()
        .map(|s| Value::String(s.clone()))
        .unwrap_or(Value::Null);

    json!({
        "id": booking.id,
        "branchId": booking.branch_id.as_str(),
        "name": booking.name,
        "phone": booking.phone,
        "member": booking.is_member(),
        "type": side_str,
        "currency": booking.currency.as_str(),
        "amount": amount_num,
        "rate": rate_num,
        "status": booking.shown_status(now),
        "createdAt": created_at,
        "expiresAt": expires_at,
        "transactionRef": transaction_ref,
    })
}

/// A booking view as returned by `get`: the 13 booking fields plus `customerId`.
pub fn booking_view_json(view: &BookingView, now: DateTime<Utc>) -> Value {
    let mut obj = booking_json(&view.booking, now);
    let customer_id = view
        .customer_id
        .as_ref()
        .map(|s| Value::String(s.clone()))
        .unwrap_or(Value::Null);
    if let Value::Object(ref mut map) = obj {
        map.insert("customerId".to_string(), customer_id);
    }
    obj
}

/// A member formatted with v2's snake_case keys.
pub fn member_json(member: &Member) -> Value {
    json!({
        "customer_id": member.customer_id,
        "id": member.id,
        "name": member.name,
        "phone": member.phone,
        "verified_at": member.verified_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

/// Rules formatted per branch: default rules have no branch_id, stored rules include branch_id.
pub fn rules_json(branch: &BranchId, rules: Option<&BookingRules>) -> Value {
    match rules {
        Some(r) => json!({
            "branch_id": branch.as_str(),
            "enabled": if r.enabled { 1 } else { 0 },
            "minutes": r.minutes,
        }),
        None => json!({
            "enabled": 1,
            "minutes": 30,
        }),
    }
}

/// The response for the `list` action: bookings, members, rules, publicUrl.
pub fn list_json(branch: &BranchId, list: &BookingList, now: DateTime<Utc>) -> Value {
    json!({
        "bookings": list.bookings.iter().map(|b| booking_json(b, now)).collect::<Vec<_>>(),
        "members": list.members.iter().map(member_json).collect::<Vec<_>>(),
        "rules": rules_json(branch, list.rules.as_ref()),
        "publicUrl": "",
    })
}
