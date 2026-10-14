//! BK-10 (#1412) seeds for the re-pinned member-key tests: a member is a customer with a verified
//! ACTIVE `PHONE` identity, and `BookingMemberKeys` finds it through the Blueprint booking
//! (`bookings.transaction_id` → `customer_id`), lane controller rulings (b) and 3 on #1412
//! (issuecomment-6138038554, issuecomment-6140233542). Written by the ticket author
//! (claude/29df2ec5) with the up-front tests; do not edit, delete or weaken.
//!
//! The identity is sealed as every `PHONE` identity on `main` is: the E.164 form, AAD
//! `"{customer_id}PHONE"`, `lookup_hash = LookupHasher::hash(IdentityType::Phone, e164)`.
#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys};

/// The member's verified phone as `BookingMemberKeys` answers it: E.164 (ruling 3 on #1412).
pub const MEMBER_E164: &str = "+66812345678";
/// A phone typed at the desk and never verified (`ContactReserve`'s unverified identity).
pub const TYPED_E164: &str = "+66899999999";

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// The PII key of these tests, installed for the process so `BookingMemberKeys` can open the
/// identities the seeds seal.
pub fn keys() -> Arc<PiiKeys> {
    let keys = Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"));
    let _ = xc_customer::install_pii_keys(keys.clone());
    keys
}

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 13, 3, 0, 0).unwrap()
}

/// A `PHONE` identity of `customer_id` in its organisation, sealed as `main` seals them.
pub async fn seed_phone(
    pool: &PgPool,
    customer_id: &str,
    e164: &str,
    verified: bool,
    status: &str,
) -> Uuid {
    let keys = keys();
    let sealed = PiiCipher::new(keys.clone())
        .encrypt(format!("{customer_id}PHONE").as_bytes(), e164.as_bytes());
    let lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, e164);
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customer_identities ( \
             id, customer_id, organization_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status, created_at \
         ) SELECT $1::uuid, c.id, c.organization_id, 'PHONE', $2, $3, $4, $5, $6, $7 \
           FROM customers c WHERE c.id = $8",
    )
    .bind(id.to_string())
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&lookup_hash[..])
    .bind(verified.then(|| base() - Duration::minutes(1)))
    .bind(status)
    .bind(base() - Duration::minutes(30))
    .bind(customer_id)
    .execute(pool)
    .await
    .expect("seed a PHONE identity");
    id
}

/// A Blueprint booking of `customer_id` at `branch_id`, COMPLETED by `transaction_id`, on a slot
/// of its own.
pub async fn seed_completed_booking(
    pool: &PgPool,
    branch_id: &str,
    customer_id: &str,
    transaction_id: Uuid,
) {
    let slot = Uuid::new_v4();
    let start = base() + Duration::days(1) + Duration::minutes(rand_minutes(&slot));
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
             capacity, reserved, status) \
         SELECT $1::uuid, b.organization_id, b.id, $3, $4, $5, 4, 1, 'OPEN' FROM branches b WHERE b.id = $2",
    )
    .bind(slot.to_string())
    .bind(branch_id)
    .bind(start.date_naive())
    .bind(start)
    .bind(start + Duration::minutes(30))
    .execute(pool)
    .await
    .expect("insert a booking slot");
    sqlx::query(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
             shop_side, requested_currency, requested_quantity, source_channel, status, \
             qr_token_hash, transaction_id, created_by) \
         SELECT $1, b.organization_id, b.id, $3, $4, $5::uuid, 'SELL', 'USD', 100, 'DESK', 'COMPLETED', \
             $6, $7::uuid, 'test' FROM branches b WHERE b.id = $2",
    )
    .bind(format!("BK-{}", Uuid::new_v4()))
    .bind(branch_id)
    .bind(format!("B261014-{}", &slot.simple().to_string()[..8]))
    .bind(customer_id)
    .bind(slot.to_string())
    .bind(Uuid::new_v4().as_bytes().to_vec())
    .bind(transaction_id.to_string())
    .execute(pool)
    .await
    .expect("insert a Blueprint booking");
}

/// A start minute derived from the slot id, so two slots of one branch never share `start_at`.
fn rand_minutes(slot: &Uuid) -> i64 {
    let b = slot.as_bytes();
    i64::from(u16::from_be_bytes([b[0], b[1]])) * 30
}
