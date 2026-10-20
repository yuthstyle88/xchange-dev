//! MK-7 (#1934): `campaign_performance` (M04, D-15) groups `transaction_lines` figures (`volumeThb`,
//! `realizedMarginThb`, and missing-margin count) per campaign version in a single query.

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
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

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
        let schema = unique("mk7_perf_grp");
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

    fn uri(&self, view: &str, extra: &str) -> String {
        let today = Utc::now().date_naive();
        let from = today - chrono::Duration::days(3);
        let to = today + chrono::Duration::days(1);
        format!(
            "/api/v3/views/{view}?branch={}&from={from}&to={to}{extra}",
            self.branch
        )
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

    async fn campaign(&self, status: &str, starts: &str, ends: &str) -> (Uuid, Uuid) {
        let campaign = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, $4::uuid, now())",
        )
        .bind(campaign)
        .bind(ORG)
        .bind(unique("campaign"))
        .bind(USER)
        .execute(&self.pool)
        .await
        .expect("a campaign");
        let version = self.version_of(campaign, 1, status, starts, ends).await;
        sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
            .bind(version)
            .bind(campaign)
            .execute(&self.pool)
            .await
            .expect("the current version");
        (campaign, version)
    }

    async fn version_of(
        &self,
        campaign: Uuid,
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
        .bind(ORG)
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

    async fn exchange(&self, lines: &[(&str, Option<&str>)]) -> Uuid {
        let teller = self.add_user().await;
        let shift = unique("shift");
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(&self.branch)
        .execute(&self.pool)
        .await
        .expect("close the branch's open shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
        )
        .bind(&shift)
        .bind(&self.branch)
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
        .bind(&self.branch)
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

    async fn redeem(&self, rule: Uuid, transaction: Uuid, amount: &str, days_ago: i32) {
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
        sqlx::query(
            "INSERT INTO benefit_redemptions (id, organization_id, rule_id, campaign_version_id, reservation_id, \
             transaction_id, benefit_class, amount_thb, status, reversal_id, redeemed_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, 'REDEEMED', NULL, \
             date_trunc('minute', now()) - make_interval(days => $9))",
        )
        .bind(Uuid::new_v4())
        .bind(ORG)
        .bind(rule)
        .bind(version)
        .bind(reservation)
        .bind(transaction)
        .bind(class)
        .bind(d(amount))
        .bind(days_ago)
        .execute(&self.pool)
        .await
        .expect("a redemption");
    }
}

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
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

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

fn row_by<'a>(rows: &'a Value, key: &str, id: Uuid) -> &'a Value {
    rows.as_array()
        .unwrap_or_else(|| panic!("an array: {rows}"))
        .iter()
        .find(|r| r[key] == id.to_string())
        .unwrap_or_else(|| panic!("a row with {key} = {id}: {rows}"))
}

#[actix_web::test]
async fn campaign_performance_two_versions_keep_each_versions_volume_margin_and_missing_margin() {
    let room = Room::start().await;
    let (campaign, v1) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let v2 = room
        .version_of(campaign, 2, "APPROVED", "-30 days", "30 days")
        .await;

    // v1: two exchanges with complete realized margins (total volume 4700.00, margin 60.00).
    let v1_price = room.rule(v1, "PRICE", 1).await;
    let v1_reward = room.rule(v1, "REWARD", 2).await;
    let x1 = room.exchange(&[("3200.00", Some("40.00"))]).await;
    let x2 = room
        .exchange(&[("1000.00", Some("15.50")), ("500.00", Some("4.50"))])
        .await;
    room.redeem(v1_price, x1, "10.00", 1).await;
    room.redeem(v1_reward, x1, "5.00", 1).await;
    room.redeem(v1_price, x2, "3.00", 1).await;

    // v2: different redeemed exchanges (total volume 2300.00) where one line has no margin.
    let v2_price = room.rule(v2, "PRICE", 1).await;
    let v2_reward = room.rule(v2, "REWARD", 2).await;
    let x3 = room.exchange(&[("1200.00", Some("18.00"))]).await;
    let x4 = room
        .exchange(&[("800.00", Some("12.00")), ("300.00", None)])
        .await;
    room.redeem(v2_price, x3, "4.00", 1).await;
    room.redeem(v2_reward, x4, "7.00", 1).await;

    let a = get(&room, "MARKETING", &room.uri("campaign_performance", "")).await;
    let versions = &a.ok_data()["versions"];

    let row1 = row_by(versions, "versionId", v1);
    assert_eq!(row1["campaignId"], campaign.to_string(), "{row1}");
    assert_eq!(row1["version"], 1, "{row1}");
    assert_eq!(row1["redemptions"], 3, "{row1}");
    assert_eq!(row1["priceCostThb"], "13.00", "{row1}");
    assert_eq!(row1["rewardExpenseThb"], "5.00", "{row1}");
    assert_eq!(
        row1["volumeThb"], "4700.00",
        "v1 must only sum v1's redeemed transactions: {row1}"
    );
    assert_eq!(
        row1["realizedMarginThb"]["value"], "60.00",
        "v2's missing margin must not leak into v1: {row1}"
    );
    assert!(row1["realizedMarginThb"]["unavailable"].is_null(), "{row1}");
    assert!(
        row1["attributedContributionThb"]["value"].is_null(),
        "{row1}"
    );
    assert_eq!(
        row1["attributedContributionThb"]["unavailable"], "awaitsBroker",
        "{row1}"
    );

    let row2 = row_by(versions, "versionId", v2);
    assert_eq!(row2["campaignId"], campaign.to_string(), "{row2}");
    assert_eq!(row2["version"], 2, "{row2}");
    assert_eq!(row2["redemptions"], 2, "{row2}");
    assert_eq!(row2["priceCostThb"], "4.00", "{row2}");
    assert_eq!(row2["rewardExpenseThb"], "7.00", "{row2}");
    assert_eq!(
        row2["volumeThb"], "2300.00",
        "v2 must only sum v2's redeemed transactions: {row2}"
    );
    assert!(
        row2["realizedMarginThb"]["value"].is_null(),
        "v2 has a line with no margin: {row2}"
    );
    assert_eq!(
        row2["realizedMarginThb"]["unavailable"], "awaitsLedger",
        "{row2}"
    );
    assert!(
        row2["attributedContributionThb"]["value"].is_null(),
        "{row2}"
    );
    assert_eq!(
        row2["attributedContributionThb"]["unavailable"], "awaitsLedger",
        "{row2}"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_performance_shared_transaction_counts_in_each_redeeming_version_only() {
    let room = Room::start().await;
    let (campaign, v1) = room.campaign("APPROVED", "-30 days", "30 days").await;
    let v2 = room
        .version_of(campaign, 2, "APPROVED", "-30 days", "30 days")
        .await;

    let v1_price = room.rule(v1, "PRICE", 1).await;
    let v2_reward = room.rule(v2, "REWARD", 1).await;

    let x_shared = room.exchange(&[("2000.00", Some("25.00"))]).await;
    let x_v1_only = room.exchange(&[("1000.00", Some("15.00"))]).await;
    let x_v2_only = room.exchange(&[("600.00", None)]).await;

    room.redeem(v1_price, x_shared, "5.00", 1).await;
    room.redeem(v1_price, x_v1_only, "3.00", 1).await;

    room.redeem(v2_reward, x_shared, "4.00", 1).await;
    room.redeem(v2_reward, x_v2_only, "2.00", 1).await;

    let a = get(&room, "MARKETING", &room.uri("campaign_performance", "")).await;
    let versions = &a.ok_data()["versions"];

    let row1 = row_by(versions, "versionId", v1);
    assert_eq!(row1["volumeThb"], "3000.00", "{row1}");
    assert_eq!(row1["realizedMarginThb"]["value"], "40.00", "{row1}");
    assert!(row1["realizedMarginThb"]["unavailable"].is_null(), "{row1}");
    assert_eq!(
        row1["attributedContributionThb"]["unavailable"], "awaitsBroker",
        "{row1}"
    );

    let row2 = row_by(versions, "versionId", v2);
    assert_eq!(row2["volumeThb"], "2600.00", "{row2}");
    assert!(row2["realizedMarginThb"]["value"].is_null(), "{row2}");
    assert_eq!(
        row2["realizedMarginThb"]["unavailable"], "awaitsLedger",
        "{row2}"
    );
    assert_eq!(
        row2["attributedContributionThb"]["unavailable"], "awaitsLedger",
        "{row2}"
    );

    room.finish().await;
}
