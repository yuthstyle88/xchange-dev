//! The drawer low-cash level (design `docs/design/blueprint/ledger-inventory.md` §5.1.3 "2. Low-cash warning", owner's
//! ruling (a) of 17:3x ICT 2026-11-02, and the owner's banking notice of 22:2x ICT, which replaces ruling (b)): **the Owner**
//! sets a per-branch, per-currency level (`drawerLowLevelsSave`, under `cashLimitsSave`'s permission, which the B2 ticket
//! makes EXECUTIVE-only), and finance reads it; the `cash_limits` view lists it; `my_shift` flags each
//! drawer currency at or below its level; a DESK quote names the currencies it would leave at or below the level.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. The harness is `cash_rounding_transaction_test.rs`'s `Room` (MAIN's open shift, THB 40.00 in the
//! drawer, USD BUY 32.10 published), plus the Owner account (ADMIN+EXECUTIVE), which saves every level.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    // `ROLE+ROLE` gives one person several roles (the Owner account is ADMIN+EXECUTIVE).
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = role
        .split('+')
        .map(|r| Role::parse(r).map(|r| ScopedRole::new(r, branch.clone(), from, None)))
        .collect::<Option<Vec<_>>>()?;
    // A session branch, as `v3_cash_limits_test.rs` gives every caller: the role's branch, else MAIN.
    let session_branch = Some(branch.clone().unwrap_or_else(|| BranchId::new("MAIN")));
    Some(User::new(id, ORG_1, roles, None).with_branch(session_branch))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
    }
}

struct Room {
    db: TestDb,
    teller: Who,
    manager: Who,
    customer_id: String,
    shift_id: String,
    scratch: std::path::PathBuf,
}

impl Room {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            let _ = xc_customer::install_pii_keys(keys);
        });
        let db = TestDb::start().await;
        let now_utc = Utc::now();
        let open_hhmm = (now_utc - chrono::Duration::hours(2))
            .format("%H:%M")
            .to_string();
        let close_hhmm = (now_utc + chrono::Duration::hours(2))
            .format("%H:%M")
            .to_string();
        let day_win = json!([{ "open": open_hhmm, "close": close_hhmm }]);
        let hours = json!({
            "mon": day_win, "tue": day_win, "wed": day_win, "thu": day_win,
            "fri": day_win, "sat": day_win, "sun": day_win
        });
        db.exec(&format!(
            "UPDATE branches SET timezone = 'UTC', opening_hours = '{hours}'::jsonb WHERE id = 'MAIN'"
        ))
        .await;
        db.publish("MAIN", &[USD]).await;
        db.exec(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('MAIN', 'OPEN') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'",
        )
        .await;

        let mut tx = db.pool.begin().await.expect("begin");
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(BranchId::new("MAIN")),
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal("1000.00".parse().expect("amount")),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund-safe-thb"),
                        source_key: "fund".to_owned(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund safe");
        tx.commit().await.expect("commit");

        let teller = Self::seed_person(&db, Role::Teller, Some("MAIN")).await;
        let manager = Self::seed_person(&db, Role::BranchManager, Some("MAIN")).await;

        for (key, sha) in [("PASSPORT-MAIN", "sha-passport"), ("SLIP-MAIN", "sha-slip")] {
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{key}', 'MAIN', '{}', 'image/jpeg', '{sha}', now())",
                teller.id
            ))
            .await;
        }

        let customers = CustomerService::new(BranchTx::new(Transactor::new(db.pool.clone())));
        let customer_id = customers
            .reserve(
                &BranchId::new("MAIN"),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: format!("P{}", &Uuid::new_v4().simple().to_string()[..19])
                        .to_uppercase(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "LIVE".to_owned(),
                },
                whole_second_now(),
            )
            .await
            .expect("reserve LIVE customer")
            .customer_id;

        let scratch = std::env::temp_dir().join(unique("xc-crb"));
        std::fs::create_dir_all(scratch.join("uploads")).expect("create scratch dir");

        let mut room = Self {
            db,
            teller,
            manager,
            customer_id,
            shift_id: String::new(),
            scratch,
        };

        // Open MAIN's shift with THB 20 x 2 = 40.00 through the cash-counts routes.
        let thb20 = room.denomination("THB", "20").await;
        let open = room
            .call(
                &room.teller,
                Method::POST,
                "/api/cash-counts",
                Some(json!({
                    "branch": "MAIN",
                    "kind": "OPEN",
                    "lines": [{ "currency": "THB", "denominationId": thb20, "qty": "2" }]
                })),
            )
            .await;
        assert_eq!(open.status, StatusCode::OK, "{}", open.text);
        let open_id = open.body["id"].as_str().expect("open count id");

        let verified = room
            .call(
                &room.manager,
                Method::POST,
                &format!("/api/cash-counts/{open_id}/verify"),
                Some(json!({ "branch": "MAIN" })),
            )
            .await;
        assert_eq!(verified.status, StatusCode::OK, "{}", verified.text);

        room.shift_id = sqlx::query_scalar(
            "SELECT id::text FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(&room.teller.id)
        .fetch_one(&room.db.pool)
        .await
        .expect("open shift id");

        room
    }

    async fn seed_person(db: &TestDb, role: Role, branch: Option<&str>) -> Who {
        let id = xc_testdb::people::seed_person(
            &mut db.pool.acquire().await.expect("acquire"),
            ORG_1,
            role.as_str(),
            &[xc_testdb::people::RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("seed person");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.db.pool)
        .await
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    async fn call(&self, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
        let uploads = self.scratch.join("uploads");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                uploads.to_str().expect("UTF-8 path"),
            )])),
            self.db.pool.clone(),
        );
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let mut req = atest::TestRequest::default()
            .method(method)
            .uri(uri)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, who.spec.clone()));
        if let Some(b) = &body {
            req = req
                .insert_header(("x-xchange-request", "1"))
                .insert_header((header::ORIGIN, SAME_ORIGIN))
                .insert_header((header::CONTENT_TYPE, "application/json"))
                .insert_header(("idempotency-key", unique("key")))
                .set_payload(b.to_string());
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, text, body }
    }

    async fn drawer_balance(&self, currency: CurrencyCode) -> Decimal {
        let mut tx = self.db.pool.begin().await.expect("begin");
        let balances = LedgerCashPostings::new()
            .expected_in(&mut tx, &CashPlace::Drawer(self.shift_id.clone()))
            .await
            .expect("expected_in");
        tx.commit().await.expect("commit");
        balances.get(currency).value()
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.db.finish().await;
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

const LEVELS_ROUTE: &str = "/api/cash-limits/drawer-low-levels";

/// The Owner account: one organisation-wide person holding ADMIN and EXECUTIVE.
async fn owner(room: &Room) -> Who {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let id = xc_testdb::people::seed_person(
        &mut room.db.pool.acquire().await.expect("acquire"),
        ORG_1,
        "Owner",
        &[
            xc_testdb::people::RoleRow {
                role_code: Role::Admin.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            },
            xc_testdb::people::RoleRow {
                role_code: Role::Executive.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            },
        ],
    )
    .await
    .expect("seed the Owner");
    Who {
        spec: format!("{id}|ADMIN+EXECUTIVE|*"),
        id,
    }
}

async fn save_levels(room: &Room, who: &Who, levels: Value) -> Answer {
    room.call(
        who,
        Method::PUT,
        LEVELS_ROUTE,
        Some(json!({ "branch": "MAIN", "levels": levels })),
    )
    .await
}

async fn cash_limits(room: &Room, who: &Who) -> Value {
    let a = room
        .call(
            who,
            Method::GET,
            "/api/v3/views/cash_limits?branch=MAIN",
            None,
        )
        .await;
    assert_eq!(a.status, StatusCode::OK, "cash_limits: {}", a.text);
    a.body["data"].clone()
}

/// The teller's `my_shift` balance row for `code`.
async fn my_balance(room: &Room, code: &str) -> Value {
    let a = room
        .call(
            &room.teller,
            Method::GET,
            "/api/v3/views/my_shift?branch=MAIN",
            None,
        )
        .await;
    assert_eq!(a.status, StatusCode::OK, "my_shift: {}", a.text);
    a.body["data"]["balances"]
        .as_array()
        .unwrap_or_else(|| panic!("balances: {}", a.text))
        .iter()
        .find(|b| b["code"] == code)
        .cloned()
        .unwrap_or_else(|| panic!("no {code} row: {}", a.text))
}

/// L1–L2: the Owner saves two levels; the save and the `cash_limits` view list them, by currency, normalized.
#[actix_web::test]
async fn l1_the_owner_saves_levels_and_cash_limits_lists_them() {
    let room = Room::start().await;
    let own = owner(&room).await;

    let a = save_levels(
        &room,
        &own,
        json!([{ "currency": "USD", "lowLevel": "10" }, { "currency": "THB", "lowLevel": "50.00" }]),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let expected =
        json!([{ "currency": "THB", "lowLevel": "50" }, { "currency": "USD", "lowLevel": "10" }]);
    assert_eq!(a.body["drawerLowLevels"], expected, "{}", a.text);
    assert_eq!(cash_limits(&room, &own).await["drawerLowLevels"], expected);
    room.finish().await;
}

/// L3: a save replaces the whole set: a currency left out is deleted.
#[actix_web::test]
async fn l3_a_save_replaces_the_whole_set() {
    let room = Room::start().await;
    let own = owner(&room).await;
    let first = save_levels(
        &room,
        &own,
        json!([{ "currency": "THB", "lowLevel": "50" }, { "currency": "USD", "lowLevel": "10" }]),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    let second = save_levels(&room, &own, json!([{ "currency": "USD", "lowLevel": "5" }])).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert_eq!(
        cash_limits(&room, &own).await["drawerLowLevels"],
        json!([{ "currency": "USD", "lowLevel": "5" }])
    );
    let empty = save_levels(&room, &own, json!([])).await;
    assert_eq!(empty.status, StatusCode::OK, "{}", empty.text);
    assert_eq!(cash_limits(&room, &own).await["drawerLowLevels"], json!([]));
    room.finish().await;
}

/// L4: invalid input is refused `400 validationFailed` naming `levels`, and writes nothing.
#[actix_web::test]
async fn l4_invalid_levels_are_refused_and_write_nothing() {
    let room = Room::start().await;
    let own = owner(&room).await;
    for (bad, what) in [
        (json!([{ "currency": "THB", "lowLevel": "0" }]), "zero"),
        (json!([{ "currency": "THB", "lowLevel": "-5" }]), "negative"),
        (
            json!([{ "currency": "THB", "lowLevel": "abc" }]),
            "not a decimal",
        ),
        (
            json!([{ "currency": "ZZZ", "lowLevel": "5" }]),
            "unknown currency",
        ),
        (
            json!([{ "currency": "THB", "lowLevel": "5" }, { "currency": "THB", "lowLevel": "6" }]),
            "duplicate currency",
        ),
    ] {
        let a = save_levels(&room, &own, bad).await;
        assert_eq!(
            (a.status, a.code(), a.field()),
            (StatusCode::BAD_REQUEST, "validationFailed", Some("levels")),
            "{what}: {}",
            a.text
        );
    }
    assert_eq!(
        room.db
            .count("SELECT count(*) FROM cash_drawer_low_levels")
            .await,
        0
    );
    room.finish().await;
}

/// L5: the save is `cashLimitsSave`'s permission: a TELLER is refused and nothing is written.
#[actix_web::test]
async fn l5_a_teller_may_not_save_levels() {
    let room = Room::start().await;
    let a = save_levels(
        &room,
        &room.teller,
        json!([{ "currency": "THB", "lowLevel": "50" }]),
    )
    .await;
    // A policy refusal is pinned by its code, as `v3_cash_limits_test.rs` does for `cashLimitsSave`.
    assert_ne!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.code(), "roleNotAllowedHere", "{}", a.text);
    assert_eq!(
        room.db
            .count("SELECT count(*) FROM cash_drawer_low_levels")
            .await,
        0
    );
    room.finish().await;
}

/// L6: `my_shift` flags a drawer currency at or below its level; a currency with no level is `null` / `false`.
#[actix_web::test]
async fn l6_my_shift_flags_a_drawer_currency_at_or_below_its_level() {
    let room = Room::start().await;
    let own = owner(&room).await;
    let thb = my_balance(&room, "THB").await;
    assert_eq!(
        (thb["lowCashLevel"].clone(), thb["low"].clone()),
        (Value::Null, json!(false)),
        "no level yet"
    );

    let a = save_levels(
        &room,
        &own,
        json!([{ "currency": "THB", "lowLevel": "50" }]),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let thb = my_balance(&room, "THB").await;
    assert_eq!(thb["lowCashLevel"], json!("50"), "{thb}");
    assert_eq!(thb["low"], json!(true), "40.00 ≤ 50: {thb}");

    let a = save_levels(
        &room,
        &own,
        json!([{ "currency": "THB", "lowLevel": "40" }]),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        my_balance(&room, "THB").await["low"],
        json!(true),
        "at the level counts as low"
    );

    let a = save_levels(
        &room,
        &own,
        json!([{ "currency": "THB", "lowLevel": "39.99" }]),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        my_balance(&room, "THB").await["low"],
        json!(false),
        "40.00 > 39.99"
    );
    room.finish().await;
}

/// L7: a DESK quote the drawer can pay but that would leave THB at or below its level names THB in
/// `belowLowCashLevel`; one that leaves it above names nothing.
#[actix_web::test]
async fn l7_a_quote_names_the_currencies_it_would_leave_at_or_below_the_level() {
    let room = Room::start().await;
    let own = owner(&room).await;
    let a = save_levels(
        &room,
        &own,
        json!([{ "currency": "THB", "lowLevel": "10" }]),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);

    let quote = |qty: &'static str| {
        let room = &room;
        async move {
            room.call(
                &room.teller,
                Method::POST,
                "/api/quotes",
                Some(json!({
                    "branch": "MAIN",
                    "customerId": room.customer_id,
                    "paymentMethod": "Cash",
                    "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": qty }]
                })),
            )
            .await
        }
    };
    // USD 1 settles THB 32.25: 40.00 − 32.25 = 7.75 ≤ 10 → named; the drawer can still pay.
    let q = quote("1").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["belowLowCashLevel"], json!(["THB"]), "{}", q.text);

    let a = save_levels(&room, &own, json!([{ "currency": "THB", "lowLevel": "5" }])).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let q = quote("1").await;
    assert_eq!(
        q.body["belowLowCashLevel"],
        json!([]),
        "7.75 > 5: {}",
        q.text
    );
    room.finish().await;
}

/// L8: a branch with no level row: `drawerLowLevels: []`.
#[actix_web::test]
async fn l8_no_levels_reads_as_an_empty_list() {
    let room = Room::start().await;
    let own = owner(&room).await;
    assert_eq!(cash_limits(&room, &own).await["drawerLowLevels"], json!([]));
    room.finish().await;
}

/// L9 (the owner's principle, `docs/design/user-set-settings.md`, PR #5993): the Owner account (ADMIN+EXECUTIVE) can save
/// the drawer levels too, and reads them back. `CashLimitsManage` comes with EXECUTIVE.
#[actix_web::test]
async fn l9_the_owner_account_saves_and_reads_the_levels() {
    let room = Room::start().await;
    let own = owner(&room).await;
    let a = save_levels(
        &room,
        &own,
        json!([{ "currency": "THB", "lowLevel": "25" }]),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        cash_limits(&room, &own).await["drawerLowLevels"],
        json!([{ "currency": "THB", "lowLevel": "25" }])
    );
    room.finish().await;
}
