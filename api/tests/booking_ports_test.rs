//! Written before the implementation (ticket T15-e, #411). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! `BookingRulesService` and `PgBookingPorts`, design `docs/design/xchange-v3.md` §5.6.2
//! (`booking_rules` moved to the setting store), §5.6.3 (`settings`, `list`, `create` refusals
//! 8–11), §5.6.4 (*Per branch everywhere*), §5.6.5 (*Dependency rule*, `BookingRulesService`,
//! `PgBookingPorts`), §5.5.1 (the setting store) and §5.6.8 (rows marked T15-e).
//!
//! - The rules are the branch's `branch_settings` document `bookingRules`, body exactly
//!   `{"enabled": <bool>, "minutes": <integer>}`. No document reads `None`; the default
//!   (`BookingRules::DEFAULT`, enabled, 30 minutes) is the reader's to apply. Reading writes
//!   nothing.
//! - `save` runs in the branch's `BranchTx::run`, sends no version and wins over whatever is
//!   stored (§5.6.3 `settings`: "last write wins (the screen sends no version)"): §5.6 has no
//!   optimistic version conflict, so a save never answers one. Each save is a new version of the
//!   document and a history row (§5.5.1); the first save's history `before` is JSON `null`
//!   (lane controller, 2026-10-08, on #411).
//! - A stored document that no longer parses as `BookingRules` is corruption — every write
//!   validates — so reading it answers an error, never `None` or the default (lane controller,
//!   2026-10-08, on #411).
//! - `rules_in` reads the caller's transaction.
//! - `PgBookingPorts` answers per branch: `branch_exists`; `live_customer` only for a LIVE
//!   customer of that branch; `has_open_shift_in` only for that branch's OPEN shift;
//!   `counter_rate_in` the branch's latest counter buy (BUY) or sell (SELL) rate of that
//!   currency, `None` when the branch has none.
//!
//! The ports are called through T15-a's `BookingPorts` trait object, so the signatures are
//! T15-a's. What the ticket names: `BookingRulesService::new(BranchTx, BranchSettingStore)`,
//! `PgBookingPorts::new(PgPool)`. What the tests assume where the ticket is silent, as
//! `PaymentSettingsService::save` does, confirmed by the lane controller on #411: `save(&BranchId,
//! BookingRules, actor: &str, DateTime<Utc>)` answers an `AppResult<_>`, and `actor` is stored as
//! `updated_by` and the history's `actor`.
//!
//! Times are fixed instants, never a clock; history is ordered by its version.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use snapshot_common::{TestDb, branch, time};
use sqlx::{Postgres, Transaction};
use xc_exchange::domain::booking::entities::booking_steps::LiveCustomer;
use xc_exchange::domain::booking::repositories::booking_ports::BookingPorts;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::{CurrencyCode, Rate, Side};
use xchange::application::bookings::booking_rules_service::BookingRulesService;
use xchange::infrastructure::bookings::booking_ports::PgBookingPorts;
use xchange::infrastructure::settings::branch_setting_store::BranchSettingStore;

const MAIN: &str = "MAIN";
const AIRPORT: &str = "AIRPORT";
const NAME: &str = "bookingRules";

fn at(n: i64) -> DateTime<Utc> {
    time("2026-10-08T03:00:00.000Z") + Duration::seconds(n)
}

fn rules(enabled: bool, minutes: u16) -> BookingRules {
    BookingRules { enabled, minutes }
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a rate")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency")
}

struct Shop {
    db: TestDb,
    branch_tx: BranchTx,
    store: BranchSettingStore,
    service: BookingRulesService,
    ports: Box<dyn BookingPorts>,
}

impl Shop {
    /// `MAIN` and `AIRPORT`, nothing else.
    async fn open() -> Shop {
        let db = TestDb::start().await;
        db.seed_branch(AIRPORT, "Airport").await;
        let branch_tx = db.branch_tx();
        let store = BranchSettingStore::new(db.pool.clone());
        let service = BookingRulesService::new(db.branch_tx(), store.clone());
        let ports: Box<dyn BookingPorts> = Box::new(PgBookingPorts::new(db.pool.clone()));
        Shop {
            db,
            branch_tx,
            store,
            service,
            ports,
        }
    }

    async fn begin(&self) -> Transaction<'static, Postgres> {
        self.db.pool.begin().await.expect("begin a transaction")
    }

    async fn save(&self, branch_id: &str, r: BookingRules, actor: &str, when: DateTime<Utc>) {
        self.service
            .save(&branch(branch_id), r, actor, when)
            .await
            .unwrap_or_else(|e| panic!("save the rules of {branch_id}: {e:?}"));
    }

    /// What the service and the port both answer, committed; they must agree.
    async fn rules_of(&self, branch_id: &str) -> Option<BookingRules> {
        let b = branch(branch_id);
        let from_service = self.service.rules(&b).await.expect("service rules");
        let from_port = self.ports.rules(&b).await.expect("port rules");
        assert_eq!(
            from_service, from_port,
            "the port reads the rules through the service"
        );
        from_service
    }

    /// What `rules_in` of the service and of the port both answer in a fresh transaction.
    async fn rules_in_of(&self, branch_id: &str) -> Option<BookingRules> {
        let b = branch(branch_id);
        let mut tx = self.begin().await;
        let from_service = self
            .service
            .rules_in(&mut tx, &b)
            .await
            .expect("service rules_in");
        let from_port = self
            .ports
            .rules_in(&mut tx, &b)
            .await
            .expect("port rules_in");
        tx.rollback().await.expect("rollback");
        assert_eq!(from_service, from_port);
        from_service
    }

    async fn documents(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM branch_settings WHERE name = $1")
            .bind(NAME)
            .fetch_one(&self.db.pool)
            .await
            .expect("count documents")
    }

    async fn seed_customer(&self, id: &str, branch_id: &str, name: &str, mode: &str) {
        sqlx::query(
            "INSERT INTO customers (id, branch_id, name, passport_no, nationality, mode, created_at) \
             VALUES ($1, $2, $3, $4, 'THA', $5, $6)",
        )
        .bind(id)
        .bind(branch_id)
        .bind(name)
        .bind(format!("P-{id}"))
        .bind(mode)
        .bind(at(0))
        .execute(&self.db.pool)
        .await
        .expect("insert into customers");
    }

    async fn seed_shift(&self, id: &str, branch_id: &str, status: &str) {
        let closed = status == "CLOSED";
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, \
                 open_request_id) \
             VALUES ($1, $2, $3, $4, 'owner', $5, $6, $7)",
        )
        .bind(id)
        .bind(branch_id)
        .bind(status)
        .bind(at(0))
        .bind(closed.then(|| at(10)))
        .bind(closed.then_some("owner"))
        .bind(format!("open-{id}"))
        .execute(&self.db.pool)
        .await
        .expect("insert into shifts");
    }

    async fn seed_rate(&self, branch_id: &str, currency: &str, buy: &str, sell: &str, n: i64) {
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
             VALUES ($1, $2, $3::numeric, $4::numeric, $5, 'owner', $6)",
        )
        .bind(branch_id)
        .bind(currency)
        .bind(buy)
        .bind(sell)
        .bind(at(n))
        .bind(format!("rate-{branch_id}-{currency}-{n}"))
        .execute(&self.db.pool)
        .await
        .expect("insert into rates");
    }

    async fn open_shift(&self, branch_id: &str) -> bool {
        let mut tx = self.begin().await;
        let open = self
            .ports
            .has_open_shift_in(&mut tx, &branch(branch_id))
            .await
            .expect("has_open_shift_in");
        tx.rollback().await.expect("rollback");
        open
    }

    async fn counter(&self, branch_id: &str, currency: &str, side: Side) -> Option<Rate> {
        let c = code(currency);
        let mut tx = self.begin().await;
        let got = self
            .ports
            .counter_rate_in(&mut tx, &branch(branch_id), &c, side)
            .await
            .expect("counter_rate_in");
        tx.rollback().await.expect("rollback");
        got
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

// ── the rules (BookingRulesService, and the port's rules / rules_in) ─────────

/// §5.6.8: "no `bookingRules` stored — `rules` answers `None`; a reader applies 30 minutes".
#[tokio::test]
async fn rules_default() {
    let shop = Shop::open().await;
    assert_eq!(shop.rules_of(MAIN).await, None);
    assert_eq!(shop.rules_in_of(MAIN).await, None);
    assert_eq!(
        shop.rules_of(MAIN).await.unwrap_or(BookingRules::DEFAULT),
        rules(true, 30),
        "what create and list fall back to"
    );
    assert_eq!(shop.documents().await, 0, "reading writes nothing");
    shop.finish().await;
}

/// The document is `bookingRules`, body exactly `{"enabled", "minutes"}`, written by `actor` at
/// `now` as version 1.
#[tokio::test]
async fn save_writes_the_branch_document_and_rules_read_it_back() {
    let shop = Shop::open().await;
    shop.save(MAIN, rules(false, 45), "owner", at(0)).await;

    assert_eq!(shop.rules_of(MAIN).await, Some(rules(false, 45)));
    assert_eq!(shop.rules_in_of(MAIN).await, Some(rules(false, 45)));
    let stored = shop
        .store
        .read(&branch(MAIN), NAME)
        .await
        .expect("read the store")
        .expect("a bookingRules document");
    assert_eq!(stored.body, json!({ "enabled": false, "minutes": 45 }));
    assert_eq!(stored.version, 1);
    assert_eq!(stored.updated_by, "owner");
    assert_eq!(stored.updated_at, at(0));
    assert_eq!(shop.documents().await, 1);
    shop.finish().await;
}

/// §5.6.3 `settings`: last write wins, no version is sent; each save is a version and a history
/// row (§5.5.1), newest first.
#[tokio::test]
async fn save_twice_the_later_wins_and_history_keeps_both() {
    let shop = Shop::open().await;
    shop.save(MAIN, rules(false, 45), "owner", at(0)).await;
    shop.save(MAIN, rules(true, 1440), "owner-2", at(60)).await;

    assert_eq!(shop.rules_of(MAIN).await, Some(rules(true, 1440)));
    let stored = shop
        .store
        .read(&branch(MAIN), NAME)
        .await
        .expect("read the store")
        .expect("the document");
    assert_eq!(stored.version, 2);
    assert_eq!(stored.body, json!({ "enabled": true, "minutes": 1440 }));
    let history = shop
        .store
        .history(&branch(MAIN), NAME, 10)
        .await
        .expect("history");
    let rows: Vec<(i64, String, DateTime<Utc>, Value)> = history
        .into_iter()
        .map(|c| (c.version, c.actor, c.at, c.after))
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                2,
                "owner-2".to_string(),
                at(60),
                json!({ "enabled": true, "minutes": 1440 })
            ),
            (
                1,
                "owner".to_string(),
                at(0),
                json!({ "enabled": false, "minutes": 45 })
            ),
        ]
    );
    shop.finish().await;
}

/// The first save has nothing before it: its history row's `before` is JSON `null`; the next
/// save's `before` is the first save's body.
#[tokio::test]
async fn the_first_save_records_a_null_before() {
    let shop = Shop::open().await;
    shop.save(MAIN, rules(false, 45), "owner", at(0)).await;
    shop.save(MAIN, rules(true, 90), "owner", at(60)).await;
    let history = shop
        .store
        .history(&branch(MAIN), NAME, 10)
        .await
        .expect("history");
    let before: Vec<(i64, Value)> = history.into_iter().map(|c| (c.version, c.before)).collect();
    assert_eq!(
        before,
        vec![
            (2, json!({ "enabled": false, "minutes": 45 })),
            (1, Value::Null),
        ]
    );
    shop.finish().await;
}

/// A stored `bookingRules` that `BookingRules::parse` refuses is corruption (every save
/// validates): `rules` and `rules_in`, of the service and of the port, answer an error — never
/// `None`, never the default.
#[tokio::test]
async fn a_stored_document_that_no_longer_parses_is_an_error() {
    let shop = Shop::open().await;
    let corrupt = [
        json!({ "enabled": 1, "minutes": 30 }),
        json!({ "enabled": true, "minutes": 3 }),
        json!({ "enabled": true, "minutes": "30" }),
        json!({ "enabled": true }),
        json!([]),
    ];
    for (version, body) in corrupt.iter().enumerate() {
        let mut tx = shop.begin().await;
        shop.store
            .write_in(
                &mut tx,
                &branch(MAIN),
                NAME,
                version as i64,
                &Value::Null,
                body,
                "by hand",
                at(version as i64),
            )
            .await
            .expect("write")
            .expect("no conflict");
        tx.commit().await.expect("commit");

        let b = branch(MAIN);
        assert!(
            shop.service.rules(&b).await.is_err(),
            "service rules of {body}"
        );
        assert!(shop.ports.rules(&b).await.is_err(), "port rules of {body}");
        let mut tx = shop.begin().await;
        assert!(
            shop.service.rules_in(&mut tx, &b).await.is_err(),
            "service rules_in of {body}"
        );
        tx.rollback().await.expect("rollback");
        let mut tx = shop.begin().await;
        assert!(
            shop.ports.rules_in(&mut tx, &b).await.is_err(),
            "port rules_in of {body}"
        );
        tx.rollback().await.expect("rollback");
    }
    assert_eq!(
        shop.rules_of(AIRPORT).await,
        None,
        "another branch is untouched"
    );
    shop.finish().await;
}

/// No version conflict exists for these rules: a save over a document that someone else moved
/// to version 3 still wins, as version 4.
#[tokio::test]
async fn save_needs_no_version_and_wins_over_any_stored_one() {
    let shop = Shop::open().await;
    for (version, minutes) in [(0_i64, 10), (1, 20), (2, 30)] {
        let store = shop.store.clone();
        let body = json!({ "enabled": true, "minutes": minutes });
        shop.branch_tx
            .run(&branch(MAIN), move |tx| {
                Box::pin(async move {
                    store
                        .write_in(
                            tx,
                            &branch(MAIN),
                            NAME,
                            version,
                            &json!({ "enabled": true, "minutes": 30 }),
                            &body,
                            "other",
                            at(version),
                        )
                        .await
                })
            })
            .await
            .expect("write")
            .expect("no conflict");
    }
    shop.save(MAIN, rules(false, 5), "owner", at(100)).await;
    let stored = shop
        .store
        .read(&branch(MAIN), NAME)
        .await
        .expect("read")
        .expect("the document");
    assert_eq!(stored.version, 4);
    assert_eq!(shop.rules_of(MAIN).await, Some(rules(false, 5)));
    shop.finish().await;
}

/// `save` runs in the branch's `BranchTx::run` (§5.6.4: every write runs in it): the branch's
/// version moves, another branch's does not.
#[tokio::test]
async fn save_runs_in_the_branch_transaction() {
    let shop = Shop::open().await;
    let main_before = shop
        .branch_tx
        .version(&branch(MAIN))
        .await
        .expect("version");
    let airport_before = shop
        .branch_tx
        .version(&branch(AIRPORT))
        .await
        .expect("version");
    shop.save(MAIN, rules(true, 15), "owner", at(0)).await;
    assert!(
        shop.branch_tx
            .version(&branch(MAIN))
            .await
            .expect("version")
            > main_before,
        "the save took MAIN's branch transaction"
    );
    assert_eq!(
        shop.branch_tx
            .version(&branch(AIRPORT))
            .await
            .expect("version"),
        airport_before
    );
    shop.finish().await;
}

/// Rules saved for one branch stay in it (§5.6.4 *Per branch everywhere*).
#[tokio::test]
async fn rules_saved_for_another_branch_leave_main_unset() {
    let shop = Shop::open().await;
    shop.save(AIRPORT, rules(false, 5), "owner", at(0)).await;
    assert_eq!(shop.rules_of(MAIN).await, None);
    assert_eq!(shop.rules_in_of(MAIN).await, None);
    assert_eq!(shop.rules_of(AIRPORT).await, Some(rules(false, 5)));

    shop.save(MAIN, rules(true, 60), "owner", at(10)).await;
    assert_eq!(shop.rules_of(MAIN).await, Some(rules(true, 60)));
    assert_eq!(shop.rules_of(AIRPORT).await, Some(rules(false, 5)));
    shop.finish().await;
}

/// `rules_in` reads the caller's transaction: a document written in it and not committed is
/// seen there and nowhere else.
#[tokio::test]
async fn rules_in_reads_the_callers_transaction() {
    let shop = Shop::open().await;
    let mut tx = shop.begin().await;
    shop.store
        .write_in(
            &mut tx,
            &branch(MAIN),
            NAME,
            0,
            &json!({ "enabled": true, "minutes": 30 }),
            &json!({ "enabled": false, "minutes": 10 }),
            "owner",
            at(0),
        )
        .await
        .expect("write")
        .expect("no conflict");
    let from_service = shop
        .service
        .rules_in(&mut tx, &branch(MAIN))
        .await
        .expect("rules_in");
    let from_port = shop
        .ports
        .rules_in(&mut tx, &branch(MAIN))
        .await
        .expect("rules_in");
    assert_eq!(from_service, Some(rules(false, 10)));
    assert_eq!(from_port, Some(rules(false, 10)));
    assert_eq!(
        shop.rules_of(MAIN).await,
        None,
        "not committed: the committed read does not see it"
    );
    tx.rollback().await.expect("rollback");
    assert_eq!(shop.rules_of(MAIN).await, None);
    shop.finish().await;
}

// ── PgBookingPorts ───────────────────────────────────────────────────────────

#[tokio::test]
async fn branch_exists_answers_for_known_branches_only() {
    let shop = Shop::open().await;
    for (name, expected) in [(MAIN, true), (AIRPORT, true), ("NOWHERE", false)] {
        assert_eq!(
            shop.ports
                .branch_exists(&branch(name))
                .await
                .expect("branch_exists"),
            expected,
            "{name}"
        );
    }
    shop.finish().await;
}

/// §5.6.8: `member` for a DEMO customer is refused because `live_customer` answers `None` for
/// it; so does a customer of another branch, or an unknown id.
#[tokio::test]
async fn a_demo_customer_is_not_live() {
    let shop = Shop::open().await;
    shop.seed_customer("C-LIVE", MAIN, "สมชาย ใจดี", "LIVE")
        .await;
    shop.seed_customer("C-DEMO", MAIN, "Demo Person", "DEMO")
        .await;
    shop.seed_customer("C-AIR", AIRPORT, "Airport Person", "LIVE")
        .await;

    let live = |id: &'static str, name: &'static str| LiveCustomer {
        id: id.to_string(),
        name: name.to_string(),
    };
    let ask = |branch_id: &'static str, id: &'static str| {
        let ports = &shop.ports;
        async move {
            ports
                .live_customer(&branch(branch_id), id)
                .await
                .expect("live_customer")
        }
    };
    assert_eq!(ask(MAIN, "C-LIVE").await, Some(live("C-LIVE", "สมชาย ใจดี")));
    assert_eq!(ask(MAIN, "C-DEMO").await, None, "a DEMO customer");
    assert_eq!(ask(MAIN, "C-AIR").await, None, "another branch's customer");
    assert_eq!(
        ask(AIRPORT, "C-AIR").await,
        Some(live("C-AIR", "Airport Person"))
    );
    assert_eq!(ask(AIRPORT, "C-LIVE").await, None);
    assert_eq!(ask(MAIN, "C-NOBODY").await, None, "an unknown id");
    shop.finish().await;
}

/// `create` refusal 10 reads this: only an OPEN shift of the branch itself counts.
#[tokio::test]
async fn an_open_shift_counts_only_in_its_own_branch() {
    let shop = Shop::open().await;
    assert!(!shop.open_shift(MAIN).await, "no shift at all");

    shop.seed_shift("S-MAIN-1", MAIN, "CLOSED").await;
    shop.seed_shift("S-AIR-1", AIRPORT, "OPEN").await;
    assert!(
        !shop.open_shift(MAIN).await,
        "a closed shift, another branch's open one"
    );
    assert!(shop.open_shift(AIRPORT).await);

    shop.seed_shift("S-MAIN-2", MAIN, "OPEN").await;
    assert!(shop.open_shift(MAIN).await);
    shop.finish().await;
}

/// `create` refusal 11 and `confirm` rule 6 read this: the branch's current counter rate of the
/// currency — buy for BUY, sell for SELL — and `None` when the branch has none.
#[tokio::test]
async fn the_counter_rate_is_the_branchs_latest_for_the_side() {
    let shop = Shop::open().await;
    shop.seed_rate(MAIN, "USD", "32.1", "32.6", 0).await;
    shop.seed_rate(AIRPORT, "USD", "31.9", "32.9", 1).await;
    shop.seed_rate(AIRPORT, "JPY", "0.2195", "0.2250", 2).await;
    shop.seed_rate(MAIN, "USD", "32.25", "32.75", 3).await;

    assert_eq!(
        shop.counter(MAIN, "USD", Side::Buy).await,
        Some(rate("32.25")),
        "the latest row, its buy"
    );
    assert_eq!(
        shop.counter(MAIN, "USD", Side::Sell).await,
        Some(rate("32.75")),
        "the latest row, its sell"
    );
    assert_eq!(
        shop.counter(AIRPORT, "USD", Side::Buy).await,
        Some(rate("31.9"))
    );
    assert_eq!(
        shop.counter(AIRPORT, "USD", Side::Sell).await,
        Some(rate("32.9"))
    );
    assert_eq!(
        shop.counter(MAIN, "JPY", Side::Sell).await,
        None,
        "another branch's currency"
    );
    assert_eq!(
        shop.counter(MAIN, "EUR", Side::Buy).await,
        None,
        "no rate set"
    );
    shop.finish().await;
}
