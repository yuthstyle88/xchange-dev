//! Ticket #601 (RE-4) — what the RE-4 tests share: publishing a rate set through the rate engine
//! (RE-1, merged) and reading the line a reader must have used.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.1 (tables), §4.6 (OWNER publishes an out-of-bound
//! set directly, A5), §4.8 (every reader reads the current publication through
//! `PublishedRates::line_in`), §9 RE-4.
//!
//! Rates are published the way the owner publishes them: a draft of MANUAL lines (no policy is
//! seeded, so every line is out-of-bound, §4.5), submitted by the owner (closed at submit by
//! `ApprovalService::decide_self`, §4.6), then published with the branch's current version as
//! `expected_version`. Nothing here writes a publication by SQL.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips (the callers' harnesses check it).
//!
//! Signatures this module assumes (all merged in RE-1/RE-3, #598):
//! - `xchange::presentation::http::v3::commands::make_approval_service(PgPool)
//!   -> Arc<ApprovalService>`, with the rate subject registered;
//! - `RateDraftService::{new, create, submit}` and `PublishService::{new, publish}` as
//!   `crates/xc-rate/tests/rate_draft_flow_test.rs` names them;
//! - `DraftLine::new(CurrencyCode, LineSide, LineSide, Option<String>)`, `LineSide::Manual(Rate)`.
#![allow(dead_code)]

use std::str::FromStr;

use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xchange::presentation::http::v3::commands::make_approval_service;

pub fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{text} is a decimal: {e}"))
}

/// The publication a branch points at after a publish: `(id, version)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub id: String,
    pub version: i32,
}

/// One line of a branch's current publication, as stored (§4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    pub unit_size: Decimal,
}

/// Publish `lines` (`(currency, buy, sell)`, each rate per the currency's `unit_size`) as the
/// next publication of `branch`, as the owner. Answers the new publication.
pub async fn publish(pool: &PgPool, branch: &str, lines: &[(&str, &str, &str)]) -> Published {
    let owner = Actor::owner("owner");
    let b = BranchId::new(branch);
    let drafts = RateDraftService::new(pool.clone(), make_approval_service(pool.clone()));
    let publisher = PublishService::new(pool.clone());
    let draft_lines: Vec<DraftLine> = lines
        .iter()
        .map(|(code, buy, sell)| {
            DraftLine::new(
                CurrencyCode::parse(code).unwrap_or_else(|_| panic!("{code} is a currency")),
                LineSide::Manual(Rate::parse(buy).unwrap_or_else(|e| panic!("{buy}: {e:?}"))),
                LineSide::Manual(Rate::parse(sell).unwrap_or_else(|e| panic!("{sell}: {e:?}"))),
                Some("published by the RE-4 test".to_owned()),
            )
        })
        .collect();

    let mut tx = pool.begin().await.expect("begin");
    let draft = drafts
        .create(&mut tx, &b, &owner, draft_lines)
        .await
        .unwrap_or_else(|e| panic!("create the draft: {}", e.code()));
    tx.commit().await.expect("commit the draft");

    let mut tx = pool.begin().await.expect("begin");
    drafts
        .submit(&mut tx, &b, &owner, &draft.id, &unique_key())
        .await
        .unwrap_or_else(|e| panic!("submit the draft: {}", e.code()));
    tx.commit().await.expect("commit the submit");

    let expected: Option<i32> = sqlx::query_scalar(
        "SELECT p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the current version");

    let mut tx = pool.begin().await.expect("begin");
    let published = publisher
        .publish(&mut tx, &b, &owner, &draft.id, expected)
        .await
        .unwrap_or_else(|e| panic!("publish the draft: {}", e.code()));
    tx.commit().await.expect("commit the publish");

    let now = current(pool, branch)
        .await
        .expect("the branch points at it");
    assert_eq!(
        now.version, published.version,
        "the branch points at the publication just made"
    );
    now
}

/// The branch's current publication, or `None` before its first.
pub async fn current(pool: &PgPool, branch: &str) -> Option<Published> {
    sqlx::query_as::<_, (String, i32)>(
        "SELECT p.id::text, p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the current publication")
    .map(|(id, version)| Published { id, version })
}

/// The `ALL` line of `code` in `branch`'s current publication.
pub async fn line(pool: &PgPool, branch: &str, code: &str) -> Line {
    let (id, buy, sell, unit_size): (String, Option<Decimal>, Option<Decimal>, Decimal) =
        sqlx::query_as(
            "SELECT l.id::text, l.buy_rate, l.sell_rate, l.unit_size \
             FROM branches b JOIN published_rate_lines l \
               ON l.publication_id = b.current_publication_id \
             WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = 'ALL'",
        )
        .bind(branch)
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("the current {code} line of {branch}: {e}"));
    Line {
        id,
        buy,
        sell,
        unit_size,
    }
}

fn unique_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    format!(
        "re4-key-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
