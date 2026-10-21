//! MK-7 (#1406, design `docs/design/blueprint/marketing-campaigns.md` D-2, D-15, §4.5, §6.1; §9.1 row MK-7; ticket
//! §4 items 1–3, §7, §9; the lane controller claude/ad05bf8d's rulings 1–4 on #1406): the three marketing views through
//! the real v3 seam.
//!
//! Written before the implementation by the test writer (claude/29df2ec5, the ticket author, rule A); the implementer
//! must not edit, delete or weaken them.
//!
//! - `campaign_performance` (M04): per version, redemptions, PRICE cost, REWARD expense, THB volume, realized margin
//!   and attributed contribution; contribution `awaitsLedger` when a redeeming line has no margin, else `awaitsBroker`
//!   (Broker not connected), never `"0.00"`; REVERSED adds nothing; no field named profit or lift.
//! - `campaigns` (M01): status derived for APPROVED (SCHEDULED / ACTIVE / EXPIRED), stored otherwise; budget used/left;
//!   uses; period totals.
//! - `campaign` (M02): versions, rules, simulations, approval; `campaign: null` for an unknown id or another
//!   organisation's (the `transaction` view's convention).
//! - gates: `campaigns`/`campaign` need `MarketingCampaigns`, `campaign_performance` needs `MarketingPerformance`; a
//!   refusal is 400 `roleNotAllowedHere` (the `Rpc` path's `RoleNotAllowed` sentence, `policy_error.rs:110`).
//!
//! Every request names the test's own branch (`?branch=`) and the period today−3 ..= today+1; redemptions in the
//! period are one day old, outside it twenty days old. Needs `TEST_DATABASE_URL` naming an existing database whose name
//! ends in `_test`; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const USER: &str = "00000000-0000-0000-0000-000000000002";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("valid decimal")
}

// ------------------------------------------------------------ the room

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
    branch: String,
    customer: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("mk7_views");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
             VALUES ($1::uuid, $2::uuid, 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(USER)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("seed test user");

        let branch = unique("B").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch");
        let customer = unique("CX");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $1, 'Somchai', 'LIVE', now())",
        )
        .bind(&customer)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("a customer");

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
            branch,
            customer,
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }

    /// `/api/v3/views/<view>?branch=<the room's>&from=<today−3>&to=<today+1>` plus `extra`.
    fn uri(&self, view: &str, extra: &str) -> String {
        let today = Utc::now().date_naive();
        let from = today - chrono::Duration::days(3);
        let to = today + chrono::Duration::days(1);
        format!(
            "/api/v3/views/{view}?branch={}&from={from}&to={to}{extra}",
            self.branch
        )
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn add_user(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(id)
        .bind(ORG)
        .bind(unique("user"))
        .execute(&self.pool)
        .await
        .expect("a user");
        id
    }

    /// A campaign of `org` with one version `status`, from `starts` to `ends` (SQL interval offsets from now,
    /// minute-truncated), its budget row, and `current_version_id` pointing at it. Returns (campaign, version).
    async fn campaign_in(&self, org: &str, status: &str, starts: &str, ends: &str) -> (Uuid, Uuid) {
        let campaign = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, $4::uuid, now())",
        )
        .bind(campaign)
        .bind(org)
        .bind(unique("campaign"))
        .bind(USER)
        .execute(&self.pool)
        .await
        .expect("a campaign");
        let version = self
            .version_of(campaign, org, 1, status, starts, ends)
            .await;
        sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
            .bind(version)
            .bind(campaign)
            .execute(&self.pool)
            .await
            .expect("the current version");
        (campaign, version)
    }

    async fn campaign(&self, status: &str, starts: &str, ends: &str) -> (Uuid, Uuid) {
        self.campaign_in(ORG, status, starts, ends).await
    }

    /// Version `no` of `campaign`, budget 1000.00, max uses 10, with its budget row.
    async fn version_of(
        &self,
        campaign: Uuid,
        org: &str,
        no: i32,
        status: &str,
        starts: &str,
        ends: &str,
    ) -> Uuid {
        let version = Uuid::new_v4();
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, max_uses, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, date_trunc('minute', now()) + interval '{starts}', \
             date_trunc('minute', now()) + interval '{ends}', 1000.00, 10, 'g', 1, \
             CASE WHEN $5 IN ('DRAFT','SIMULATED') THEN NULL ELSE 'hash' END, now(), $6::uuid, now())"
        )))
        .bind(version)
        .bind(org)
        .bind(campaign)
        .bind(no)
        .bind(status)
        .bind(USER)
        .execute(&self.pool)
        .await
        .expect("a version");
        sqlx::query("INSERT INTO campaign_budgets (campaign_version_id) VALUES ($1)")
            .bind(version)
            .execute(&self.pool)
            .await
            .expect("the version's budget row");
        version
    }

    async fn rule(&self, version: Uuid, class: &str, no: i32) -> Uuid {
        let rule = Uuid::new_v4();
        let kind = if class == "PRICE" {
            "FEE_WAIVER"
        } else {
            "REWARD_THB"
        };
        sqlx::query(
            "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
             VALUES ($1, $2, $3, $4, '{\"channels\":[\"DESK\"]}'::jsonb, '{}'::jsonb, '{}'::jsonb, false, $5)",
        )
        .bind(rule)
        .bind(version)
        .bind(kind)
        .bind(class)
        .bind(no)
        .execute(&self.pool)
        .await
        .expect("a rule");
        rule
    }

    /// A COMPLETED LIVE exchange in the room's branch with lines `(thb_amount, realized_margin_thb)`.
    async fn exchange(&self, lines: &[(&str, Option<&str>)]) -> Uuid {
        self.exchange_in(&self.branch, lines).await
    }

    /// Another branch of the room's organisation.
    async fn other_branch(&self) -> String {
        let branch = unique("B").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("another branch");
        branch
    }

    /// Another organisation; returns its id.
    async fn other_org(&self) -> Uuid {
        let org = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{org}', '{}', NULL, 'Asia/Bangkok', now())",
            unique("ORG")
        ))
        .await;
        org
    }

    /// A COMPLETED LIVE exchange in `branch` with lines `(thb_amount, realized_margin_thb)`.
    async fn exchange_in(&self, branch: &str, lines: &[(&str, Option<&str>)]) -> Uuid {
        let teller = self.add_user().await;
        let shift = unique("shift");
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("close the branch's open shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
        )
        .bind(&shift)
        .bind(branch)
        .bind(unique("open-req"))
        .bind(teller)
        .execute(&self.pool)
        .await
        .expect("a shift");
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
             recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2::uuid, $3, $4, 'LIVE', $6, $5, 'hash', $7, 'Somchai', 'THA', 'system', 'WALK-IN', \
             '', current_date, now(), now(), 'system', 'COMPLETED', now())",
        )
        .bind(id)
        .bind(ORG)
        .bind(unique("DOC"))
        .bind(branch)
        .bind(unique("sub"))
        .bind(&shift)
        .bind(&self.customer)
        .execute(&self.pool)
        .await
        .expect("a completed transaction");
        for (no, (thb, margin)) in lines.iter().enumerate() {
            sqlx::query(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
                 unit_size, rate_snapshot, thb_amount, settle_thb, realized_margin_thb, line_no, created_at, \
                 updated_at) \
                 VALUES ($1, $2::uuid, $3, 'SELL', 'USD', 100, 1, 32, $4, $4, $5, $6, now(), now())",
            )
            .bind(Uuid::new_v4())
            .bind(ORG)
            .bind(id)
            .bind(d(thb))
            .bind(margin.map(d))
            .bind(no as i32 + 1)
            .execute(&self.pool)
            .await
            .expect("a transaction line");
        }
        id
    }

    /// A quote of the room's branch, expiring `expires` (an SQL interval offset from now).
    async fn quote(&self, expires: &str) -> Uuid {
        let request = Uuid::new_v4();
        let publication = Uuid::new_v4();
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4::text, 'test', '1', '{}'::jsonb, 'test', now(), \
             'APPROVED', $5)",
        )
        .bind(request)
        .bind(ORG)
        .bind(&self.branch)
        .bind(publication.to_string())
        .bind(unique("pub"))
        .execute(&self.pool)
        .await
        .expect("an approved publication request");
        sqlx::query(
            "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, published_by) \
             SELECT $1, $2::uuid, $3, COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = $3), 0) + 1, \
             now(), $4, 'test'",
        )
        .bind(publication)
        .bind(ORG)
        .bind(&self.branch)
        .bind(request)
        .execute(&self.pool)
        .await
        .expect("a publication");
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, NULL, 'INDICATIVE', 'DESK', 'ACTIVE', $4, now() + interval '{expires}', \
             'test', now(), now())"
        )))
        .bind(id)
        .bind(ORG)
        .bind(&self.branch)
        .bind(publication)
        .execute(&self.pool)
        .await
        .expect("a quote");
        id
    }

    /// A reservation of `rule` for `amount` in `status`, expiring `expires` (an SQL interval offset from now).
    async fn reservation(
        &self,
        version: Uuid,
        rule: Uuid,
        amount: &str,
        status: &str,
        expires: &str,
    ) -> Uuid {
        let quote = self.quote(expires).await;
        let id = Uuid::new_v4();
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO benefit_reservations (id, organization_id, rule_id, campaign_version_id, quote_id, \
             amount_thb, status, expires_at, created_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, now() + interval '{expires}', now())"
        )))
        .bind(id)
        .bind(ORG)
        .bind(rule)
        .bind(version)
        .bind(quote)
        .bind(d(amount))
        .bind(status)
        .execute(&self.pool)
        .await
        .expect("a reservation");
        id
    }

    /// A redemption of `rule` (its version and class) by `transaction`, `days_ago` days old; `reversed` = REVERSED
    /// with a reversal row.
    async fn redeem(
        &self,
        rule: Uuid,
        transaction: Uuid,
        amount: &str,
        days_ago: i32,
        reversed: bool,
    ) {
        let (version, class): (Uuid, String) = sqlx::query_as(
            "SELECT campaign_version_id, benefit_class FROM benefit_rules WHERE id = $1",
        )
        .bind(rule)
        .fetch_one(&self.pool)
        .await
        .expect("the rule's version and class");
        let reservation = self
            .reservation(version, rule, amount, "CONSUMED", "15 minutes")
            .await;
        let reversal = if reversed {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, reason, \
                 amount_thb, business_date, requested_by, requested_at) \
                 VALUES ($1, $2::uuid, $3, $4, 'REQUESTED', 'test', 3200, current_date, 'test', now())",
            )
            .bind(id)
            .bind(ORG)
            .bind(&self.branch)
            .bind(transaction)
            .execute(&self.pool)
            .await
            .expect("a reversal");
            Some(id)
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO benefit_redemptions (id, organization_id, rule_id, campaign_version_id, reservation_id, \
             transaction_id, benefit_class, amount_thb, status, reversal_id, redeemed_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9, $10, \
             date_trunc('minute', now()) - make_interval(days => $11))",
        )
        .bind(Uuid::new_v4())
        .bind(ORG)
        .bind(rule)
        .bind(version)
        .bind(reservation)
        .bind(transaction)
        .bind(class)
        .bind(d(amount))
        .bind(if reversed { "REVERSED" } else { "REDEEMED" })
        .bind(reversal)
        .bind(days_ago)
        .execute(&self.pool)
        .await
        .expect("a redemption");
    }
}

// ------------------------------------------------------------ callers

fn user_of(spec: &str) -> Option<Actor> {
    let role = match spec {
        "EXECUTIVE" => Role::Executive,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        _ => return None,
    };
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    Some(Actor::new(
        &format!("user-{}", role.as_str()),
        ORG,
        vec![ScopedRole::new(role, None, from, None)],
        None,
    ))
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user.clone();
        Box::pin(async move { Callers { user, staff_user } })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }

    fn ok_data(&self) -> &Value {
        assert_eq!(self.status, StatusCode::OK, "{}", self.text);
        let data = &self.body["data"];
        assert!(data.is_object(), "an answer with data: {}", self.text);
        data
    }
}

async fn get(room: &Room, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn assert_refused(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(a.code(), "roleNotAllowedHere", "{what}: {}", a.text);
}

fn row_by<'a>(rows: &'a Value, key: &str, id: Uuid) -> &'a Value {
    rows.as_array()
        .unwrap_or_else(|| panic!("an array: {rows}"))
        .iter()
        .find(|r| r[key] == id.to_string())
        .unwrap_or_else(|| panic!("a row with {key} = {id}: {rows}"))
}

/// Every object key in `v`, at any depth.
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                out.push(k.clone());
                keys(x, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|x| keys(x, out)),
        _ => {}
    }
}

// ------------------------------------------------------------ campaign_performance (M04, D-15)

#[actix_web::test]
async fn campaign_performance_shows_each_versions_figures_and_contribution_awaits_broker() {
    let room = Room::start().await;
    let (campaign, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let price = room.rule(v, "PRICE", 1).await;
    let reward = room.rule(v, "REWARD", 2).await;
    let x1 = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x2 = room
        .exchange(&[("1000.00", Some("15.50")), ("500.00", Some("4.50"))])
        .await;
    room.redeem(price, x1, "10.00", 1, false).await;
    room.redeem(reward, x1, "5.00", 1, false).await;
    room.redeem(price, x2, "3.00", 1, false).await;

    let a = get(&room, "MARKETING", &room.uri("campaign_performance", "")).await;
    let row = row_by(&a.ok_data()["versions"], "versionId", v);
    assert_eq!(row["campaignId"], campaign.to_string(), "{row}");
    assert_eq!(row["version"], 1, "{row}");
    assert_eq!(row["redemptions"], 3, "{row}");
    assert_eq!(row["priceCostThb"], "13.00", "{row}");
    assert_eq!(row["rewardExpenseThb"], "5.00", "{row}");
    assert_eq!(
        row["volumeThb"], "4700.00",
        "each redeeming exchange once: {row}"
    );
    assert_eq!(row["realizedMarginThb"]["value"], "60.00", "{row}");
    assert!(row["realizedMarginThb"]["unavailable"].is_null(), "{row}");
    assert!(
        row["attributedContributionThb"]["value"].is_null(),
        "contribution is never a number while commission is unavailable, and never \"0.00\": {row}"
    );
    assert_eq!(
        row["attributedContributionThb"]["unavailable"], "awaitsBroker",
        "{row}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_contribution_awaits_the_ledger_when_a_line_has_no_margin() {
    let room = Room::start().await;
    let (_, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let price = room.rule(v, "PRICE", 1).await;
    let reward = room.rule(v, "REWARD", 2).await;
    let x1 = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x2 = room
        .exchange(&[("1000.00", Some("10.00")), ("400.00", None)])
        .await;
    room.redeem(reward, x1, "5.00", 1, false).await;
    room.redeem(price, x2, "2.00", 1, false).await;

    let a = get(&room, "MARKETING", &room.uri("campaign_performance", "")).await;
    let row = row_by(&a.ok_data()["versions"], "versionId", v);
    assert!(row["realizedMarginThb"]["value"].is_null(), "{row}");
    assert_eq!(
        row["realizedMarginThb"]["unavailable"], "awaitsLedger",
        "{row}"
    );
    assert!(
        row["attributedContributionThb"]["value"].is_null(),
        "never \"0.00\": {row}"
    );
    assert_eq!(
        row["attributedContributionThb"]["unavailable"], "awaitsLedger",
        "the ledger reason comes before the Broker one (ruling 4): {row}"
    );
    assert_eq!(
        row["redemptions"], 2,
        "the other figures still render: {row}"
    );
    assert_eq!(row["priceCostThb"], "2.00", "{row}");
    assert_eq!(row["rewardExpenseThb"], "5.00", "{row}");
    assert_eq!(row["volumeThb"], "4600.00", "{row}");
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_a_reversed_redemption_adds_nothing() {
    let room = Room::start().await;
    let (_, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let price = room.rule(v, "PRICE", 1).await;
    let reward = room.rule(v, "REWARD", 2).await;
    let x1 = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x2 = room.exchange(&[("900.00", None)]).await;
    room.redeem(price, x1, "10.00", 1, false).await;
    room.redeem(reward, x2, "6.00", 1, true).await;

    let a = get(&room, "MARKETING", &room.uri("campaign_performance", "")).await;
    let row = row_by(&a.ok_data()["versions"], "versionId", v);
    assert_eq!(row["redemptions"], 1, "{row}");
    assert_eq!(row["priceCostThb"], "10.00", "{row}");
    assert_eq!(row["rewardExpenseThb"], "0.00", "{row}");
    assert_eq!(row["volumeThb"], "3200.00", "{row}");
    assert_eq!(
        row["realizedMarginThb"]["value"], "40.00",
        "the reversed exchange's missing margin does not count either: {row}"
    );
    assert_eq!(
        row["attributedContributionThb"]["unavailable"], "awaitsBroker",
        "{row}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_has_no_field_named_profit_or_lift() {
    let room = Room::start().await;
    let (_, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let reward = room.rule(v, "REWARD", 1).await;
    let x = room.exchange(&[("3200.00", Some("40.00"))]).await;
    room.redeem(reward, x, "5.00", 1, false).await;

    let a = get(&room, "EXECUTIVE", &room.uri("campaign_performance", "")).await;
    let data = a.ok_data();
    row_by(&data["versions"], "versionId", v);
    let mut all = Vec::new();
    keys(&a.body, &mut all);
    let named: Vec<&String> = all
        .iter()
        .filter(|k| {
            let k = k.to_lowercase();
            k.contains("profit") || k.contains("lift")
        })
        .collect();
    assert!(
        named.is_empty(),
        "attributed, never profit or lift (D-15, bp:622-623): {named:?} in {}",
        a.text
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_is_gated_by_marketing_performance() {
    let room = Room::start().await;
    let uri = room.uri("campaign_performance", "");
    let marketing = get(&room, "MARKETING", &uri).await;
    assert!(
        marketing.ok_data()["versions"].is_array(),
        "MARKETING holds MarketingPerformance: {}",
        marketing.text
    );
    let executive = get(&room, "EXECUTIVE", &uri).await;
    assert!(
        executive.ok_data()["versions"].is_array(),
        "EXECUTIVE holds MarketingPerformance: {}",
        executive.text
    );
    assert_refused(&get(&room, "TELLER", &uri).await, "TELLER");
    room.finish().await;
}

// ------------------------------------------------------------ campaigns (M01)

#[actix_web::test]
async fn campaigns_shows_scheduled_for_an_approved_version_before_it_starts() {
    let room = Room::start().await;
    let (scheduled, _) = room.campaign("APPROVED", "2 days", "30 days").await;

    let a = get(&room, "MARKETING", &room.uri("campaigns", "")).await;
    let row = row_by(&a.ok_data()["campaigns"], "id", scheduled);
    assert_eq!(
        row["status"], "SCHEDULED",
        "derived from the clock, never stored (D-2, §9 edge 4): {row}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_shows_active_and_expired_by_the_clock_and_a_stored_status_otherwise() {
    let room = Room::start().await;
    let (active, _) = room.campaign("APPROVED", "-2 days", "30 days").await;
    let (expired, _) = room.campaign("APPROVED", "-30 days", "-2 days").await;
    let (paused, _) = room.campaign("PAUSED", "-2 days", "30 days").await;
    let (pending, _) = room.campaign("PENDING_APPROVAL", "2 days", "30 days").await;

    let a = get(&room, "MARKETING", &room.uri("campaigns", "")).await;
    let rows = &a.ok_data()["campaigns"];
    assert_eq!(row_by(rows, "id", active)["status"], "ACTIVE", "{rows}");
    assert_eq!(row_by(rows, "id", expired)["status"], "EXPIRED", "{rows}");
    assert_eq!(row_by(rows, "id", paused)["status"], "PAUSED", "{rows}");
    assert_eq!(
        row_by(rows, "id", pending)["status"],
        "PENDING_APPROVAL",
        "only an APPROVED version is derived: {rows}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_shows_budget_used_and_left_uses_and_period_totals() {
    let room = Room::start().await;
    let (campaign, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let price = room.rule(v, "PRICE", 1).await;
    let reward = room.rule(v, "REWARD", 2).await;
    room.exec(&format!(
        "UPDATE campaign_budgets SET consumed_thb = 120.00, used_count = 3 WHERE campaign_version_id = '{v}'"
    ))
    .await;
    // an active reservation holds budget; an expired one does not (design §4.1 "available")
    room.reservation(v, price, "50.00", "RESERVED", "15 minutes")
        .await;
    room.reservation(v, price, "70.00", "RESERVED", "-15 minutes")
        .await;
    let x1 = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x2 = room.exchange(&[("1000.00", Some("10.00"))]).await;
    let x3 = room.exchange(&[("800.00", Some("8.00"))]).await;
    room.redeem(price, x1, "10.00", 1, false).await;
    room.redeem(reward, x1, "4.00", 1, false).await;
    room.redeem(reward, x2, "9.00", 1, true).await;
    room.redeem(price, x3, "100.00", 20, false).await;

    let a = get(&room, "MARKETING", &room.uri("campaigns", "")).await;
    let row = row_by(&a.ok_data()["campaigns"], "id", campaign);
    assert_eq!(row["versionId"], v.to_string(), "{row}");
    assert_eq!(row["budgetThb"], "1000.00", "{row}");
    assert_eq!(row["budgetUsedThb"], "120.00", "consumed_thb: {row}");
    assert_eq!(
        row["budgetLeftThb"], "830.00",
        "1000 − 120 consumed − 50 actively reserved: {row}"
    );
    assert_eq!(row["uses"], 3, "used_count: {row}");
    assert_eq!(row["maxUses"], 10, "{row}");
    assert_eq!(
        row["period"]["redemptions"], 2,
        "REDEEMED in the period only: {row}"
    );
    assert_eq!(row["period"]["priceCostThb"], "10.00", "{row}");
    assert_eq!(
        row["period"]["rewardExpenseThb"], "4.00",
        "the REVERSED 9.00 adds nothing: {row}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_is_gated_by_marketing_campaigns() {
    let room = Room::start().await;
    let uri = room.uri("campaigns", "");
    let marketing = get(&room, "MARKETING", &uri).await;
    assert!(
        marketing.ok_data()["campaigns"].is_array(),
        "MARKETING holds MarketingCampaigns: {}",
        marketing.text
    );
    // Under RBAC §11.2, Executive holds MarketingCampaigns (Customers: Full).
    let exec = get(&room, "EXECUTIVE", &uri).await;
    assert!(
        exec.ok_data()["campaigns"].is_array(),
        "EXECUTIVE holds MarketingCampaigns under §11.2: {}",
        exec.text
    );
    assert_refused(&get(&room, "TELLER", &uri).await, "TELLER");
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_lists_only_the_callers_organisations_campaigns() {
    let room = Room::start().await;
    let (ours, _) = room.campaign("APPROVED", "-2 days", "30 days").await;
    let other_org = room.other_org().await;
    let (theirs, _) = room
        .campaign_in(&other_org.to_string(), "APPROVED", "-2 days", "30 days")
        .await;

    let a = get(&room, "MARKETING", &room.uri("campaigns", "")).await;
    let rows = &a.ok_data()["campaigns"];
    row_by(rows, "id", ours);
    assert!(
        !rows
            .as_array()
            .expect("an array")
            .iter()
            .any(|r| r["id"] == theirs.to_string()),
        "another organisation's campaign is never listed: {rows}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_period_totals_count_only_the_resolved_branch() {
    let room = Room::start().await;
    let (campaign, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let price = room.rule(v, "PRICE", 1).await;
    let reward = room.rule(v, "REWARD", 2).await;
    let elsewhere = room.other_branch().await;
    let x_here = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x_there = room
        .exchange_in(&elsewhere, &[("1000.00", Some("10.00"))])
        .await;
    room.redeem(price, x_here, "10.00", 1, false).await;
    room.redeem(price, x_there, "70.00", 1, false).await;
    room.redeem(reward, x_there, "30.00", 1, false).await;

    let a = get(&room, "MARKETING", &room.uri("campaigns", "")).await;
    let row = row_by(&a.ok_data()["campaigns"], "id", campaign);
    assert_eq!(
        row["period"]["redemptions"], 1,
        "a redemption on another branch of the organisation is not this branch's: {row}"
    );
    assert_eq!(row["period"]["priceCostThb"], "10.00", "{row}");
    assert_eq!(row["period"]["rewardExpenseThb"], "0.00", "{row}");
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_counts_only_the_resolved_branch() {
    let room = Room::start().await;
    let (_, v) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let price = room.rule(v, "PRICE", 1).await;
    let reward = room.rule(v, "REWARD", 2).await;
    let elsewhere = room.other_branch().await;
    let x_here = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x_there = room
        .exchange_in(&elsewhere, &[("1000.00", Some("10.00"))])
        .await;
    room.redeem(price, x_here, "10.00", 1, false).await;
    room.redeem(price, x_there, "70.00", 1, false).await;
    room.redeem(reward, x_there, "30.00", 1, false).await;

    let a = get(&room, "MARKETING", &room.uri("campaign_performance", "")).await;
    let row = row_by(&a.ok_data()["versions"], "versionId", v);
    assert_eq!(
        row["redemptions"], 1,
        "the other branch's redemptions are not in the resolved branch's M04: {row}"
    );
    assert_eq!(row["priceCostThb"], "10.00", "{row}");
    assert_eq!(row["rewardExpenseThb"], "0.00", "{row}");
    assert_eq!(row["volumeThb"], "3200.00", "{row}");
    assert_eq!(row["realizedMarginThb"]["value"], "40.00", "{row}");
    room.finish().await;
}

// ------------------------------------------------------------ campaign (M02)

#[actix_web::test]
async fn campaign_shows_versions_rules_simulations_and_approval() {
    let room = Room::start().await;
    let (campaign, v1) = room.campaign("APPROVED", "-2 days", "30 days").await;
    let v2 = room
        .version_of(campaign, ORG, 2, "DRAFT", "1 day", "40 days")
        .await;
    room.exec(&format!(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ('{v1}', '{}')",
        room.branch
    ))
    .await;
    let r2 = room.rule(v1, "REWARD", 2).await;
    let r1 = room.rule(v1, "PRICE", 1).await;
    let approval = Uuid::new_v4();
    room.exec(&format!(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
         policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ('{approval}', '{ORG}', 'CAMPAIGN_VERSION', NULL, '{v1}', 'hash', '1', '{{}}'::jsonb, 'test', now(), \
         'APPROVED', '{}')",
        unique("campaign-approval")
    ))
    .await;
    room.exec(&format!(
        "UPDATE campaign_versions SET approval_id = '{approval}' WHERE id = '{v1}'"
    ))
    .await;
    let simulation = Uuid::new_v4();
    room.exec(&format!(
        "INSERT INTO campaign_simulations (id, campaign_version_id, input, result, cost_source, simulated_by, \
         simulated_at) VALUES ('{simulation}', '{v1}', '{{\"scenarios\":[]}}'::jsonb, '{{\"lines\":[]}}'::jsonb, \
         'UNAVAILABLE', '{USER}', date_trunc('minute', now()))"
    ))
    .await;

    let a = get(
        &room,
        "MARKETING",
        &room.uri("campaign", &format!("&id={campaign}")),
    )
    .await;
    let c = &a.ok_data()["campaign"];
    assert_eq!(c["id"], campaign.to_string(), "{c}");
    assert_eq!(c["currentVersionId"], v1.to_string(), "{c}");
    let versions = c["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 2, "{c}");
    assert_eq!(versions[0]["id"], v2.to_string(), "newest first: {c}");
    assert_eq!(versions[0]["status"], "DRAFT", "{c}");
    assert!(versions[0]["approval"].is_null(), "{c}");
    let one = &versions[1];
    assert_eq!(one["id"], v1.to_string(), "{c}");
    assert_eq!(one["version"], 1, "{c}");
    assert_eq!(one["status"], "ACTIVE", "derived for APPROVED: {c}");
    assert_eq!(one["budgetThb"], "1000.00", "{c}");
    assert_eq!(one["branches"], serde_json::json!([room.branch]), "{c}");
    let rules = one["rules"].as_array().expect("rules");
    assert_eq!(rules.len(), 2, "{c}");
    assert_eq!(rules[0]["id"], r1.to_string(), "by rule_no: {c}");
    assert_eq!(rules[0]["benefitClass"], "PRICE", "{c}");
    assert_eq!(rules[0]["benefitType"], "FEE_WAIVER", "{c}");
    assert_eq!(rules[1]["id"], r2.to_string(), "{c}");
    assert_eq!(rules[1]["benefitClass"], "REWARD", "{c}");
    assert_eq!(
        rules[1]["eligibility"],
        serde_json::json!({"channels": ["DESK"]}),
        "{c}"
    );
    let sims = one["simulations"].as_array().expect("simulations");
    assert_eq!(sims.len(), 1, "{c}");
    assert_eq!(sims[0]["id"], simulation.to_string(), "{c}");
    assert_eq!(sims[0]["costSource"], "UNAVAILABLE", "{c}");
    assert_eq!(one["approval"]["id"], approval.to_string(), "{c}");
    assert_eq!(one["approval"]["status"], "APPROVED", "{c}");
    room.finish().await;
}

#[actix_web::test]
async fn campaign_of_an_unknown_id_or_another_organisation_is_null() {
    let room = Room::start().await;
    let other_org = Uuid::new_v4();
    room.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{other_org}', '{}', NULL, 'Asia/Bangkok', now())",
        unique("ORG")
    ))
    .await;
    let (theirs, _) = room
        .campaign_in(&other_org.to_string(), "APPROVED", "-2 days", "30 days")
        .await;

    for (what, id) in [
        ("unknown", Uuid::new_v4()),
        ("another organisation's", theirs),
    ] {
        let a = get(
            &room,
            "MARKETING",
            &room.uri("campaign", &format!("&id={id}")),
        )
        .await;
        let data = a.ok_data();
        assert!(
            data["campaign"].is_null(),
            "{what} id answers campaign: null, as the transaction view does: {}",
            a.text
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn campaign_is_gated_by_marketing_campaigns() {
    let room = Room::start().await;
    let (campaign, _) = room.campaign("APPROVED", "-2 days", "30 days").await;
    let uri = room.uri("campaign", &format!("&id={campaign}"));
    let marketing = get(&room, "MARKETING", &uri).await;
    assert_eq!(
        marketing.ok_data()["campaign"]["id"],
        campaign.to_string(),
        "MARKETING holds MarketingCampaigns: {}",
        marketing.text
    );
    // Under RBAC §11.2, Executive holds MarketingCampaigns (Customers: Full).
    let exec = get(&room, "EXECUTIVE", &uri).await;
    assert_eq!(
        exec.ok_data()["campaign"]["id"],
        campaign.to_string(),
        "EXECUTIVE holds MarketingCampaigns under §11.2: {}",
        exec.text
    );
    assert_refused(&get(&room, "TELLER", &uri).await, "TELLER");
    room.finish().await;
}

#[actix_web::test]
async fn campaign_versions_each_keep_their_own_children() {
    let room = Room::start().await;
    let other_branch = room.other_branch().await;
    let (campaign, v1) = room.campaign("APPROVED", "-2 days", "30 days").await;
    let v2 = room
        .version_of(campaign, ORG, 2, "PENDING_APPROVAL", "1 day", "40 days")
        .await;

    room.exec(&format!(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES \
         ('{v1}', '{}'), ('{v2}', '{other_branch}')",
        room.branch
    ))
    .await;

    let r_v1 = room.rule(v1, "PRICE", 1).await;
    let r_v2 = room.rule(v2, "REWARD", 1).await;

    let sim_v1 = Uuid::new_v4();
    let sim_v2 = Uuid::new_v4();
    room.exec(&format!(
        "INSERT INTO campaign_simulations (id, campaign_version_id, input, result, cost_source, simulated_by, \
         simulated_at) VALUES \
         ('{sim_v1}', '{v1}', '{{\"scenarios\":[]}}'::jsonb, '{{\"lines\":[]}}'::jsonb, 'UNAVAILABLE', '{USER}', date_trunc('minute', now())), \
         ('{sim_v2}', '{v2}', '{{\"scenarios\":[]}}'::jsonb, '{{\"lines\":[]}}'::jsonb, 'UNAVAILABLE', '{USER}', date_trunc('minute', now()))"
    ))
    .await;

    let app_v1 = Uuid::new_v4();
    let app_v2 = Uuid::new_v4();
    room.exec(&format!(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
         policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) VALUES \
         ('{app_v1}', '{ORG}', 'CAMPAIGN_VERSION', NULL, '{v1}', 'hash', '1', '{{}}'::jsonb, 'test', now(), 'APPROVED', '{}'), \
         ('{app_v2}', '{ORG}', 'CAMPAIGN_VERSION', NULL, '{v2}', 'hash', '1', '{{}}'::jsonb, 'test', now(), 'PENDING', '{}')",
        unique("campaign-approval-v1"),
        unique("campaign-approval-v2")
    ))
    .await;
    room.exec(&format!(
        "UPDATE campaign_versions SET approval_id = '{app_v1}' WHERE id = '{v1}'"
    ))
    .await;
    room.exec(&format!(
        "UPDATE campaign_versions SET approval_id = '{app_v2}' WHERE id = '{v2}'"
    ))
    .await;

    let a = get(
        &room,
        "MARKETING",
        &room.uri("campaign", &format!("&id={campaign}")),
    )
    .await;
    let versions = a.ok_data()["campaign"]["versions"]
        .as_array()
        .expect("versions")
        .clone();
    assert_eq!(versions.len(), 2);

    let two = &versions[0];
    assert_eq!(two["id"], v2.to_string());
    assert_eq!(two["branches"], serde_json::json!([other_branch]));
    let two_rules = two["rules"].as_array().expect("v2 rules");
    assert_eq!(two_rules.len(), 1, "v2 must have only its own rule: {two}");
    assert_eq!(two_rules[0]["id"], r_v2.to_string());
    let two_sims = two["simulations"].as_array().expect("v2 simulations");
    assert_eq!(
        two_sims.len(),
        1,
        "v2 must have only its own simulation: {two}"
    );
    assert_eq!(two_sims[0]["id"], sim_v2.to_string());
    assert_eq!(two["approval"]["id"], app_v2.to_string());
    assert_eq!(two["approval"]["status"], "PENDING");

    let one = &versions[1];
    assert_eq!(one["id"], v1.to_string());
    assert_eq!(one["branches"], serde_json::json!([room.branch]));
    let one_rules = one["rules"].as_array().expect("v1 rules");
    assert_eq!(one_rules.len(), 1, "v1 must have only its own rule: {one}");
    assert_eq!(one_rules[0]["id"], r_v1.to_string());
    let one_sims = one["simulations"].as_array().expect("v1 simulations");
    assert_eq!(
        one_sims.len(),
        1,
        "v1 must have only its own simulation: {one}"
    );
    assert_eq!(one_sims[0]["id"], sim_v1.to_string());
    assert_eq!(one["approval"]["id"], app_v1.to_string());
    assert_eq!(one["approval"]["status"], "APPROVED");

    room.finish().await;
}

// ------------------------------------------------------------ period check (#2200, §9)

fn assert_invalid_field(a: &Answer, field: &str, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(a.code(), "invalidRequest", "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["field"].as_str(),
        Some(field),
        "{what}: {}",
        a.text
    );
}

fn assert_period_order(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(a.code(), "periodOrder", "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["field"].as_str(),
        Some("to"),
        "{what}: {}",
        a.text
    );
}

#[actix_web::test]
async fn campaigns_period_check_accepts_valid_range_and_single_day() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaigns?branch={}", room.branch);
    let ok_range = get(
        &room,
        "MARKETING",
        &format!("{base}&from=2026-01-01&to=2026-01-31"),
    )
    .await;
    assert!(ok_range.ok_data()["campaigns"].is_array());
    let ok_one_day = get(
        &room,
        "MARKETING",
        &format!("{base}&from=2026-01-31&to=2026-01-31"),
    )
    .await;
    assert!(ok_one_day.ok_data()["campaigns"].is_array());
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_refuses_missing_or_empty_from_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaigns?branch={}", room.branch);
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&to=2026-01-31")).await,
        "from",
        "campaigns missing from",
    );
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&from=&to=2026-01-31")).await,
        "from",
        "campaigns empty from",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_refuses_missing_or_empty_to_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaigns?branch={}", room.branch);
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&from=2026-01-01")).await,
        "to",
        "campaigns missing to",
    );
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&from=2026-01-01&to=")).await,
        "to",
        "campaigns empty to",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_refuses_invalid_from_date_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaigns?branch={}", room.branch);
    for bad in ["2026-13-01", "x", "2026-02-30"] {
        assert_invalid_field(
            &get(
                &room,
                "MARKETING",
                &format!("{base}&from={bad}&to=2026-01-31"),
            )
            .await,
            "from",
            &format!("campaigns from={bad}"),
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_refuses_invalid_to_date_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaigns?branch={}", room.branch);
    assert_invalid_field(
        &get(
            &room,
            "MARKETING",
            &format!("{base}&from=2026-01-01&to=2026-02-30"),
        )
        .await,
        "to",
        "campaigns to=2026-02-30",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaigns_refuses_reversed_period_with_400_period_order() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaigns?branch={}", room.branch);
    assert_period_order(
        &get(
            &room,
            "MARKETING",
            &format!("{base}&from=2026-02-01&to=2026-01-01"),
        )
        .await,
        "campaigns reversed period",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_period_check_accepts_valid_range_and_single_day() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaign_performance?branch={}", room.branch);
    let ok_range = get(
        &room,
        "MARKETING",
        &format!("{base}&from=2026-01-01&to=2026-01-31"),
    )
    .await;
    assert!(ok_range.ok_data()["versions"].is_array());
    let ok_one_day = get(
        &room,
        "MARKETING",
        &format!("{base}&from=2026-01-31&to=2026-01-31"),
    )
    .await;
    assert!(ok_one_day.ok_data()["versions"].is_array());
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_refuses_missing_or_empty_from_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaign_performance?branch={}", room.branch);
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&to=2026-01-31")).await,
        "from",
        "campaign_performance missing from",
    );
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&from=&to=2026-01-31")).await,
        "from",
        "campaign_performance empty from",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_refuses_missing_or_empty_to_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaign_performance?branch={}", room.branch);
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&from=2026-01-01")).await,
        "to",
        "campaign_performance missing to",
    );
    assert_invalid_field(
        &get(&room, "MARKETING", &format!("{base}&from=2026-01-01&to=")).await,
        "to",
        "campaign_performance empty to",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_refuses_invalid_from_date_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaign_performance?branch={}", room.branch);
    for bad in ["2026-13-01", "x", "2026-02-30"] {
        assert_invalid_field(
            &get(
                &room,
                "MARKETING",
                &format!("{base}&from={bad}&to=2026-01-31"),
            )
            .await,
            "from",
            &format!("campaign_performance from={bad}"),
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_refuses_invalid_to_date_with_400_invalid_request() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaign_performance?branch={}", room.branch);
    assert_invalid_field(
        &get(
            &room,
            "MARKETING",
            &format!("{base}&from=2026-01-01&to=2026-02-30"),
        )
        .await,
        "to",
        "campaign_performance to=2026-02-30",
    );
    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_refuses_reversed_period_with_400_period_order() {
    let room = Room::start().await;
    let base = format!("/api/v3/views/campaign_performance?branch={}", room.branch);
    assert_period_order(
        &get(
            &room,
            "MARKETING",
            &format!("{base}&from=2026-02-01&to=2026-01-01"),
        )
        .await,
        "campaign_performance reversed period",
    );
    room.finish().await;
}
