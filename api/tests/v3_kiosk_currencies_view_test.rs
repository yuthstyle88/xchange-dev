//! Ticket #399 (V1-kiosk-currencies, `docs/design/xchange-v3-web.md` §4, §5.1, §6 row
//! `/kiosk-currencies.html`): the `kiosk_currencies` view the v3 currency screen reads instead
//! of v2's `manage` on `POST /api/kiosk`. Written before the implementation by the lane
//! controller. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/kiosk_currencies` — a `BranchView` inheriting `(EntryPath::Kiosk,
//!   "manage")`: it answers exactly whom `Policy::authorize` lets call `manage`, on the branch
//!   `Policy::branch` gives (ticket §7.1, §8, V1).
//! - `data` is `{currencies, kbank}`: the 152 currencies of the table in the table's order,
//!   each `{code, name, nameEn, countries, buy, sell, updated, partnerName, partnerCost,
//!   enabled, kbank}`, and the KBank reference `{source, publishedAt, round}` (§7.3).
//! - Rates are the decimal strings the rate holds, never JSON numbers (W7). `updated` is
//!   byte-identical to `manage`'s, because `configure` compares it (§7.3).
//! - The KBank comparison is computed on the server: every value is a decimal string with
//!   exactly five places, or `null` where v2 printed `—`; a difference is shop − bank, and
//!   `null` unless both sides are > 0 (§7.3, C14).
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`). The test's clock is advanced before
//! every write, so no two writes share an instant. Branch isolation is checked with two
//! branches holding different data.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c), where #603 contradicts what was
//! pinned here (`docs/design/blueprint/rates.md` §4.8, §9 RE-6):
//! - item 3: the view reads the branch's current rate publication, not `rates`. `Room::set_rate`
//!   now publishes (every line the branch was given so far, plus the new one) instead of
//!   inserting a `rates` row; rates are answered as the 8-place strings the publication holds
//!   (`"32.10000000"`, was `"32.1000"`); each currency gains `unitSize`; `updated` is the
//!   publication's `published_at`, so it is no longer `manage`'s.
//! - items 1 and 2: `configure` writes no rate and enables only a published currency, so the
//!   writers `configure` and `adminSaveRate` move the version but no longer change a rate the
//!   view answers, and every enabling `configure` here first publishes its currency.
//!   The rest is as the #399 author wrote it.

mod common;
mod compat_common;
mod publication_seed;
mod rpc_common;
mod snapshot_common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::test_callers;

const ORIGIN: &str = "http://desk.test";
const KIOSK: &str = "/api/kiosk";
const RPC: &str = "/api/rpc";
const STAFF_RPC: &str = "/api/staff/rpc";
const VIEW: &str = "kiosk_currencies";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

/// A RATES employee of `MAIN` whose first password is unchanged: nobody on this path.
const RATES_MUST_CHANGE: &str = "RATES_MUST_CHANGE";

const OWNER_OR_RATES: &str = "Owner or Rates access required.";
const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";

/// The keys of `data` (§7.3), plus `freshness` (RF-3, rates.md §4.10; re-expected by the RF-3 ticket author).
const DATA_KEYS: [&str; 3] = ["currencies", "freshness", "kbank"];
/// The keys of one currency (§7.3) — none of `manage`'s `symbol`, `requestId`, `botReference`;
/// plus `unitSize` (#603 item 3).
const CURRENCY_KEYS: [&str; 12] = [
    "code",
    "name",
    "nameEn",
    "countries",
    "buy",
    "sell",
    "unitSize",
    "updated",
    "partnerName",
    "partnerCost",
    "enabled",
    "kbank",
];
/// The keys of one KBank comparison row (§7.3).
const COMPARISON_KEYS: [&str; 7] = [
    "label",
    "bankBuy",
    "shopBuy",
    "buyDifference",
    "bankSell",
    "shopSell",
    "sellDifference",
];

/// v2's `kbank-reference.json` (`xchange-v2-source@4d0bdbb:src/devices-public/`), committed
/// beside the view with the up-front tests.
const KBANK_FILE: &str = "src/presentation/http/v3/views/kiosk_currencies.kbank.json";
const KBANK_BYTES: usize = 3556;
const KBANK_SHA256: &str = "a31c8f27f2af74fe8f07bbed61bde63c4421a71e1b144b10ae21068eefd333c4";

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A number no other call of this test binary has used.
fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A request id in v2's shape, different on every call.
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

/// A GET as the screen sends it: no body, no marker header.
fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{VIEW}")
    } else {
        format!("/api/v3/views/{VIEW}?{query}")
    }
}

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// The 152-entry currency table `manage` reads (`dto/kiosk.rs:30-38`).
fn currency_table() -> Vec<Value> {
    let text = std::fs::read_to_string(manifest("tests/fixtures/kiosk/currency_table.json"))
        .expect("read currency_table.json");
    let raw: Value = serde_json::from_str(&text).expect("currency_table.json is JSON");
    raw["currencies"]
        .as_array()
        .expect("the table's currencies")
        .clone()
}

/// The committed KBank file.
fn kbank_file() -> Value {
    let text = std::fs::read_to_string(manifest(KBANK_FILE))
        .unwrap_or_else(|e| panic!("{KBANK_FILE}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{KBANK_FILE} is JSON: {e}"))
}

/// A bank figure of the file as the view must print it: the number's own text, read as a
/// decimal (never through `f64` arithmetic), with five places; `null` when absent or not > 0.
fn bank_text(value: &Value) -> Value {
    match value {
        Value::Number(n) => {
            let d = Decimal::from_str(&n.to_string())
                .unwrap_or_else(|e| panic!("{n} is a decimal: {e}"));
            if d > Decimal::ZERO {
                assert!(d.scale() <= 5, "the file has at most five places: {n}");
                json!(format!("{d:.5}"))
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

/// A comparison row with every value given (`None` = `null`).
fn row(label: &str, values: [Option<&str>; 6]) -> Value {
    let [
        bank_buy,
        shop_buy,
        buy_diff,
        bank_sell,
        shop_sell,
        sell_diff,
    ] = values;
    json!({
        "label": label,
        "bankBuy": bank_buy, "shopBuy": shop_buy, "buyDifference": buy_diff,
        "bankSell": bank_sell, "shopSell": shop_sell, "sellDifference": sell_diff,
    })
}

fn currency<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("currencies is an array: {data}"))
        .iter()
        .find(|c| c["code"] == json!(code))
        .unwrap_or_else(|| panic!("{code} in the view's currencies"))
}

fn is_unchanged(body: &Value, version: i64, what: &str) {
    assert_eq!(
        body,
        &json!({ "version": version, "changed": false }),
        "{what}: exactly the version and changed: false, no data"
    );
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
    /// The lines published so far, per branch (#603: `set_rate` publishes them all again).
    published: Mutex<BTreeMap<String, BTreeMap<String, publication_seed::Line>>>,
}

impl Room {
    /// `MAIN` (from the migration) and `B9`; the owner, one employee of every staff role in
    /// `MAIN`, and a RATES employee of `MAIN` who must change the first password, all signed
    /// in. The two branches hold different counter rates for USD: `MAIN` 32.1 / 33,
    /// `B9` 31.5 / 32.75.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, B9_NAME).await;
        let app = rpc_app!(desk);
        let mut people = one_of_each_role();
        let mut must_change: Person = people
            .iter()
            .find(|p| p.key == "RATES")
            .expect("a RATES employee")
            .clone();
        must_change.key = RATES_MUST_CHANGE.to_string();
        must_change.id = "00000000-0000-4000-8000-0000000000fe".to_string();
        must_change.username = "rateschange".to_string();
        must_change.must_change = true;
        people.push(must_change);
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let room = Room {
            desk,
            app,
            cookies,
            t0,
            published: Mutex::new(BTreeMap::new()),
        };
        room.set_rate(MAIN, "USD", "32.1", "33").await;
        room.set_rate(B9, "USD", "31.5", "32.75").await;
        room
    }

    /// The test's clock moves forward one second (and a fraction) on every call: no two
    /// writes share an instant.
    fn tick(&self) -> DateTime<Utc> {
        let n = next() as i64;
        let t = self.t0 + Duration::seconds(n) + Duration::microseconds(123_456);
        self.desk.clock.set(t);
        t
    }

    fn cookie(&self, who: &str) -> String {
        match who.split_once('+') {
            Some((a, b)) => format!("{}; {}", self.cookies[a], self.cookies[b]),
            None => self.cookies[who].clone(),
        }
    }

    /// `GET path` as `who` (`""` = nobody; `OWNER+FRONT` = both cookies in one browser).
    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    /// The view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, query: &str) -> Value {
        let answer = self.get(who, &path(query)).await;
        assert_eq!(
            answer.status,
            200,
            "GET {} as {who:?}: {:.400}",
            path(query),
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "GET {}: never cached",
            path(query)
        );
        answer.json()
    }

    /// `data` of the view read with no `since`.
    async fn data(&self, who: &str, query: &str) -> Value {
        let body = self.view(who, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {} with no since: {body}",
            path(query)
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: data in {body}", path(query)))
    }

    async fn post(&self, who: &str, route: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        let call = Call::action(ORIGIN, route, action, payload);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    async fn ok(&self, who: &str, route: &str, action: &str, payload: Value) -> Value {
        let answer = self.post(who, route, action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} on {route} as {who} with {payload}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// `configure` on `/api/kiosk` as the screen sends it: rates as strings.
    async fn configure(&self, who: &str, payload: Value) {
        let saved = self.ok(who, KIOSK, "configure", payload).await;
        assert_eq!(saved, json!({ "saved": true }), "configure as {who}");
    }

    /// v2's `manage` on `/api/kiosk` as the owner, for `branch`.
    async fn manage(&self, branch: &str) -> Value {
        self.ok("OWNER", KIOSK, "manage", json!({ "branchId": branch }))
            .await
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    /// A published rate behind the routes (moves no version: only reads without `since` see it).
    /// #603: the branch's next publication holds every line it was given so far, with this one.
    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        self.set_rate_with_partner(branch, code, buy, sell, "", None)
            .await;
    }

    async fn set_rate_with_partner(
        &self,
        branch: &str,
        code: &str,
        buy: &str,
        sell: &str,
        partner_name: &str,
        partner_cost: Option<&str>,
    ) {
        let at = self.tick();
        let eight = |text: &str| {
            let d = Decimal::from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            format!("{d:.8}")
        };
        let mut line = publication_seed::both(code, buy, sell);
        if let Some(cost) = partner_cost {
            line.partner = publication_seed::Partner::Quote(partner_name.to_owned(), eight(cost));
        }
        let lines: Vec<publication_seed::Line> = {
            let mut published = self.published.lock().expect("not poisoned");
            let mine = published.entry(branch.to_owned()).or_default();
            mine.insert(code.to_owned(), line);
            mine.values().cloned().collect()
        };
        publication_seed::publish(&self.desk.db.pool, branch, at, &lines).await;
    }

    /// A kiosk switch behind the routes.
    async fn switch(&self, branch: &str, code: &str, enabled: bool) {
        sqlx::query(
            "INSERT INTO kiosk_currencies (branch_id, code, enabled) VALUES ($1, $2, $3) \
             ON CONFLICT (branch_id, code) DO UPDATE SET enabled = excluded.enabled",
        )
        .bind(branch)
        .bind(code)
        .bind(enabled)
        .execute(&self.desk.db.pool)
        .await
        .expect("switch a kiosk currency");
    }

    /// Every table the view can read, renamed away: loading the view now fails, while
    /// `branch_versions` (the `since` check) and the sign-in tables stay.
    /// `branches` stays too: the core's branch check reads it before `since` (#500).
    async fn hide_the_views_tables(&self) {
        for table in [
            "published_rate_lines",
            "rate_publications",
            "kiosk_currencies",
            "currencies",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {table} RENAME TO hidden_{table}"
            )))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("rename {table}: {e}"));
        }
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

const STAFF: [(&str, &[Role]); 9] = [
    ("FRONT", &[Role::Teller]),
    ("BACKEND", &[Role::BranchManager]),
    ("MANAGER", &[Role::BranchManager]),
    ("RATES", &[Role::RateManager]),
    ("MONITOR", &[Role::VerificationReviewer]),
    ("FINANCE", &[Role::Finance]),
    ("SYSTEM", &[Role::Admin]),
    ("DOCUMENTS", &[Role::VerificationReviewer]),
    ("DOCUMENT_REVIEW", &[Role::VerificationReviewer]),
];

/// The actor `Policy` sees for each caller key of [`Room`] on the path `Kiosk`.
fn callers() -> Vec<(String, Option<Actor>)> {
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(test_callers::owner("owner"))),
        // a session whose first password is unchanged is nobody (`kiosk.rs:182-186`)
        (RATES_MUST_CHANGE.to_string(), None),
    ];
    for (name, roles) in STAFF {
        let staff = Some(test_callers::staff("employee", roles, BranchId::new(MAIN)));
        callers.push((name.to_string(), staff));
        // the owner and an employee in one browser: a view not on StaffRpc answers the owner
        callers.push((format!("OWNER+{name}"), Some(test_callers::owner("owner"))));
    }
    callers
}

// ── who may read ─────────────────────────────────────────────────────────────

/// OR-4b (#767, D-4, design §2 and §02 "Server ตรวจ … branch scope"): an employee who names a
/// branch outside their scope is refused 403 `outOfScope`, where v3 answered their own branch.
fn assert_out_of_scope(answer: &rpc_common::Answer, label: &str) {
    assert_eq!(answer.status, 403, "{label}: {:.300}", answer.text);
    assert_eq!(
        answer.json()["error"]["code"],
        serde_json::json!("outOfScope"),
        "{label}: {:.300}",
        answer.text
    );
}

/// OR-4b (#767): the rule a route asks once it is on `Policy::authorize` (design §4.2): the caller
/// as B1 maps them (§3.3), naming `named` (D-4: a branch outside the caller's scope is refused).
fn blueprint_rule(
    actor: Option<&Actor>,
    path: EntryPath,
    action: &str,
    named: Option<&str>,
) -> Result<
    xc_auth::domain::access::Grant,
    xc_auth::domain::policy::value_objects::policy_error::PolicyError,
> {
    use xc_auth::domain::policy::entities::policy::Policy;
    let act = xc_auth::domain::policy::value_objects::action::Action::new(action);
    Policy::authorize(
        actor,
        path,
        &act,
        named.map(xc_kernel::ids::BranchId::new).as_ref(),
        chrono::Utc::now(),
    )
}

/// §8: nobody signed in gets the `manage` row's refusal, as a v3 refusal.
#[actix_web::test]
async fn nobody_signed_in_gets_the_kiosk_manage_refusal() {
    let room = Room::open().await;
    for query in ["", "branch=MAIN", "branch=B9"] {
        let answer = room.get("", &path(query)).await;
        assert_eq!(answer.status, 400, "{query:?}: {:.300}", answer.text);
        assert_eq!(
            answer.json(),
            json!({ "error": { "code": "kioskOwnerOrRates", "message": OWNER_OR_RATES } }),
            "{query:?}"
        );
    }
    room.finish().await;
}

/// §8: a staff role other than RATES, and a RATES employee who must change the password,
/// get the same refusal with no data.
#[actix_web::test]
async fn other_staff_roles_and_a_must_change_employee_get_the_same_refusal() {
    let room = Room::open().await;
    let mut who: Vec<String> = STAFF
        .iter()
        .filter(|(name, _)| *name != "RATES")
        .map(|(name, _)| (*name).to_string())
        .collect();
    who.push(RATES_MUST_CHANGE.to_string());
    assert_eq!(who.len(), 9, "eight roles and the must-change employee");
    for key in &who {
        let answer = room.get(key, &path("branch=MAIN")).await;
        assert_eq!(answer.status, 400, "{key}: {:.300}", answer.text);
        assert_eq!(
            answer.json(),
            json!({ "error": { "code": "kioskOwnerOrRates", "message": OWNER_OR_RATES } }),
            "{key}"
        );
    }
    room.finish().await;
}

/// V1: the view answers exactly whom `Policy::authorize(_, Kiosk, "manage", ..)` answers, for every
/// caller kind, and on the branch `Policy::branch` gives (the two branches' USD rates differ,
/// so the answer says which branch was read).
#[actix_web::test]
async fn the_view_answers_exactly_whom_the_kiosk_manage_row_allows() {
    let room = Room::open().await;
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        // OR-4b (#767): the route table's rule, naming B9 (D-4: RATES of MAIN is refused)
        let policy = blueprint_rule(actor.as_ref(), EntryPath::Kiosk, "manage", Some(B9));
        let answer = room.get(&who, &path("branch=B9")).await;
        let what = format!("as {who:?}");
        match &policy {
            Ok(grant) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                let branch = &grant.branch;
                let usd = currency(&body["data"], "USD");
                let expected = if branch.as_str() == B9 {
                    "31.50000000"
                } else {
                    "32.10000000"
                };
                assert_eq!(
                    usd["buy"],
                    json!(expected),
                    "{what}: the branch Policy::branch gives ({})",
                    branch.as_str()
                );
            }
            Err(e) => {
                refused += 1;
                assert_eq!(answer.status, e.status(), "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["error"]["message"], json!(e.to_string()), "{what}");
                assert!(body.get("data").is_none(), "{what}: no data");
            }
        }
    }
    // D-4: RATES of MAIN naming B9 is refused too
    assert_eq!(
        allowed, 10,
        "the owner, and the owner beside each of the nine roles"
    );
    assert_eq!(
        refused, 11,
        "nobody, the nine roles, the must-change employee"
    );
    room.finish().await;
}

/// A RATES employee naming another branch is answered for their own, with their own branch's
/// rates, switches, partner and KBank shop columns; their `since` is compared with their own
/// branch's version. The owner reads the branch named, `MAIN` when none is named.
#[actix_web::test]
async fn a_rates_employee_naming_another_branch_is_answered_for_their_own() {
    let room = Room::open().await;
    // JPY is one of the switches that are on until switched off; KRW and SGD are off until
    // switched on (`xc-kiosk` `DEFAULT_CODES`)
    room.switch(MAIN, "JPY", false).await;
    room.switch(MAIN, "KRW", true).await;
    room.switch(B9, "SGD", true).await;
    room.set_rate_with_partner(B9, "SGD", "25.9", "26.6", "SuperRich", Some("25.5"))
        .await;

    // OR-4b (D-4): RATES naming B9 is refused; naming MAIN they read MAIN's
    assert_out_of_scope(
        &room.get("RATES", &path("branch=B9")).await,
        "RATES naming B9",
    );
    let rates = room.data("RATES", "branch=MAIN").await;
    let usd = currency(&rates, "USD");
    assert_eq!(usd["buy"], json!("32.10000000"), "RATES works in MAIN");
    assert_eq!(usd["sell"], json!("33.00000000"));
    assert_eq!(
        usd["kbank"][1]["shopBuy"],
        json!("32.10000"),
        "MAIN's shop rate"
    );
    assert_eq!(
        usd["kbank"][1]["buyDifference"],
        json!("-0.48661"),
        "against MAIN's rate"
    );
    assert_eq!(
        currency(&rates, "JPY")["enabled"],
        json!(false),
        "MAIN's switch"
    );
    assert_eq!(
        currency(&rates, "KRW")["enabled"],
        json!(true),
        "MAIN's switch"
    );
    let sgd = currency(&rates, "SGD");
    assert_eq!(sgd["enabled"], json!(false), "B9's switch is not MAIN's");
    assert_eq!(sgd["buy"], Value::Null, "B9's rate is not MAIN's");
    assert_eq!(sgd["partnerName"], json!(""), "B9's partner is not MAIN's");
    assert_eq!(
        sgd["partnerCost"],
        Value::Null,
        "B9's partner is not MAIN's"
    );
    assert_eq!(sgd["updated"], json!(""), "B9's rate time is not MAIN's");
    assert_eq!(
        sgd["kbank"][0]["shopBuy"],
        Value::Null,
        "B9's shop rate is not MAIN's"
    );

    let owner_b9 = room.data("OWNER", "branch=B9").await;
    let usd = currency(&owner_b9, "USD");
    assert_eq!(
        usd["buy"],
        json!("31.50000000"),
        "the owner reads the branch named"
    );
    assert_eq!(usd["sell"], json!("32.75000000"));
    assert_eq!(
        usd["kbank"][1]["shopBuy"],
        json!("31.50000"),
        "B9's shop rate"
    );
    assert_eq!(
        usd["kbank"][1]["buyDifference"],
        json!("-1.08661"),
        "against B9's rate"
    );
    assert_eq!(
        currency(&owner_b9, "JPY")["enabled"],
        json!(true),
        "B9's default"
    );
    assert_eq!(currency(&owner_b9, "KRW")["enabled"], json!(false));
    let sgd = currency(&owner_b9, "SGD");
    assert_eq!(sgd["enabled"], json!(true));
    assert_eq!(sgd["buy"], json!("25.90000000"));
    assert_eq!(sgd["sell"], json!("26.60000000"));
    assert_eq!(sgd["partnerName"], json!("SuperRich"));
    assert_eq!(sgd["partnerCost"], json!("25.50000000"));
    assert_ne!(sgd["updated"], json!(""), "B9's rate has a time");
    assert_eq!(sgd["kbank"][0]["shopBuy"], json!("25.90000"));
    assert_eq!(sgd["kbank"][0]["buyDifference"], json!("0.18547"));

    for query in ["", "branch="] {
        let owner = room.data("OWNER", query).await;
        assert_eq!(
            currency(&owner, "USD")["buy"],
            json!("32.10000000"),
            "{query:?}: MAIN when no branch is named"
        );
    }

    // RATES's since is MAIN's version, whatever happens in B9
    // (#603 item 2: GBP is published in B9 first, so the owner may switch it on)
    room.set_rate(B9, "GBP", "43.1", "44.9").await;
    let v = room.version(MAIN).await;
    room.configure(
        "OWNER",
        json!({ "branchId": B9, "code": "GBP", "enabled": true, "buy": "43.1", "sell": "44.9",
                "previousUpdated": "", "partnerName": "", "partnerCost": null }),
    )
    .await;
    assert_eq!(
        room.version(MAIN).await,
        v,
        "a write in B9 leaves MAIN's version"
    );
    let body = room.view("RATES", &format!("branch=MAIN&since={v}")).await;
    is_unchanged(&body, v, "RATES's since is MAIN's version");
    room.finish().await;
}

/// §8: an ill-formed `branch` is the v3 refusal `invalidRequest`.
#[actix_web::test]
async fn an_ill_formed_branch_is_an_invalid_request() {
    let room = Room::open().await;
    for query in ["branch=no%20such", "branch=a%2Fb", "since=-1"] {
        let answer = room.get("OWNER", &path(query)).await;
        assert_eq!(answer.status, 400, "{query}: {:.300}", answer.text);
        assert_eq!(
            answer.json(),
            json!({ "error": { "code": "invalidRequest", "message": INVALID_REQUEST } }),
            "{query}"
        );
    }
    room.finish().await;
}

// ── only what changed (§4.2) ─────────────────────────────────────────────────

/// With `since` equal to the branch's version the view answers `changed: false` and is not
/// loaded: with every table it could read renamed away, `since` still answers, while the same
/// read without `since` fails (so it would have been loaded).
#[actix_web::test]
async fn an_equal_since_is_unchanged_and_the_view_is_not_loaded() {
    let room = Room::open().await;
    // #603 item 2: JPY is published (per 100) before it is switched on
    room.set_rate(MAIN, "JPY", "21.05", "22.05").await;
    room.configure(
        "OWNER",
        json!({ "code": "JPY", "enabled": true, "buy": "0.2105", "sell": "0.2205",
                "previousUpdated": "", "partnerName": "", "partnerCost": null }),
    )
    .await;
    let v = room.version(MAIN).await;
    assert!(v > 0, "configure moved MAIN's version");
    let body = room.view("OWNER", &format!("branch=MAIN&since={v}")).await;
    is_unchanged(&body, v, "owner");
    let body = room.view("RATES", &format!("since={v}")).await;
    is_unchanged(&body, v, "RATES");

    room.hide_the_views_tables().await;
    for _ in 0..3 {
        let body = room.view("OWNER", &format!("branch=MAIN&since={v}")).await;
        is_unchanged(&body, v, "with the view's tables gone");
    }
    let loaded = room.get("OWNER", &path("branch=MAIN")).await;
    assert_eq!(
        loaded.status, 500,
        "read with no since, it is loaded and finds no table: {:.300}",
        loaded.text
    );
    room.finish().await;
}

/// V4 (§9 writers 1–3): one write by each writer of what the view reads, then the same `since`
/// reports `changed: true`. Writer 1: `configure` on `/api/kiosk`. Writer 2: `adminSaveRate` on
/// `/api/rpc` by the owner. Writer 3: `adminSaveRate` on `/api/staff/rpc` by RATES.
///
/// Re-expected for RE-7 #604 by the ticket author: writers 2 and 3 are retired (design §9); each
/// is now held to the unknown-name answer and to a version that does not move.
///
/// Amended for #603: `configure` writes no rate (item 1) and the view reads the publication, not
/// `rates` (item 3), so after each write the view still answers the published USD rate; the
/// version still moves. The `adminSaveRate` writers send back `manage`'s `updated` (the newest
/// `rates` time, which that route compares), no longer the view's. A publish moving the version
/// is pinned in `v3_kiosk_currencies_publication_test.rs`.
#[actix_web::test]
async fn every_writer_of_the_views_tables_moves_the_version_it_answers() {
    let room = Room::open().await;
    let published = |usd: &Value, what: &str| {
        assert_eq!(usd["buy"], json!("32.10000000"), "{what}: {usd}");
        assert_eq!(usd["sell"], json!("33.00000000"), "{what}: {usd}");
    };
    let manage_updated = |manage: &Value| -> Value {
        manage["currencies"]
            .as_array()
            .expect("manage's currencies")
            .iter()
            .find(|c| c["code"] == json!("USD"))
            .map(|c| c["updated"].clone())
            .unwrap_or_else(|| json!(""))
    };

    // writer 1
    let v = room.version(MAIN).await;
    room.configure(
        "OWNER",
        json!({ "branchId": MAIN, "code": "USD", "enabled": true, "buy": "33.25",
                "sell": "33.5", "previousUpdated": "", "partnerName": "",
                "partnerCost": null }),
    )
    .await;
    let body = room.view("OWNER", &format!("branch=MAIN&since={v}")).await;
    let now = room.version(MAIN).await;
    assert!(now > v, "configure moved MAIN's version ({v} → {now})");
    assert_eq!(body["version"], json!(now), "configure: {body}");
    assert_eq!(body["changed"], json!(true), "configure: {body}");
    let usd = currency(&body["data"], "USD");
    published(usd, "configure writes no rate");
    assert_eq!(usd["enabled"], json!(true), "configure: {usd}");

    // writers 2 and 3, re-expected for RE-7 #604 by the ticket author: a known difference
    // (design §9 removes the action). `adminSaveRate` is no longer a writer: on either route it
    // is answered as a name v2 does not know, and MAIN's version does not move.
    for (who, route, buy, sell, query) in [
        ("OWNER", RPC, "33.3", "33.6", "branch=MAIN&"),
        ("RATES", STAFF_RPC, "33.35", "33.65", ""),
    ] {
        let previous = manage_updated(&room.manage(MAIN).await);
        let v = room.version(MAIN).await;
        let payload = json!({ "branchId": MAIN, "requestId": request_id(), "code": "USD",
                              "buy": buy, "sell": sell, "previousUpdated": previous,
                              "partnerName": "", "partnerCost": null });
        let save = room
            .post(who, route, "adminSaveRate", payload.clone())
            .await;
        let unknown = room
            .post(who, route, rpc_common::NO_SUCH_NAME, payload)
            .await;
        assert_eq!(
            (save.status, save.text.as_str()),
            (unknown.status, unknown.text.as_str()),
            "adminSaveRate on {route} as {who}: answered as a name v2 does not know"
        );
        assert_eq!(
            room.version(MAIN).await,
            v,
            "adminSaveRate on {route} moved nothing"
        );
        let body = room.view(who, &format!("{query}since={v}")).await;
        assert_eq!(
            body["changed"],
            json!(false),
            "adminSaveRate on {route}: {body}"
        );
    }
    room.finish().await;
}

// ── the currencies (§7.3) ────────────────────────────────────────────────────

/// The shape of `data`: the 152 currencies of the table in the table's order (no THB), each
/// with exactly the ticket's keys, its code, names and countries as the table holds them.
#[actix_web::test]
async fn the_view_answers_the_tables_currencies_in_order_with_the_tickets_keys() {
    let room = Room::open().await;
    let data = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(keys(&data), set_of(&DATA_KEYS), "the keys of data");
    let table = currency_table();
    assert_eq!(table.len(), 152);
    let currencies = data["currencies"]
        .as_array()
        .expect("currencies is an array");
    let codes: Vec<&Value> = currencies.iter().map(|c| &c["code"]).collect();
    let table_codes: Vec<&Value> = table.iter().map(|c| &c["code"]).collect();
    assert_eq!(codes, table_codes, "the table's order, AED first, no THB");
    for (c, entry) in currencies.iter().zip(&table) {
        let code = &entry["code"];
        assert_eq!(keys(c), set_of(&CURRENCY_KEYS), "{code}: the keys");
        assert_eq!(c["name"], entry["name"], "{code}: name");
        assert_eq!(c["nameEn"], entry["nameEn"], "{code}: nameEn");
        assert_eq!(c["countries"], entry["countries"], "{code}: countries");
        for country in c["countries"].as_array().expect("countries is an array") {
            assert_eq!(keys(country), set_of(&["code", "en", "th"]), "{code}");
        }
    }
    room.finish().await;
}

/// §9: a currency with no rate row has `buy`, `sell`, `partnerCost` `null`, `partnerName` and
/// `updated` `""`, and is off unless switched on.
#[actix_web::test]
async fn a_currency_with_no_rate_row_answers_nulls_and_an_empty_updated() {
    let room = Room::open().await;
    let data = room.data("OWNER", "branch=MAIN").await;
    let afn = currency(&data, "AFN");
    assert_eq!(afn["buy"], Value::Null, "{afn}");
    assert_eq!(afn["sell"], Value::Null, "{afn}");
    assert_eq!(afn["partnerCost"], Value::Null, "{afn}");
    assert_eq!(afn["partnerName"], json!(""), "{afn}");
    assert_eq!(afn["updated"], json!(""), "{afn}");
    assert_eq!(afn["enabled"], json!(false), "{afn}");

    room.switch(MAIN, "AFN", true).await;
    let data = room.data("OWNER", "branch=MAIN").await;
    let afn = currency(&data, "AFN");
    assert_eq!(afn["enabled"], json!(true), "switched on: {afn}");
    assert_eq!(afn["buy"], Value::Null, "switched on, still no rate: {afn}");
    assert_eq!(afn["updated"], json!(""), "{afn}");
    room.finish().await;
}

/// W7, §9: rates are sent as the strings the rate holds, never JSON numbers; the partner's
/// cost too. Amended for #603 item 3: the strings the publication holds (8 places, JPY per 100),
/// with each currency's `unitSize`, and `updated` the publication's `published_at`.
#[actix_web::test]
async fn rates_are_the_strings_the_rate_holds_never_numbers() {
    let room = Room::open().await;
    room.set_rate(MAIN, "USD", "33.25", "33.5").await;
    room.set_rate(MAIN, "JPY", "21.05", "22.05").await;
    room.set_rate_with_partner(MAIN, "EUR", "37", "38.1", "SuperRich", Some("36.75"))
        .await;
    let data = room.data("OWNER", "branch=MAIN").await;
    let usd = currency(&data, "USD");
    assert_eq!(usd["buy"], json!("33.25000000"), "{usd}");
    assert_eq!(usd["sell"], json!("33.50000000"), "{usd}");
    assert_eq!(usd["unitSize"], json!("1"), "{usd}");
    let jpy = currency(&data, "JPY");
    assert_eq!(jpy["buy"], json!("21.05000000"), "{jpy}");
    assert_eq!(jpy["sell"], json!("22.05000000"), "{jpy}");
    assert_eq!(jpy["unitSize"], json!("100"), "{jpy}");
    let eur = currency(&data, "EUR");
    assert_eq!(eur["buy"], json!("37.00000000"), "{eur}");
    assert_eq!(eur["partnerName"], json!("SuperRich"), "{eur}");
    assert_eq!(eur["partnerCost"], json!("36.75000000"), "{eur}");
    for c in data["currencies"].as_array().expect("currencies") {
        for key in ["buy", "sell", "partnerCost", "unitSize"] {
            assert!(
                c[key].is_string() || c[key].is_null(),
                "{}: {key} is a string or null, never a number: {}",
                c["code"],
                c[key]
            );
        }
    }
    room.finish().await;
}

/// §7.3, amended for #603 item 3 (`updated` is the publication's `published_at`, no longer
/// `manage`'s rate time) and item 1 (`configure` ignores `previousUpdated` and the rates it
/// carries): every currency of the current publication shows its `published_at` in today's
/// format, the others `""`; `configure` accepts any `previousUpdated` back and changes no rate.
#[actix_web::test]
async fn updated_is_the_publications_time_and_configure_ignores_it() {
    let room = Room::open().await;
    room.set_rate(MAIN, "JPY", "21.05", "22.05").await;
    room.configure(
        "OWNER",
        json!({ "code": "JPY", "enabled": true, "buy": "0.2105", "sell": "0.2205",
                "previousUpdated": "", "partnerName": "", "partnerCost": null }),
    )
    .await;
    let at = room.tick();
    room.set_rate(MAIN, "EUR", "37", "38.1").await;
    let published_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT p.published_at FROM branches b \
         JOIN rate_publications p ON p.id = b.current_publication_id WHERE b.id = 'MAIN'",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("MAIN's current publication");
    assert!(published_at > at, "EUR's publication is the current one");
    let want = published_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let data = room.data("OWNER", "branch=MAIN").await;
    let mut with_a_time = 0;
    for c in data["currencies"].as_array().expect("currencies") {
        let code = c["code"].as_str().expect("a code");
        if ["USD", "JPY", "EUR"].contains(&code) {
            assert_eq!(c["updated"], json!(want), "{code}: the publication's time");
            with_a_time += 1;
        } else {
            assert_eq!(c["updated"], json!(""), "{code}: no line");
        }
    }
    assert_eq!(with_a_time, 3, "USD, JPY and EUR are published");

    for previous in [json!(want), json!("2020-01-01T00:00:00.000Z"), json!("")] {
        room.configure(
            "OWNER",
            json!({ "branchId": MAIN, "code": "JPY", "enabled": false, "buy": "0.2110",
                    "sell": "0.2210", "previousUpdated": previous, "partnerName": "",
                    "partnerCost": null }),
        )
        .await;
    }
    let jpy = currency(&room.data("OWNER", "").await, "JPY").clone();
    assert_eq!(
        jpy["buy"],
        json!("21.05000000"),
        "configure changes no rate: {jpy}"
    );
    assert_eq!(jpy["enabled"], json!(false), "{jpy}");
    room.finish().await;
}

// ── KBank, computed on the server (§7.3, C11–C14) ────────────────────────────

/// The committed file is v2's `kbank-reference.json` byte for byte.
#[test]
fn the_kbank_file_is_v2s_byte_for_byte() {
    let bytes = std::fs::read(manifest(KBANK_FILE)).unwrap_or_else(|e| panic!("{KBANK_FILE}: {e}"));
    assert_eq!(bytes.len(), KBANK_BYTES, "{KBANK_FILE}: size");
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, KBANK_SHA256, "{KBANK_FILE}: sha256");
}

/// `kbank` of `data` is the file's source, publication time and round.
#[actix_web::test]
async fn the_kbank_reference_is_the_files_source_time_and_round() {
    let room = Room::open().await;
    let data = room.data("OWNER", "branch=MAIN").await;
    assert_eq!(
        data["kbank"],
        json!({
            "source": "https://www.kasikornbank.com/en/rate/Pages/Foreign-Exchange.aspx",
            "publishedAt": "2026-10-02T22:16:01+07:00",
            "round": 10,
        })
    );
    let file = kbank_file();
    assert_eq!(data["kbank"]["source"], file["source"]);
    assert_eq!(data["kbank"]["publishedAt"], file["publishedAt"]);
    assert_eq!(data["kbank"]["round"], file["round"]);
    room.finish().await;
}

/// For every currency with no shop rate: one row per KBank note of its code, in the file's
/// order (or one row labelled with the code when KBank has none); the bank's figures with
/// five places, `null` where absent; the shop's and the differences `null`.
#[actix_web::test]
async fn every_currency_carries_its_kbank_notes_in_the_files_order() {
    let room = Room::open().await;
    let data = room.data("OWNER", "branch=B9").await;
    let file = kbank_file();
    let notes = file["rates"].as_array().expect("the file's rates");
    for c in data["currencies"].as_array().expect("currencies") {
        let code = c["code"].as_str().expect("a code");
        if code == "USD" {
            continue; // B9 has a USD shop rate: see the USD test
        }
        let expected: Vec<Value> = {
            let mine: Vec<&Value> = notes.iter().filter(|n| n["code"] == json!(code)).collect();
            if mine.is_empty() {
                vec![row(code, [None; 6])]
            } else {
                mine.iter()
                    .map(|n| {
                        json!({
                            "label": n["label"],
                            "bankBuy": bank_text(&n["buy"]), "shopBuy": null,
                            "buyDifference": null,
                            "bankSell": bank_text(&n["sell"]), "shopSell": null,
                            "sellDifference": null,
                        })
                    })
                    .collect()
            }
        };
        assert_eq!(c["kbank"], Value::Array(expected), "{code}");
        for r in c["kbank"].as_array().expect("kbank is an array") {
            assert_eq!(keys(r), set_of(&COMPARISON_KEYS), "{code}: the keys");
        }
    }
    room.finish().await;
}

/// §9: USD shop buy 33.2500 against KBank `USD 5-20` buy 32.58661 is a buy difference of
/// `0.66339`; a shop rate below the bank's is a negative difference; every value has exactly
/// five places.
#[actix_web::test]
async fn usd_differences_are_shop_minus_bank_with_five_places() {
    let room = Room::open().await;
    room.set_rate(MAIN, "USD", "33.25", "33.5").await;
    let data = room.data("OWNER", "branch=MAIN").await;
    let usd = currency(&data, "USD");
    let s = Some;
    assert_eq!(
        usd["kbank"],
        json!([
            row(
                "USD 1",
                [
                    s("32.37919"),
                    s("33.25000"),
                    s("0.87081"),
                    s("33.81656"),
                    s("33.50000"),
                    s("-0.31656"),
                ],
            ),
            row(
                "USD 5-20",
                [
                    s("32.58661"),
                    s("33.25000"),
                    s("0.66339"),
                    s("33.81656"),
                    s("33.50000"),
                    s("-0.31656"),
                ],
            ),
            row(
                "USD 50-100",
                [
                    s("33.10516"),
                    s("33.25000"),
                    s("0.14484"),
                    s("33.85113"),
                    s("33.50000"),
                    s("-0.35113"),
                ],
            ),
        ]),
        "{usd}"
    );
    room.finish().await;
}

/// §9: where KBank has no buy (NOK), the bank buy and the buy difference are `null` while the
/// shop's buy shows; a bank figure of fewer places is padded (CAD `22.8087`); a shop rate that
/// is unset leaves the shop column and the difference `null`; a currency KBank has no note for
/// (AFN) is one row labelled with its code, every value `null`; a note with no figures (DKK)
/// is one row of `null`s under its own label.
#[actix_web::test]
async fn absent_rates_are_null_never_zero() {
    let room = Room::open().await;
    room.set_rate(MAIN, "NOK", "3.2", "3.6").await;
    let data = room.data("OWNER", "branch=MAIN").await;
    let s = Some;
    assert_eq!(
        currency(&data, "NOK")["kbank"],
        json!([row(
            "NOK",
            [
                None,
                s("3.20000"),
                None,
                s("3.56995"),
                s("3.60000"),
                s("0.03005")
            ]
        )]),
        "NOK"
    );
    assert_eq!(
        currency(&data, "CAD")["kbank"],
        json!([row(
            "CAD",
            [s("22.80870"), None, None, s("23.91709"), None, None]
        )]),
        "CAD"
    );
    assert_eq!(
        currency(&data, "AFN")["kbank"],
        json!([row("AFN", [None; 6])]),
        "AFN"
    );
    assert_eq!(
        currency(&data, "DKK")["kbank"],
        json!([row("DKK", [None; 6])]),
        "DKK"
    );
    room.finish().await;
}

// ── the generated types (§7.3, #393 §7) ──────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "xc-v3-kiosk-currencies-{}-{}",
            std::process::id(),
            next()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exported() -> String {
    let view = VIEWS.iter().find(|v| v.name == VIEW).unwrap_or_else(|| {
        panic!(
            "{VIEW} is in the generated view table: {:?}",
            VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
        )
    });
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    std::fs::read_to_string(scratch.0.join(format!("{VIEW}.ts")))
        .unwrap_or_else(|e| panic!("{VIEW} writes {VIEW}.ts: {e}"))
}

fn has_field(ts: &str, field: &str, ty: &str) {
    let wanted = format!("{field}: {ty}");
    assert!(ts.contains(&wanted), "{VIEW}.ts: `{wanted}` in:\n{ts}");
}

#[test]
fn the_view_is_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(names.contains(&VIEW), "{VIEW} in {names:?}");
}

/// The generated TypeScript carries the ticket's types and fields: rates as `Rate` (a decimal
/// string, `common.ts`), the KBank figures as strings, the round as a number, never `bigint`.
#[test]
fn the_generated_types_carry_the_tickets_fields() {
    let ts = exported();
    for name in [
        "KioskCurrencies",
        "KioskCurrency",
        "KioskCountry",
        "KbankReference",
        "KbankComparison",
    ] {
        assert!(
            ts.contains(&format!("export type {name} = ")),
            "{VIEW}.ts: type {name} in:\n{ts}"
        );
    }
    for (field, ty) in [
        ("currencies", "Array<KioskCurrency>"),
        ("kbank", "KbankReference | null"),
        ("code", "string"),
        ("name", "string"),
        ("nameEn", "string"),
        ("countries", "Array<KioskCountry>"),
        ("buy", "Rate | null"),
        ("sell", "Rate | null"),
        ("updated", "string"),
        ("partnerName", "string"),
        ("partnerCost", "Rate | null"),
        ("enabled", "boolean"),
        ("kbank", "Array<KbankComparison>"),
        ("en", "string"),
        ("th", "string"),
        ("source", "string"),
        ("publishedAt", "string"),
        ("round", "number"),
        ("label", "string"),
        ("bankBuy", "string | null"),
        ("shopBuy", "string | null"),
        ("buyDifference", "string | null"),
        ("bankSell", "string | null"),
        ("shopSell", "string | null"),
        ("sellDifference", "string | null"),
    ] {
        has_field(&ts, field, ty);
    }
    assert!(!ts.contains("bigint"), "{VIEW}.ts: never bigint:\n{ts}");
    assert!(!ts.contains("number | string"), "{VIEW}.ts:\n{ts}");
}

/// The file is committed under `web/src/shared/api/generated/` exactly as the DTOs generate it
/// (the drift gate of `v3_types_test.rs` covers it once it is there).
#[test]
fn the_generated_file_is_committed() {
    let file = manifest(&format!("../web/src/shared/api/generated/{VIEW}.ts"));
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{VIEW}.ts: {e}"));
    assert_eq!(committed, exported(), "{VIEW}.ts is what the DTOs generate");
}
