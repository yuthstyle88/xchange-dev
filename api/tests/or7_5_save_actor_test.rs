//! Ticket #1024 (OR-7.5) — the save of an exchange moves from the old identity model
//! (`xc_auth::domain::policy::value_objects::{actor::Actor, role::Role, actor_kind::ActorKind}`) to
//! `access::Actor` plus `Permission`, and the save steps stop resolving `users.legacy_employee_id`
//! (design `docs/design/blueprint/org-roles-branches.md` §3.3, §4.3; ticket §7–9 *"a save by each v3
//! role succeeds or is refused exactly as today"*).
//! Written before the implementation by the up-front test writer (claude/dell-claude-4, on the xchange BE
//! lane controller claude/ad05bf8d's order, 2026-10-13). Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! What these tests pin, measured on `main` `f880f2fd` through the real routes (`POST /api/transactions`,
//! `/confirm`, `/complete`, `bootstrap::configure`), so every registered save step runs:
//! - `each_v3_role_saves_or_is_refused_as_today`: one caller per v3 role (the `one_of_each_role` ten). FRONT
//!   creates, confirms and completes; the audit row and the stored exchange name the caller, and the audit's
//!   `actor_role` is `FRONT`. Every other role is refused at create with 400 `roleNotAllowedHere`, and nothing
//!   is written. **Green on `main`**: it pins today's outcome, which the move must keep. An outcome that has to
//!   change is named in the PR (ticket "The rule for every OR-7 child"), not changed here.
//! - `a_teller_with_no_legacy_employee_id_saves`: a Blueprint user whose `users.legacy_employee_id` is NULL,
//!   TELLER at MAIN, saves and completes, and is recorded by `users.id`. **Green on `main`**: no save path reads
//!   `legacy_employee_id` there any more (`git grep -n legacy_employee_id origin/main -- api/src/application`
//!   finds only the `#[cfg(test)]` seed in `booking_save_step.rs` and a `None` in `customer_signup.rs`), so this
//!   row guards the removal and cannot be red by result.
//! - `a_teller_with_a_legacy_employee_id_is_recorded_by_users_id` (fix round, ruling on #1024 after claude/dell-claude-2's
//!   must-fix 1): a Blueprint TELLER whose `users.legacy_employee_id` is `EMP-OLD-7` saves and is recorded by its
//!   `users.id` (the audit `actor_id`, `staff_id`, `recorded_by`), never by `EMP-OLD-7`. **Green on `main`**; red
//!   against a move that prefers the column (`COALESCE(legacy_employee_id, users.id)`).
//! - `the_v3_commands_no_longer_bridge_through_legacy_actor` (source-scan pin, lane controller's ruling on #1024
//!   must-fix 3): the five command files that carried `// OR-7 bridge, removed by #1024` contain no `legacy_actor` in
//!   code (line comments stripped as `no_ts_export_test.rs` strips them) and no `OR-7 bridge` marker (read in the raw
//!   text: the marker is itself a comment). **Red on `main`** (the bridges are there); the three tests above are
//!   characterisation tests and pass on both.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips. Each
//! test works in a schema of its own.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{Value, json};
use tx5_common::*;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

/// The first organisation, which MAIN belongs to.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// CI-4 (#651): the test PII key, as `tx4_create_test.rs`.
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// The ten v3 roles, in `one_of_each_role`'s order (`rpc_common/mod.rs`).
const V3_ROLES: [&str; 10] = [
    "OWNER",
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

static PII: Once = Once::new();

/// The id of the caller of v3 role `role`: the owner's `owner`, an employee's `employee-<role>`.
fn caller_id(role: &str) -> String {
    if role == "OWNER" {
        "owner".to_owned()
    } else {
        format!("employee-{}", role.to_ascii_lowercase())
    }
}

// ------------------------------------------------------------ who is signed in

/// The v3 caller named by the header `x-test-as` (a v3 role name): the owner, or an employee of MAIN. Its
/// `access::Actor` is the B1-mapped one, as `SessionReader::callers` makes it for a password session.
struct V3Sessions;

impl SessionReader for V3Sessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let role = req
            .headers()
            .get("x-test-as")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        Box::pin(async move {
            if role == "OWNER" {
                return Sessions {
                    owner: Some(Actor::owner("owner")),
                    staff: None,
                };
            }
            let staff = Role::STAFF
                .iter()
                .find(|r| r.as_str() == role)
                .and_then(|r| Actor::staff(caller_id(&role), *r, BranchId::new("MAIN")));
            Sessions { owner: None, staff }
        })
    }
}

/// A Blueprint session of `user_id`, TELLER at MAIN, made the way `AccountSessionReader::callers` makes it for a
/// `__Host-xc_session` (the `access::Actor` of the `users` row, and `legacy_actor` of it).
struct TellerSession {
    user_id: String,
}

impl SessionReader for TellerSession {
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

    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let user = xc_auth::domain::access::actor::Actor::new(
                &self.user_id,
                ORG,
                vec![ScopedRole::new(
                    xc_auth::domain::access::role::Role::Teller,
                    Some(BranchId::new("MAIN")),
                    chrono::DateTime::<Utc>::UNIX_EPOCH,
                    None,
                )],
                None,
            );
            Callers {
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

// ------------------------------------------------------------ the counter

/// `tx5_common::Counter` (MAIN's open shift and its funded drawer) plus a DEMO customer.
struct Shop {
    c: Counter,
    customer: String,
}

/// What one save answered: `create`, then `confirm` and `complete` when the step before succeeded.
#[derive(Debug, PartialEq, Eq)]
enum Saved {
    /// Refused at create with this status and `error.code`.
    Refused(u16, String),
    /// Created, confirmed and completed: the final status, the audit's `(actor_id, actor_role)` of the create,
    /// and the stored `(staff_id, recorded_by)`.
    Completed {
        status: String,
        audit: (String, String),
        recorded: (String, String),
    },
}

impl Shop {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            xc_customer::install_pii_keys(keys).expect("one test PII key per process");
        });
        let c = Counter::start().await;
        let customer = CustomerService::new(BranchTx::new(Transactor::new(c.desk.db.pool.clone())))
            .reserve(
                &BranchId::new("MAIN"),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: "XC1112223".to_owned(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "DEMO".to_owned(),
                },
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("reserve: {e}"))
            .customer_id;
        Self { c, customer }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    async fn count(&self, sql: &str) -> i64 {
        self.c.desk.db.count(sql).await
    }

    /// A passport scan at MAIN owned by `owner` (create checks the scan is the caller's, #672 Q2).
    async fn scan_of(&self, owner: &str) -> String {
        let key = unique("PASSPORT");
        self.c
            .desk
            .db
            .exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{key}', 'MAIN', '{owner}', 'image/jpeg', 'sha-{key}', now())"
            ))
            .await;
        key
    }

    /// `POST uri` as the caller `reader` reads from `x-test-as: who`, with a fresh `Idempotency-Key`.
    async fn post(
        &self,
        reader: Arc<dyn SessionReader>,
        who: &str,
        uri: &str,
        body: &Value,
    ) -> (StatusCode, Value) {
        let dir = self.c.scratch.join("uploads");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                dir.to_str().expect("a UTF-8 path"),
            )])),
            self.c.desk.db.pool.clone(),
        );
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::post()
            .uri(uri)
            .insert_header((header::HOST, "xc.test"))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", "http://xc.test"))
            .insert_header(("content-type", "application/json"))
            .insert_header(("x-test-as", who))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        )
    }

    /// A cash BUY of 100 USD at MAIN, created, confirmed and completed by the caller `who`, whose passport scan
    /// is owned by `scan_owner`.
    async fn save_as(&self, reader: Arc<dyn SessionReader>, who: &str, scan_owner: &str) -> Saved {
        let q = self
            .c
            .desk
            .quote("MAIN", Side::Buy, "100", None, Utc::now())
            .await;
        let scan = self.scan_of(scan_owner).await;
        let body = json!({
            "quoteId": q.id.to_string(),
            "customerId": self.customer,
            "passport": { "uploadKey": scan },
            "payment": {
                "method": "Cash",
                "reference": "R-1",
                "account": "Cash drawer 1",
                "bankFeeThb": "0.00",
                "paidAt": Utc::now().to_rfc3339(),
            },
            "valueDate": "2026-10-06",
            "notes": "",
            "countConfirmed": true,
        });
        let (status, made) = self
            .post(reader.clone(), who, "/api/transactions", &body)
            .await;
        if status != StatusCode::OK {
            return Saved::Refused(
                status.as_u16(),
                made["error"]["code"].as_str().unwrap_or("").to_owned(),
            );
        }
        let id = made["id"]
            .as_str()
            .expect("create answers an id")
            .to_owned();
        let audit: (String, String) = sqlx::query_as(
            "SELECT actor_id, actor_role FROM audit_events \
             WHERE object_ref = $1 AND action = 'transaction.created'",
        )
        .bind(&id)
        .fetch_one(&self.c.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("{who}: the create's audit row: {e}"));
        for step in ["confirm", "complete"] {
            let (status, answer) = self
                .post(
                    reader.clone(),
                    who,
                    &format!("/api/transactions/{id}/{step}"),
                    &json!({}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{who}: {step}: {answer}");
        }
        let (final_status, staff_id, recorded_by): (String, String, String) = sqlx::query_as(
            "SELECT status, staff_id, recorded_by FROM transactions WHERE id = $1::uuid",
        )
        .bind(&id)
        .fetch_one(&self.c.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("{who}: the stored exchange: {e}"));
        Saved::Completed {
            status: final_status,
            audit,
            recorded: (staff_id, recorded_by),
        }
    }
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn each_v3_role_saves_or_is_refused_as_today() {
    let shop = Shop::start().await;
    let reader: Arc<dyn SessionReader> = Arc::new(V3Sessions);
    for role in V3_ROLES {
        let id = caller_id(role);
        let before = shop.count("SELECT count(*) FROM transactions").await;
        let got = shop.save_as(reader.clone(), role, &id).await;
        let expected = if role == "FRONT" {
            Saved::Completed {
                status: "COMPLETED".to_owned(),
                audit: (id.clone(), "FRONT".to_owned()),
                recorded: (id.clone(), id.clone()),
            }
        } else {
            Saved::Refused(400, "roleNotAllowedHere".to_owned())
        };
        assert_eq!(got, expected, "{role}: the save's outcome is today's");
        if role != "FRONT" {
            assert_eq!(
                shop.count("SELECT count(*) FROM transactions").await,
                before,
                "{role}: a refused save writes no exchange"
            );
            assert_eq!(
                shop.count(&format!(
                    "SELECT count(*) FROM audit_events WHERE actor_id = '{id}'"
                ))
                .await,
                0,
                "{role}: a refused save writes no audit row"
            );
        }
    }
    shop.finish().await;
}

#[actix_web::test]
async fn a_teller_with_no_legacy_employee_id_saves() {
    let shop = Shop::start().await;
    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Blueprint teller', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .fetch_one(&shop.c.desk.db.pool)
    .await
    .expect("insert the teller's user");
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM users WHERE id::text = '{user_id}' AND legacy_employee_id IS NULL"
        ))
        .await,
        1,
        "the teller has no legacy_employee_id"
    );
    let reader: Arc<dyn SessionReader> = Arc::new(TellerSession {
        user_id: user_id.clone(),
    });
    let got = shop.save_as(reader, "teller", &user_id).await;
    assert_eq!(
        got,
        Saved::Completed {
            status: "COMPLETED".to_owned(),
            audit: (user_id.clone(), "FRONT".to_owned()),
            recorded: (user_id.clone(), user_id.clone()),
        },
        "a user with no legacy_employee_id saves, recorded by users.id"
    );
    shop.finish().await;
}

#[actix_web::test]
async fn a_teller_with_a_legacy_employee_id_is_recorded_by_users_id() {
    const LEGACY: &str = "EMP-OLD-7";
    let shop = Shop::start().await;
    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         VALUES ($1::uuid, 'Blueprint teller with a v3 past', 'ACTIVE', 'test', $2) RETURNING id::text",
    )
    .bind(ORG)
    .bind(LEGACY)
    .fetch_one(&shop.c.desk.db.pool)
    .await
    .expect("insert the teller's user");
    assert_ne!(user_id, LEGACY, "the two ids differ");
    let reader: Arc<dyn SessionReader> = Arc::new(TellerSession {
        user_id: user_id.clone(),
    });
    let got = shop.save_as(reader, "teller", &user_id).await;
    let Saved::Completed {
        status,
        audit,
        recorded,
    } = got
    else {
        panic!("the teller with a legacy_employee_id saves: {got:?}");
    };
    assert_eq!(status, "COMPLETED");
    for (what, value) in [
        ("the create's audit actor_id", &audit.0),
        ("staff_id", &recorded.0),
        ("recorded_by", &recorded.1),
    ] {
        assert_ne!(value, LEGACY, "{what} is never the legacy_employee_id");
        assert_eq!(value, &user_id, "{what} is the user's users.id");
    }
    assert_eq!(audit.1, "FRONT", "the audit's actor_role is today's");
    shop.finish().await;
}

// ------------------------------------------------------------ the bridges are gone

/// The five v3 command files that carried `// OR-7 bridge, removed by #1024` (§5, ruling R2).
const BRIDGED_COMMANDS: [&str; 5] = [
    "complete",
    "quotes",
    "reversal_post",
    "reversal_requests",
    "transactions",
];

/// `line` without its `//` comment, leaving a `//` inside a string literal (as `no_ts_export_test.rs`).
fn strip_line_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        if bytes[i] == b'\\' {
            escaped = true;
            continue;
        }
        if bytes[i] == b'"' {
            in_quote = !in_quote;
        } else if !in_quote && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
    }
    line
}

#[test]
fn the_v3_commands_no_longer_bridge_through_legacy_actor() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/http/v3/commands");
    let mut hits = Vec::new();
    for name in BRIDGED_COMMANDS {
        let path = dir.join(format!("{name}.rs"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (n, line) in text.lines().enumerate() {
            if strip_line_comment(line).contains("legacy_actor") || line.contains("OR-7 bridge") {
                hits.push(format!("commands/{name}.rs:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the v3 commands build SaveActor from access::Actor, with no legacy_actor bridge and no OR-7 bridge \
         marker. {} site(s):\n{}",
        hits.len(),
        hits.join("\n")
    );
}
