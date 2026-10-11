//! Ticket #672 (TX-4), G1 + G2 — the methods TX-4 adds to TX-1's repositories, against a real
//! PostgreSQL: a quote is accepted by one transaction or cancelled, and the organisation's quote
//! lifetime and a branch's approval limit are read and written.
//! Written before the implementation (claude/14cc24bd from a draft by claude/709ec749, on the
//! owner's order, 2026-10-10). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §0 T1 (the quote lifetime is ADMIN's,
//! default 300 s), T2 (a per-branch THB limit; none set ⇒ nothing waits), §2 D-5 (only ACTIVE /
//! ACCEPTED / CANCELLED are stored), §3.1 (`quotes`: `accepted_by_transaction_id` is set exactly
//! when ACCEPTED), §3.6 (`exchange_settings`, `branch_exchange_limits`; 30–3600 s; a limit is
//! NULL or > 0), §4.1 (the quote's states), §7 (`QuoteRepository::accept_in` / `cancel_in`).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Every call runs in a transaction the test opens and commits, refused or not, and the
//! rows are then read back, so "nothing written" is measured, not assumed.
//!
//! Signatures this test assumes ([required] in ticket G1/G2 unless marked "choice"; every method
//! is async, takes the caller's `&mut sqlx::Transaction<'_, Postgres>` first and answers
//! `AppResult`):
//! - `QuoteRepository::accept_in(tx, id: Uuid, transaction_id: Uuid) -> ()`: only an `ACTIVE`
//!   quote is accepted, else `staleVersion`;
//! - `QuoteRepository::cancel_in(tx, id: Uuid) -> ()`: `ACTIVE` or `ACCEPTED` becomes
//!   `CANCELLED`, else `staleVersion`;
//! - choice: an unknown quote id is `staleVersion` for both (there is no row in the asked state);
//! - `SettingsRepository::approval_limit_in(tx, organization_id: Uuid, branch: &BranchId) ->
//!   Option<Money>`: no row, or a NULL limit, is `None`;
//! - `SettingsRepository::set_ttl_in(tx, organization_id: Uuid, seconds: i32, by: &str, now:
//!   DateTime<Utc>) -> ()`; choice: outside 30–3600 is `validationFailed` and nothing is written;
//! - `SettingsRepository::set_approval_limit_in(tx, organization_id: Uuid, branch: &BranchId,
//!   limit: Option<Money>, by: &str, now: DateTime<Utc>) -> ()`; choice: a zero limit is
//!   `validationFailed`, and a branch that is not the organisation's is `outOfScope`, nothing
//!   written;
//! - TX-1's `get_ttl_in` reads what `set_ttl_in` wrote;
//! - `PgQuoteRepository::new(pool)` / `PgSettingsRepository::new(pool)` as on main;
//! - a refusal is `Err(AppError::Domain(m))` with `m` starting with its code, as TX-1's
//!   `QuoteService` answers (`workflows/exchange/quote.rs:62`).

mod quote_common;

use chrono::Duration;
use pos_error::{AppError, AppResult};
use quote_common::*;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

const ORG_2: &str = "00000000-0000-0000-0000-0000000000c2";

fn org(text: &str) -> Uuid {
    Uuid::parse_str(text).unwrap()
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

fn code<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m.split(':').next().unwrap_or("").trim().to_owned(),
        other => panic!("expected Err(AppError::Domain(\"<code>: …\")), got {other:?}"),
    }
}

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-front".to_owned(),
        username: "front".to_owned(),
        employee_id: Some("employee-front".to_owned()),
        role: Role::Front,
    }
}

struct Shop {
    db: TestDb,
    quotes: PgQuoteRepository,
    settings: PgSettingsRepository,
}

impl Shop {
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.publish("MAIN", &[USD]).await;
        Self {
            quotes: PgQuoteRepository::new(db.pool.clone()),
            settings: PgSettingsRepository::new(db.pool.clone()),
            db,
        }
    }

    /// An ACTIVE desk quote of 100 USD bought at MAIN.
    async fn quote(&self) -> Quote {
        let mut tx = self.db.pool.begin().await.unwrap();
        let q = QuoteService::new(self.db.pool.clone())
            .quote_in(
                &mut tx,
                QuoteRequest {
                    branch_id: BranchId::new("MAIN"),
                    customer_id: None,
                    channel: QuoteChannel::Desk,
                    lines: vec![QuoteRequestLine {
                        currency: CurrencyCode::parse("USD").unwrap(),
                        shop_side: Side::Buy,
                        quantity: ForeignAmount::from_decimal(Decimal::from(100)),
                        fee_thb: None,
                        denomination_group:
                            xc_rate::domain::currency::denomination_groups::GroupCode::all(),
                    }],
                },
                &teller(),
                whole_second_now(),
            )
            .await
            .expect("a quote");
        tx.commit().await.unwrap();
        q
    }

    /// "<status>|<accepted_by_transaction_id or empty>"
    async fn stored(&self, q: &Quote) -> String {
        self.db
            .text(&format!(
                "SELECT status || '|' || COALESCE(accepted_by_transaction_id::text, '') \
                 FROM quotes WHERE id = '{}'",
                q.id
            ))
            .await
            .expect("a quote row")
    }

    async fn accept(&self, id: Uuid, transaction: Uuid) -> AppResult<()> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self.quotes.accept_in(&mut tx, id, transaction).await;
        tx.commit().await.unwrap();
        got
    }

    async fn cancel(&self, id: Uuid) -> AppResult<()> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self.quotes.cancel_in(&mut tx, id).await;
        tx.commit().await.unwrap();
        got
    }

    async fn limit(&self, organization: &str, branch: &str) -> Option<Money> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .settings
            .approval_limit_in(&mut tx, org(organization), &BranchId::new(branch))
            .await
            .expect("approval_limit_in");
        tx.commit().await.unwrap();
        got
    }

    async fn set_limit(
        &self,
        organization: &str,
        branch: &str,
        limit: Option<&str>,
    ) -> AppResult<()> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .settings
            .set_approval_limit_in(
                &mut tx,
                org(organization),
                &BranchId::new(branch),
                limit.map(money),
                "admin-1",
                whole_second_now(),
            )
            .await;
        tx.commit().await.unwrap();
        got
    }

    async fn ttl(&self, organization: &str) -> i32 {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .settings
            .get_ttl_in(&mut tx, org(organization))
            .await
            .expect("get_ttl_in");
        tx.commit().await.unwrap();
        got
    }

    async fn set_ttl(&self, organization: &str, seconds: i32) -> AppResult<()> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .settings
            .set_ttl_in(
                &mut tx,
                org(organization),
                seconds,
                "admin-1",
                whole_second_now(),
            )
            .await;
        tx.commit().await.unwrap();
        got
    }
}

// ---- G1: the quote is accepted by one transaction, or cancelled ----

#[tokio::test]
async fn an_active_quote_is_accepted_by_one_transaction() {
    let shop = Shop::start().await;
    let q = shop.quote().await;
    let transaction = Uuid::new_v4();
    shop.accept(q.id, transaction).await.expect("accepted");
    assert_eq!(shop.stored(&q).await, format!("ACCEPTED|{transaction}"));

    assert_eq!(
        code(shop.accept(q.id, Uuid::new_v4()).await),
        "staleVersion",
        "an ACCEPTED quote is not accepted again"
    );
    assert_eq!(
        shop.stored(&q).await,
        format!("ACCEPTED|{transaction}"),
        "the first transaction keeps it"
    );
    shop.db.finish().await;
}

#[tokio::test]
async fn an_active_or_accepted_quote_is_cancelled_and_a_cancelled_one_is_stale() {
    let shop = Shop::start().await;
    let active = shop.quote().await;
    let accepted = shop.quote().await;
    shop.accept(accepted.id, Uuid::new_v4()).await.unwrap();

    shop.cancel(active.id).await.expect("ACTIVE ⇒ CANCELLED");
    shop.cancel(accepted.id)
        .await
        .expect("ACCEPTED ⇒ CANCELLED");
    for q in [&active, &accepted] {
        assert_eq!(
            shop.stored(q).await,
            "CANCELLED|",
            "§3.1: only an ACCEPTED quote names a transaction"
        );
    }

    assert_eq!(code(shop.cancel(active.id).await), "staleVersion");
    assert_eq!(
        code(shop.accept(active.id, Uuid::new_v4()).await),
        "staleVersion",
        "a CANCELLED quote is never accepted"
    );
    assert_eq!(shop.stored(&active).await, "CANCELLED|");
    shop.db.finish().await;
}

#[tokio::test]
async fn an_unknown_quote_is_neither_accepted_nor_cancelled() {
    let shop = Shop::start().await;
    assert_eq!(
        code(shop.accept(Uuid::new_v4(), Uuid::new_v4()).await),
        "staleVersion"
    );
    assert_eq!(code(shop.cancel(Uuid::new_v4()).await), "staleVersion");
    shop.db.finish().await;
}

/// Accepting is about the stored status only; whether the quote is still valid (D-5) is confirm's
/// question (§4.4 step 2), asked before it accepts.
#[tokio::test]
async fn accepting_does_not_rewrite_the_quotes_figures_or_expiry() {
    let shop = Shop::start().await;
    let q = shop.quote().await;
    let before = shop
        .db
        .text(&format!(
            "SELECT q.expires_at::text || '|' || q.publication_id::text || '|' || l.settle_thb::text \
             FROM quotes q JOIN quote_lines l ON l.quote_id = q.id WHERE q.id = '{}'",
            q.id
        ))
        .await;
    shop.accept(q.id, Uuid::new_v4()).await.unwrap();
    let after = shop
        .db
        .text(&format!(
            "SELECT q.expires_at::text || '|' || q.publication_id::text || '|' || l.settle_thb::text \
             FROM quotes q JOIN quote_lines l ON l.quote_id = q.id WHERE q.id = '{}'",
            q.id
        ))
        .await;
    assert_eq!(before, after);
    shop.db.finish().await;
}

// ---- G2: the branch limit and the quote lifetime ----

#[tokio::test]
async fn no_limit_row_or_a_null_limit_reads_as_none() {
    let shop = Shop::start().await;
    shop.db.branch("B2").await;
    assert_eq!(
        shop.limit(ORG_1, "MAIN").await,
        None,
        "no row: T2, nothing waits"
    );

    shop.set_limit(ORG_1, "MAIN", Some("50000.00"))
        .await
        .expect("set");
    assert_eq!(shop.limit(ORG_1, "MAIN").await, Some(money("50000.00")));
    assert_eq!(shop.limit(ORG_1, "B2").await, None, "per branch");

    shop.set_limit(ORG_1, "MAIN", None).await.expect("cleared");
    assert_eq!(shop.limit(ORG_1, "MAIN").await, None, "a NULL limit");
    shop.db.finish().await;
}

#[tokio::test]
async fn setting_a_limit_replaces_the_branchs_one_row_and_records_who_and_when() {
    let shop = Shop::start().await;
    shop.set_limit(ORG_1, "MAIN", Some("50000.00"))
        .await
        .unwrap();
    shop.set_limit(ORG_1, "MAIN", Some("75000.50"))
        .await
        .unwrap();
    assert_eq!(shop.limit(ORG_1, "MAIN").await, Some(money("75000.50")));
    assert_eq!(
        shop.db
            .count("SELECT count(*) FROM branch_exchange_limits WHERE branch_id = 'MAIN'")
            .await,
        1
    );
    assert_eq!(
        shop.db
            .text(
                "SELECT updated_by || '|' || (updated_at IS NOT NULL)::text \
                 FROM branch_exchange_limits WHERE branch_id = 'MAIN'"
            )
            .await
            .as_deref(),
        Some("admin-1|true")
    );
    shop.db.finish().await;
}

#[tokio::test]
async fn a_zero_limit_or_another_organisations_branch_is_refused_and_writes_nothing() {
    let shop = Shop::start().await;
    shop.db.second_organisation(ORG_2, "ORG2", "X1").await;

    assert_eq!(
        code(shop.set_limit(ORG_1, "MAIN", Some("0.00")).await),
        "validationFailed",
        "§3.6: a limit is NULL or > 0"
    );
    assert_eq!(
        code(shop.set_limit(ORG_1, "X1", Some("1000.00")).await),
        "outOfScope",
        "X1 is ORG_2's branch"
    );
    assert_eq!(
        shop.db
            .count("SELECT count(*) FROM branch_exchange_limits")
            .await,
        0
    );
    assert_eq!(shop.limit(ORG_2, "MAIN").await, None, "MAIN is not ORG_2's");
    shop.db.finish().await;
}

#[tokio::test]
async fn the_quote_lifetime_is_set_per_organisation_within_30_to_3600_seconds() {
    let shop = Shop::start().await;
    shop.db.second_organisation(ORG_2, "ORG2", "X1").await;
    assert_eq!(shop.ttl(ORG_1).await, 300, "T1: no row ⇒ 300 s");

    shop.set_ttl(ORG_1, 120).await.expect("set");
    assert_eq!(shop.ttl(ORG_1).await, 120);
    assert_eq!(shop.ttl(ORG_2).await, 300, "per organisation");
    shop.set_ttl(ORG_1, 30).await.expect("the lower bound");
    shop.set_ttl(ORG_1, 3600).await.expect("the upper bound");
    assert_eq!(shop.ttl(ORG_1).await, 3600);

    for seconds in [29, 3601, 0, -1] {
        assert_eq!(
            code(shop.set_ttl(ORG_1, seconds).await),
            "validationFailed",
            "{seconds} s"
        );
    }
    assert_eq!(shop.ttl(ORG_1).await, 3600, "a refusal changes nothing");
    assert_eq!(
        shop.db
            .count("SELECT count(*) FROM exchange_settings")
            .await,
        1
    );
    shop.db.finish().await;
}

/// TX-1's `quote_in` reads the lifetime `set_ttl_in` wrote (T1).
#[tokio::test]
async fn a_quote_after_the_lifetime_changes_expires_at_the_new_lifetime() {
    let shop = Shop::start().await;
    shop.set_ttl(ORG_1, 60).await.unwrap();
    let q = shop.quote().await;
    assert_eq!(q.expires_at - q.created_at, Duration::seconds(60));
    shop.db.finish().await;
}
