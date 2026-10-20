//! Ticket #995 (APP-5) — the public reads of the customer app:
//! `GET /api/public/orgs/{org}/branches` and `GET /api/public/orgs/{org}/branches/{code}/rates`.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The lane controller's rulings on #995 (claude/5edf7bb5, 2026-10-10) pinned here:
//! 1. the branch list is a public route, not a v3 view;
//! 2. the organisation code is in the path, and an unknown organisation, an unknown branch and a
//!    branch code of another organisation answer the same 404 body;
//! 3. the list's ETag changes on every branch-version bump, even one whose `changed_at` is older
//!    than the newest (`branch_tx.rs:86` writes `now()`, the transaction's start), and when a
//!    branch leaves or joins the list; another organisation's change leaves it as it was;
//! 4. no policy row: both routes answer without credentials and ignore bad ones.
//!
//! The rates body is the `published_rates` view's DTO, `PublishedRatesData` (`rates.md:454`), read
//! through `published_rates_data`, the one reader the view and the route share.
//!
//! Needs `TEST_DATABASE_URL` for every test but the TS one; nothing here skips.

mod common;
mod quote_common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::{Line, TestDb};
use serde_json::Value;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::public_branches;
use xchange::presentation::http::v3::published_rates::published_rates_data;

/// The seeded organisation of every migrated database (`20261011100000_organizations.sql`).
const XC: &str = "XC";
/// A second organisation, made by `other_organisation`.
const YY: &str = "YY";
const YY_ID: &str = "00000000-0000-0000-0000-0000000000a2";

struct Answer {
    status: StatusCode,
    etag: Option<String>,
    cache_control: Option<String>,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!("a JSON body ({e}): {}", String::from_utf8_lossy(&self.body))
        })
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// One GET through the whole route table, with `headers`.
async fn get(db: &TestDb, uri: &str, headers: &[(&str, &str)]) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let mut req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"));
    for (name, value) in headers {
        req = req.insert_header((*name, *value));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let value = |name: header::HeaderName| {
        resp.headers()
            .get(name)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
    };
    let etag = value(header::ETAG);
    let cache_control = value(header::CACHE_CONTROL);
    let body = atest::read_body(resp).await.to_vec();
    Answer {
        status,
        etag,
        cache_control,
        body,
    }
}

async fn list(db: &TestDb, org: &str) -> Answer {
    get(db, &format!("/api/public/orgs/{org}/branches"), &[]).await
}

async fn list_since(db: &TestDb, org: &str, etag: &str) -> Answer {
    get(
        db,
        &format!("/api/public/orgs/{org}/branches"),
        &[("if-none-match", etag)],
    )
    .await
}

async fn rates(db: &TestDb, org: &str, code: &str) -> Answer {
    get(
        db,
        &format!("/api/public/orgs/{org}/branches/{code}/rates"),
        &[],
    )
    .await
}

/// The codes the list answers, in its order; panics unless it is 200.
fn codes(answer: &Answer) -> Vec<String> {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    answer.json()["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("a branches list in {}", answer.text()))
        .iter()
        .map(|b| b["code"].as_str().expect("a code").to_owned())
        .collect()
}

fn etag_of(answer: &Answer) -> String {
    answer
        .etag
        .clone()
        .unwrap_or_else(|| panic!("an ETag on {} {}", answer.status, answer.text()))
}

/// A branch of the seeded organisation `XC`, with its public profile.
async fn xc_branch(db: &TestDb, id: &str, code: &str) {
    db.exec(&format!(
        "INSERT INTO branches (id, name, code) VALUES ('{id}', 'Branch {code}', '{code}')"
    ))
    .await;
}

/// The organisation `YY` with one branch `id`, whose code is `code`.
async fn other_organisation(db: &TestDb, id: &str, code: &str) {
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{YY_ID}', '{YY}', NULL, 'Asia/Bangkok', now())"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id, code) \
         VALUES ('{id}', 'Other {code}', '{YY_ID}', '{code}')"
    ))
    .await;
}

/// Every object key anywhere in `v`, lower-cased.
fn keys(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.insert(k.to_lowercase());
                keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| keys(i, out)),
        _ => {}
    }
}

/// Keys no public body may carry (design §10 *"ไม่คืน reference cost/PII"*).
fn forbidden_keys(v: &Value) -> Vec<String> {
    let mut all = BTreeSet::new();
    keys(v, &mut all);
    all.into_iter()
        .filter(|k| {
            [
                "reference",
                "cost",
                "margin",
                "partner",
                "customer",
                "snapshot",
                "policy",
                "phone",
                "email",
                "publishedby",
                "approved",
            ]
            .iter()
            .any(|bad| k.contains(bad))
        })
        .collect()
}

fn key_set(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("an object: {v}"))
        .keys()
        .cloned()
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

// ---------------------------------------------------------------- the branch list

#[tokio::test]
async fn the_list_answers_the_organisations_branches_by_code_with_the_public_keys_only() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-zulu", "ZULU").await;
    xc_branch(&db, "b-alpha", "ALPHA").await;
    db.exec(
        "UPDATE branches SET address = '1 Silom Rd', booking_enabled = true, \
             timezone = 'Asia/Bangkok', \
             opening_hours = '{\"mon\": [{\"open\": \"09:00\", \"close\": \"17:00\"}]}'::jsonb \
         WHERE id = 'b-alpha'",
    )
    .await;
    db.exec(
        "INSERT INTO branch_service_status (branch_id, organization_id, queue_open) \
         SELECT id, organization_id, true FROM branches WHERE id = 'b-alpha'",
    )
    .await;
    other_organisation(&db, "y-only", "YONLY").await;

    let answer = list(&db, XC).await;
    assert_eq!(
        codes(&answer),
        ["ALPHA", "MAIN", "ZULU"],
        "XC's branches, sorted by code; YY's branch is not listed"
    );
    assert_eq!(
        answer.cache_control.as_deref(),
        Some("no-cache"),
        "no-cache, so the browser revalidates against the ETag (no-store would stop that)"
    );
    etag_of(&answer);

    let body = answer.json();
    assert_eq!(key_set(&body), set(&["branches"]));
    let wanted = set(&[
        "code",
        "name",
        "address",
        "openingHours",
        "timezone",
        "bookingEnabled",
        "queueOpen",
    ]);
    for b in body["branches"].as_array().expect("a list") {
        assert_eq!(key_set(b), wanted, "{b}");
    }
    assert_eq!(forbidden_keys(&body), Vec::<String>::new());

    let alpha = &body["branches"][0];
    assert_eq!(alpha["name"], "Branch ALPHA");
    assert_eq!(alpha["address"], "1 Silom Rd");
    assert_eq!(alpha["timezone"], "Asia/Bangkok");
    assert_eq!(alpha["bookingEnabled"], true);
    assert_eq!(alpha["queueOpen"], true);
    assert_eq!(alpha["openingHours"]["mon"][0]["open"], "09:00");
    let zulu = &body["branches"][2];
    assert_eq!(zulu["address"], Value::Null);
    assert_eq!(zulu["openingHours"], Value::Null);
    assert_eq!(zulu["bookingEnabled"], false);
    assert_eq!(
        zulu["queueOpen"], false,
        "no branch_service_status row reads as a closed queue"
    );

    let other = list(&db, YY).await;
    assert_eq!(codes(&other), ["YONLY"], "YY lists only its own branch");
    db.finish().await;
}

#[tokio::test]
async fn unchanged_answers_304_with_the_same_etag_and_no_body() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-alpha", "ALPHA").await;
    let first = list(&db, XC).await;
    let etag = etag_of(&first);

    let again = list_since(&db, XC, &etag).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED, "{}", again.text());
    assert!(
        again.body.is_empty(),
        "a 304 has no body: {:?}",
        again.text()
    );
    assert_eq!(again.etag.as_deref(), Some(etag.as_str()));

    let stale = list_since(&db, XC, "\"not-the-current-version\"").await;
    assert_eq!(
        codes(&stale),
        ["ALPHA", "MAIN"],
        "another ETag gets the full list"
    );
    assert_eq!(
        etag_of(&stale),
        etag,
        "nothing changed, so the ETag is the same"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_branch_that_leaves_the_list_changes_the_etag_and_the_list_shrinks() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-alpha", "ALPHA").await;
    xc_branch(&db, "b-zulu", "ZULU").await;
    let before = list(&db, XC).await;
    assert_eq!(codes(&before), ["ALPHA", "MAIN", "ZULU"]);
    let etag = etag_of(&before);

    // not the newest branch, and one with no version row: only the count can tell
    db.exec("DELETE FROM branches WHERE id = 'b-alpha'").await;
    let after = list_since(&db, XC, &etag).await;
    assert_eq!(
        codes(&after),
        ["MAIN", "ZULU"],
        "the client's ETag is old, so it gets the shorter list"
    );
    assert_ne!(etag_of(&after), etag);
    db.finish().await;
}

#[tokio::test]
async fn one_branch_out_and_one_in_still_changes_the_etag() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-alpha", "ALPHA").await;
    xc_branch(&db, "b-zulu", "ZULU").await;
    let etag = etag_of(&list(&db, XC).await);

    // the count and every version are as they were; only the set of branches differs
    db.exec("DELETE FROM branches WHERE id = 'b-zulu'").await;
    xc_branch(&db, "b-yank", "YANK").await;
    let after = list_since(&db, XC, &etag).await;
    assert_eq!(codes(&after), ["ALPHA", "MAIN", "YANK"]);
    assert_ne!(etag_of(&after), etag);
    db.finish().await;
}

#[tokio::test]
async fn a_late_commit_whose_changed_at_is_older_than_the_newest_still_changes_the_etag() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-alpha", "ALPHA").await;
    db.exec(
        "INSERT INTO branch_versions (branch_id, version, changed_at) VALUES \
             ('MAIN', 4, now()), ('b-alpha', 2, now() - interval '1 hour') \
         ON CONFLICT (branch_id) DO UPDATE SET version = EXCLUDED.version, \
             changed_at = EXCLUDED.changed_at",
    )
    .await;
    let etag = etag_of(&list(&db, XC).await);

    // what `BranchTx` leaves when a transaction that started two hours ago commits now: the
    // version goes up, but `changed_at` (its `now()`) is below the newest one already served
    db.exec(
        "UPDATE branch_versions SET version = version + 1, \
             changed_at = now() - interval '2 hours' \
         WHERE branch_id = 'b-alpha'",
    )
    .await;
    let after = list_since(&db, XC, &etag).await;
    assert_eq!(
        after.status,
        StatusCode::OK,
        "a bump below max(changed_at) is still a change (ruling 3): {}",
        after.text()
    );
    assert_ne!(etag_of(&after), etag);
    db.finish().await;
}

#[tokio::test]
async fn another_organisations_change_leaves_this_etag_as_it_was() {
    let db = TestDb::start().await;
    other_organisation(&db, "y-one", "YONE").await;
    let etag = etag_of(&list(&db, XC).await);

    db.exec(
        "INSERT INTO branch_versions (branch_id, version, changed_at) VALUES ('y-one', 9, now()) \
         ON CONFLICT (branch_id) DO UPDATE SET version = 9, changed_at = now()",
    )
    .await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id, code) \
         VALUES ('y-two', 'Other two', '{YY_ID}', 'YTWO')"
    ))
    .await;
    let after = list_since(&db, XC, &etag).await;
    assert_eq!(
        after.status,
        StatusCode::NOT_MODIFIED,
        "YY's changes are not XC's: {}",
        after.text()
    );
    db.finish().await;
}

// ---------------------------------------------------------------- the rates route

#[tokio::test]
async fn the_rates_route_answers_the_published_rates_shape_and_nothing_else() {
    let db = TestDb::start().await;
    let published = db
        .publish(
            "MAIN",
            &[
                Line::manual("USD", "1", "35.10", "35.50"),
                Line::manual("JPY", "100", "23.40", "24.10"),
            ],
        )
        .await;

    let answer = rates(&db, XC, "MAIN").await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
    let body = answer.json();
    assert_eq!(
        key_set(&body),
        set(&[
            "publication",
            "sourceDate",
            "stale",
            "lines",
            "bandHeadings"
        ])
    );
    assert_eq!(
        key_set(&body["publication"]),
        set(&["id", "version", "publishedAt"])
    );
    assert_eq!(body["publication"]["id"], published.id.as_str());
    assert_eq!(body["publication"]["version"], published.version);
    assert_eq!(body["stale"], false, "MANUAL lines are never stale");
    let lines = body["lines"].as_array().expect("lines");
    let currencies: Vec<&str> = lines
        .iter()
        .map(|l| l["currency"].as_str().expect("a currency"))
        .collect();
    assert_eq!(currencies, ["JPY", "USD"]);
    for l in lines {
        assert_eq!(
            key_set(l),
            set(&[
                "currency", "group", "unitSize", "buyRate", "sellRate", "lineId", "bands"
            ]),
            "{l}"
        );
    }
    assert_eq!(lines[0]["unitSize"], "100");
    assert_eq!(lines[0]["lineId"], published.line("JPY"));
    assert_eq!(lines[1]["buyRate"], "35.10000000");
    assert_eq!(lines[1]["sellRate"], "35.50000000");
    assert_eq!(forbidden_keys(&body), Vec::<String>::new());

    // one reader: the route answers exactly what the shared function reads for the branch
    let shared = published_rates_data(&db.pool, &BranchId::new("MAIN"))
        .await
        .expect("the shared reader");
    assert_eq!(
        body,
        serde_json::to_value(&shared).expect("serialise"),
        "the route's body is published_rates_data's"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_stale_reference_is_said_with_its_source_date() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    db.publish(
        "MAIN",
        &[Line {
            code: "USD",
            unit_size: "1",
            buy: Some("35.10"),
            sell: Some("35.50"),
            stale_under: Some(policy),
        }],
    )
    .await;

    let body = rates(&db, XC, "MAIN").await.json();
    assert_eq!(
        body["stale"], true,
        "fetched 30 h ago under a 24 h limit: {body}"
    );
    assert!(
        body["sourceDate"].as_str().is_some_and(|d| d.len() == 10),
        "a YYYY-MM-DD source date: {body}"
    );
    assert_eq!(forbidden_keys(&body), Vec::<String>::new());
    db.finish().await;
}

#[tokio::test]
async fn a_branch_with_no_publication_yet_answers_an_empty_board() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-alpha", "ALPHA").await;
    let answer = rates(&db, XC, "ALPHA").await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(
        answer.json(),
        serde_json::json!({ "publication": null, "sourceDate": null, "stale": false, "lines": [], "bandHeadings": [] })
    );
    db.finish().await;
}

#[tokio::test]
async fn a_branch_code_is_read_within_the_organisation_in_the_path() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-same-xc", "SAME").await;
    other_organisation(&db, "b-same-yy", "SAME").await;
    let xc = db
        .publish("b-same-xc", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let yy = db
        .publish("b-same-yy", &[Line::manual("USD", "1", "36.00", "36.40")])
        .await;

    let xc_body = rates(&db, XC, "SAME").await.json();
    let yy_body = rates(&db, YY, "SAME").await.json();
    assert_eq!(xc_body["publication"]["id"], xc.id.as_str());
    assert_eq!(yy_body["publication"]["id"], yy.id.as_str());
    assert_eq!(yy_body["lines"][0]["buyRate"], "36.00000000");
    db.finish().await;
}

#[tokio::test]
async fn an_unknown_organisation_or_branch_is_one_and_the_same_404() {
    let db = TestDb::start().await;
    other_organisation(&db, "y-only", "YONLY").await;

    let cases = [
        ("an unknown organisation", rates(&db, "NOPE", "MAIN").await),
        ("an unknown branch", rates(&db, XC, "NOPE").await),
        (
            "another organisation's branch code",
            rates(&db, XC, "YONLY").await,
        ),
        (
            "the list of an unknown organisation",
            list(&db, "NOPE").await,
        ),
    ];
    let first = cases[0].1.body.clone();
    for (what, answer) in &cases {
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{what}: {}",
            answer.text()
        );
        assert_eq!(answer.json()["error"]["code"], "notFound", "{what}");
        assert_eq!(
            answer.body, first,
            "{what}: the same body, so a probe cannot tell which part was wrong"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn both_routes_answer_without_credentials_and_ignore_bad_ones() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let bad: [&[(&str, &str)]; 3] = [
        &[],
        &[("authorization", "Bearer not-a-token")],
        &[("cookie", "__Host-xc_session=not-a-session; xc_app_sid=nope")],
    ];
    for headers in bad {
        for uri in [
            "/api/public/orgs/XC/branches",
            "/api/public/orgs/XC/branches/MAIN/rates",
        ] {
            let answer = get(&db, uri, headers).await;
            assert_eq!(
                answer.status,
                StatusCode::OK,
                "{uri} with {headers:?}: {}",
                answer.text()
            );
        }
    }
    db.finish().await;
}

// ---------------------------------------------------------------- the committed TS

fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/shared/api/public")
}

/// `XC_UPDATE_GENERATED=1 cargo test --test app_public_reads_test` rewrites the committed file, as
/// `v3_types_test.rs` does for `generated/`; CI never sets it.
#[test]
fn the_committed_public_types_are_what_the_dto_generates() {
    let scratch = std::env::temp_dir().join(format!(
        "xc-public-types-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).expect("a scratch directory");
    public_branches::export_types(&scratch).expect("export the public types");
    let produced = std::fs::read_to_string(scratch.join("public_branches.ts"))
        .expect("public_branches.ts is written");
    let _ = std::fs::remove_dir_all(&scratch);

    if std::env::var("XC_UPDATE_GENERATED").as_deref() == Ok("1") {
        std::fs::create_dir_all(committed_dir()).expect("create web/src/shared/api/public");
        std::fs::write(committed_dir().join("public_branches.ts"), &produced)
            .expect("write the committed file");
    }
    let committed =
        std::fs::read_to_string(committed_dir().join("public_branches.ts")).unwrap_or_default();
    assert_eq!(
        committed, produced,
        "web/src/shared/api/public/public_branches.ts differs from what the DTO generates"
    );
    assert!(
        !produced
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == "any"),
        "no `any` type in the public types: {produced}"
    );
}
