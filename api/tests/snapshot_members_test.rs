//! Written before the implementation (ticket T15-g, #412). Do not edit, delete or weaken.
//!
//! The member flag of `adminSnapshot`: `ServiceSnapshotSources::members` reads
//! the `members` rows of the branch asked about through `PgMemberRepository`
//! (design `docs/design/xchange-v3.md` §5.6.1 *Member flag*, §5.6.5
//! `ServiceSnapshotSources --> PgMemberRepository`, §5.6.8, §10 row T15-g).
//!
//! v2 (`108-Plaza/xchange-v2-source` `4d0bdbb`, `src/online/lib/ledger.ts:37`):
//! `SELECT customer_id,verified_at FROM members WHERE branch_id=?`, then every
//! customer gets `member` (has a row of that branch) and `memberVerifiedAt`
//! (that row's `verified_at`, else `null`). Re-issuing a member replaces the row
//! of (branch, customer) in place (`src/online/lib/bookings.ts:80`).
//!
//! Members are written through `PgMemberRepository::upsert_in` (T15-b), as the
//! `member` action will write them. The sources are built exactly as the routes
//! build them: `ServiceSnapshotSources::new(transactions, accounts)` — its
//! signature does not change (§10 [required]).
//!
//! Re-pinned for BK-10 #1412 by the ticket author claude/29df2ec5 (lane controller ruling 1,
//! issuecomment-6140233542): a member is now a customer with a verified ACTIVE `PHONE` identity,
//! company-wide (booking-slots design §6 `members` row, PR 639 K1). Members are written through
//! `ContactVerify::verify_in` (BK-10's writer, `customerContactVerify`); a v2 `members` row no
//! longer marks anybody. Each test keeps its purpose.
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.

mod compat_common;
mod save_common;
mod snapshot_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use save_common::{MAIN, Shop, TransactionService, at, branch, person};
use serde_json::{Value, json};
use snapshot_common::{NoPorts, RecordedSource};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_customer::application::customer::contact_verify::{ContactSummary, ContactVerify};
use xc_exchange::domain::booking::entities::member::Member;
use xc_exchange::domain::booking::repositories::member_repository::MemberRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::compat::snapshot::{SnapshotBuilder, write_answer};
use xchange::application::compat::snapshot_sources::{MemberMark, SnapshotSources};
use xchange::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;

/// The second branch of these tests.
const B2: &str = "XC-B02";

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(pos_infra::transactions::Transactor::new(shop.pool.clone()))
}

/// The sources as the routes build them (`presentation/http/rpc.rs`): over the
/// two services, on the shop's own database. Nothing else is handed in.
fn sources(shop: &Shop) -> ServiceSnapshotSources {
    ServiceSnapshotSources::new(
        TransactionService::new(
            Arc::new(PgTransactionRepository::new(shop.pool.clone())),
            branch_tx(shop),
        ),
        AccountService::new(
            Arc::new(PgAccountRepository::new(shop.pool.clone())),
            PasswordHasher::with_cost(8, 1, 1),
        ),
    )
}

/// The snapshot builder over the real sources, as `snapshot_sources_test.rs`
/// builds it.
fn builder(shop: &Shop) -> SnapshotBuilder {
    let custody = CustodyService::new(
        Arc::new(PgCustodyRepository::new(shop.pool.clone())),
        shop.shifts.clone(),
        Arc::new(NoPorts),
        branch_tx(shop),
    );
    SnapshotBuilder::new(
        BranchService::new(branch_tx(shop)),
        RateService::new(branch_tx(shop), Arc::new(RecordedSource::default())),
        shop.customers.clone(),
        shop.shifts.clone(),
        custody,
        Arc::new(sources(shop)),
    )
}

/// A LIVE customer of `branch_id` with a passport number of its own.
async fn customer(shop: &Shop, branch_id: &str, passport_no: &str) -> String {
    let mut who = person();
    who.passport_no = passport_no.to_owned();
    shop.reserve(branch_id, who).await
}

/// Issue (or re-issue) a member through the repository T15-b wrote, in a
/// transaction of its own, committed.
async fn issue(
    shop: &Shop,
    id: &str,
    branch_id: &str,
    customer_id: &str,
    token_hash: &str,
    verified_at: DateTime<Utc>,
) -> Member {
    let repo = PgMemberRepository::new(shop.pool.clone());
    let member = Member {
        id: id.to_owned(),
        branch_id: branch(branch_id),
        customer_id: customer_id.to_owned(),
        name: "Somchai Jaidee".to_owned(),
        phone: "0812345678".to_owned(),
        token_hash: token_hash.to_owned(),
        verified_at,
        verified_by: "owner".to_owned(),
    };
    let mut tx = shop.pool.begin().await.expect("begin a transaction");
    let stored = repo
        .upsert_in(&mut tx, &member)
        .await
        .unwrap_or_else(|e| panic!("upsert_in {id}: {e:?}"))
        .unwrap_or_else(|taken| panic!("upsert_in {id}: {taken:?}"));
    tx.commit().await.expect("commit the member");
    stored
}

/// Verify `phone` as `customer_id`'s phone at `branch_id` through BK-10's writer, as a Teller of
/// that branch, at `verified_at`, in a transaction of its own, committed.
async fn verify(
    shop: &Shop,
    branch_id: &str,
    customer_id: &str,
    phone: &str,
    verified_at: DateTime<Utc>,
) -> ContactSummary {
    let who = User::new(
        "00000000-0000-4000-8000-0000000000aa",
        "00000000-0000-0000-0000-000000000001",
        vec![ScopedRole::new(
            Role::Teller,
            Some(branch(branch_id)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    );
    let mut tx = shop.pool.begin().await.expect("begin a transaction");
    let summary = ContactVerify::new(branch_tx(shop))
        .verify_in(
            &mut tx,
            &who,
            &branch(branch_id),
            customer_id,
            phone,
            verified_at,
        )
        .await
        .unwrap_or_else(|e| panic!("verify {phone} for {customer_id}: {e:?}"));
    tx.commit().await.expect("commit the verification");
    summary
}

/// The admin snapshot of `here`, as JSON.
async fn snapshot(shop: &Shop, here: &BranchId) -> Value {
    builder(shop)
        .admin_snapshot(here, "owner", at(600))
        .await
        .unwrap_or_else(|e| panic!("the snapshot of {}: {e:?}", here.as_str()))
        .to_value()
}

/// The printed customer whose `id` is `customer_id` — found by id, never by
/// position.
fn printed<'a>(snapshot: &'a Value, customer_id: &str) -> &'a Value {
    snapshot["customers"]
        .as_array()
        .unwrap_or_else(|| panic!("`customers` is not an array in {snapshot}"))
        .iter()
        .find(|row| row["id"] == customer_id)
        .unwrap_or_else(|| panic!("no customer {customer_id} in {}", snapshot["customers"]))
}

/// The two member fields of a printed customer, `null` for a missing key.
fn flag(row: &Value) -> Value {
    json!({ "member": row["member"], "memberVerifiedAt": row["memberVerifiedAt"] })
}

/// Marks in a fixed order (the trait answers them "in any order").
fn sorted(mut marks: Vec<MemberMark>) -> Vec<MemberMark> {
    marks.sort_by(|a, b| a.customer_id.cmp(&b.customer_id));
    marks
}

/// §5.6.8 row *the admin snapshot after `member`*: the member is marked with
/// its verification time (milliseconds, `Z`), the customer beside it is not;
/// and the six write answers still strip both fields (`snapshot.rs:183`), the
/// member's included. BK-10: the member is the customer whose phone was verified.
#[tokio::test]
async fn a_member_of_the_branch_is_marked_and_the_write_answers_strip_both_fields() {
    let shop = Shop::start().await;
    let member = customer(&shop, MAIN, "AA1000001").await;
    let walk_in = customer(&shop, MAIN, "AA1000002").await;
    verify(&shop, MAIN, &member, "081-234-5678", at(100)).await;

    let marks = sources(&shop)
        .members(&branch(MAIN))
        .await
        .expect("the members of MAIN");
    assert_eq!(
        marks,
        vec![MemberMark {
            customer_id: member.clone(),
            verified_at: Some(at(100)),
        }]
    );

    let shown = snapshot(&shop, &branch(MAIN)).await;
    assert_eq!(
        flag(printed(&shown, &member)),
        json!({ "member": true, "memberVerifiedAt": "2026-10-06T05:01:40.123Z" }),
        "the member"
    );
    assert_eq!(
        flag(printed(&shown, &walk_in)),
        json!({ "member": false, "memberVerifiedAt": null }),
        "the customer who is not a member"
    );

    let answer = write_answer(
        &builder(&shop)
            .admin_snapshot(&branch(MAIN), "owner", at(600))
            .await
            .expect("the snapshot of MAIN"),
    )
    .to_value();
    for id in [&member, &walk_in] {
        let row = printed(&answer, id)
            .as_object()
            .unwrap_or_else(|| panic!("customer {id} is not an object"));
        assert!(
            !row.contains_key("member") && !row.contains_key("memberVerifiedAt"),
            "a write answer strips both member fields of {id}: {row:?}"
        );
    }
    shop.finish().await;
}

/// Which rows mark a member of a branch. v2: per branch (§5.6.1 *Member flag*: "from `members` of
/// the branch"). BK-10 (design §6, PR 639 K1): company-wide — a phone verified at B2 marks its
/// customer in MAIN's snapshot and in B2's, and a v2 `members` row marks nobody, here or there.
#[tokio::test]
async fn a_verified_phone_marks_its_customer_at_every_branch_and_a_members_row_marks_nobody() {
    let shop = Shop::start().await;
    shop.seed_branch(B2, "Second").await;
    let at_main = customer(&shop, MAIN, "AA2000001").await;
    let at_b2 = customer(&shop, B2, "AA2000002").await;
    // a v2 member row of B2 for B2's customer: no longer a member anywhere
    issue(&shop, "MB-b2-own", B2, &at_b2, "hash-b2-own", at(200)).await;
    // MAIN's customer's phone, verified at B2
    verify(&shop, B2, &at_main, "081-234-5678", at(100)).await;

    let sources = sources(&shop);
    for here in [MAIN, B2] {
        assert_eq!(
            sorted(
                sources
                    .members(&branch(here))
                    .await
                    .unwrap_or_else(|e| panic!("the members of {here}: {e:?}"))
            ),
            vec![MemberMark {
                customer_id: at_main.clone(),
                verified_at: Some(at(100)),
            }],
            "{here}: the verified phone's customer, and only that customer"
        );
    }

    let main_shown = snapshot(&shop, &branch(MAIN)).await;
    assert_eq!(
        flag(printed(&main_shown, &at_main)),
        json!({ "member": true, "memberVerifiedAt": "2026-10-06T05:01:40.123Z" }),
        "a phone verified at B2 marks its customer in MAIN"
    );

    let b2_shown = snapshot(&shop, &branch(B2)).await;
    assert_eq!(
        flag(printed(&b2_shown, &at_b2)),
        json!({ "member": false, "memberVerifiedAt": null }),
        "a v2 members row of B2 marks nobody in B2"
    );
    shop.finish().await;
}

/// A member re-issued — BK-10: the customer's phone verified again with a new number (ruling (a)
/// on #1412: the old identity is revoked, the new one is ACTIVE) — shows the new verification
/// time: one mark, never two.
#[tokio::test]
async fn a_reissued_member_shows_the_new_verification_time() {
    let shop = Shop::start().await;
    let member = customer(&shop, MAIN, "AA3000001").await;
    verify(&shop, MAIN, &member, "081-234-5678", at(100)).await;
    assert_eq!(
        flag(printed(&snapshot(&shop, &branch(MAIN)).await, &member)),
        json!({ "member": true, "memberVerifiedAt": "2026-10-06T05:01:40.123Z" }),
        "after the first verification"
    );

    verify(&shop, MAIN, &member, "089-876-5432", at(300)).await;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM customer_identities \
         WHERE customer_id = $1 AND type = 'PHONE' AND status = 'ACTIVE'",
    )
    .bind(&member)
    .fetch_one(&shop.pool)
    .await
    .expect("count the ACTIVE phones");
    assert_eq!(active, 1, "one ACTIVE phone after the re-verification");

    assert_eq!(
        sources(&shop)
            .members(&branch(MAIN))
            .await
            .expect("the members of MAIN"),
        vec![MemberMark {
            customer_id: member.clone(),
            verified_at: Some(at(300)),
        }]
    );
    assert_eq!(
        flag(printed(&snapshot(&shop, &branch(MAIN)).await, &member)),
        json!({ "member": true, "memberVerifiedAt": "2026-10-06T05:05:00.123Z" }),
        "after the re-verification"
    );
    shop.finish().await;
}

/// §7 [required]: a database failure is an `AppError`, never an empty list —
/// a snapshot that cannot read its members does not show everybody as a
/// non-member.
#[tokio::test]
async fn a_members_read_that_fails_is_an_error_not_nobody() {
    let shop = Shop::start().await;
    let member = customer(&shop, MAIN, "AA4000001").await;
    verify(&shop, MAIN, &member, "081-234-5678", at(100)).await;
    // BK-10: the members source reads the customers' identities
    sqlx::query("ALTER TABLE customer_identities RENAME TO customer_identities_unreadable")
        .execute(&shop.pool)
        .await
        .expect("hide the customer_identities table");

    match sources(&shop).members(&branch(MAIN)).await {
        Err(_) => {}
        Ok(marks) => panic!("the members read must fail, it answered {marks:?}"),
    }
    match builder(&shop)
        .admin_snapshot(&branch(MAIN), "owner", at(600))
        .await
    {
        Err(_) => {}
        Ok(shown) => panic!(
            "the snapshot must fail, it answered customers {}",
            shown.to_value()["customers"]
        ),
    }

    sqlx::query("ALTER TABLE customer_identities_unreadable RENAME TO customer_identities")
        .execute(&shop.pool)
        .await
        .expect("put the customer_identities table back");
    shop.finish().await;
}
