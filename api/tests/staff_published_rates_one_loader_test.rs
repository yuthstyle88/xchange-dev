//! The `staff_published_rates` view reads the branch's published rates through the **one** loader, not a copy of it
//! (fleet rule: several variants of one job get one shared core). The `published_rates` view (`(Rpc, "adminSnapshot")`)
//! and `staff_published_rates` (`(StaffRpc, "adminSnapshot")`) answer the same `PublishedRatesData`; they differ only
//! in the gate and in whose branch they read. The loader is `published_rates_data(pool, branch)` in
//! `views/published_rates.rs`.
//!
//! Written before the implementation by the ticket author (claude/5edf7bb5). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! - The `the_staff_view_*` tests pin what the staff view answers on `main` today (fields, 8-place rates, the
//!   normalised unit size, the order by currency, the folded `stale` and `sourceDate`, the empty answer). They pass
//!   before and after the refactor: it is a pure code move.
//! - `the_staff_view_reads_through_the_one_loader` is the guard. It fails while `staff_published_rates.rs` carries its
//!   own SQL and mapping, and passes once it calls `published_rates_data`.
//!
//! Publications are seeded by SQL with a crafted `source_snapshot`, as in `rate_freshness_test.rs`, so the cases do
//! not depend on the draft flow. `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets
//! its own schema.

mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

const MAIN: &str = "MAIN";

struct Line {
    currency: &'static str,
    group: &'static str,
    unit_size: &'static str,
    buy: Option<&'static str>,
    sell: Option<&'static str>,
    mode: &'static str,
    fetched_hours_ago: Option<i64>,
    source_date: Option<&'static str>,
}

/// One publication on MAIN holding `lines`, inserted in the given order, made current. Gives back the publication
/// id and its `published_at`.
async fn publish(pool: &PgPool, lines: &[Line]) -> (String, DateTime<Utc>) {
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'one-loader-seed', 'h', 'p', '{}'::jsonb, 'test', now(), \
         'APPROVED', gen_random_uuid()::text) RETURNING id::text",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("an approved request");
    let (publication, published_at): (String, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, \
                 (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1), \
                 $2::uuid, 'test') RETURNING id::text, published_at",
    )
    .bind(MAIN)
    .bind(&request)
    .fetch_one(pool)
    .await
    .expect("a publication");
    for l in lines {
        let mut snapshot =
            json!({ "buy_mode": l.mode, "sell_mode": l.mode, "max_reference_age_hours": 24 });
        if let Some(h) = l.fetched_hours_ago {
            snapshot["fetched_at"] = json!((Utc::now() - Duration::hours(h)).to_rfc3339());
        }
        if let Some(d) = l.source_date {
            snapshot["source_date"] = json!(d);
        }
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot) \
             VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4::numeric, $5::numeric, $6::numeric, $7)",
        )
        .bind(&publication)
        .bind(l.currency)
        .bind(l.group)
        .bind(l.unit_size)
        .bind(l.buy)
        .bind(l.sell)
        .bind(&snapshot)
        .execute(pool)
        .await
        .expect("a line");
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("make it current");
    (publication, published_at)
}

async fn line_id(pool: &PgPool, publication: &str, currency: &str) -> String {
    sqlx::query_scalar(
        "SELECT id::text FROM published_rate_lines WHERE publication_id = $1::uuid AND currency_code = $2",
    )
    .bind(publication)
    .bind(currency)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the {currency} line: {e}"))
}

/// The owner and a FRONT employee of MAIN, both signed in.
async fn setup() -> (Desk, TestApp, String, String) {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    let (_, staff) = working_employee(&app, &desk, &owner, "front1", "FRONT", MAIN).await;
    (desk, app, owner, staff)
}

/// `GET /api/v3/views/<name>`, `data`.
async fn view(app: &TestApp, name: &str, call: impl FnOnce(Call) -> Call) -> Value {
    let answer = send(
        app,
        call(Call {
            method: Some(Method::GET),
            path: format!("/api/v3/views/{name}"),
            proto: Some("https".to_owned()),
            ..Call::default()
        }),
    )
    .await;
    assert_eq!(answer.status, 200, "{name}: {}", answer.text);
    answer.json()["data"].clone()
}

async fn staff_view(app: &TestApp, staff: &str) -> Value {
    view(app, "staff_published_rates", |c| c.staff(staff)).await
}

async fn owner_view(app: &TestApp, owner: &str) -> Value {
    view(app, "published_rates", |c| c.owner(owner)).await
}

/// Four lines inserted out of order; one without a sell side, one without a buy side, one per 100 units, one in a
/// denomination group other than ALL. The oldest reference is JPY's.
fn four_lines() -> Vec<Line> {
    vec![
        Line {
            currency: "USD",
            group: "ALL",
            unit_size: "1",
            buy: Some("32.1"),
            sell: Some("32.5"),
            mode: "AUTO",
            fetched_hours_ago: Some(1),
            source_date: Some("2026-10-09"),
        },
        Line {
            currency: "JPY",
            group: "ALL",
            unit_size: "100",
            buy: Some("21.5"),
            sell: Some("22.75"),
            mode: "AUTO",
            fetched_hours_ago: Some(2),
            source_date: Some("2026-10-07"),
        },
        Line {
            currency: "EUR",
            group: "ALL",
            unit_size: "1",
            buy: Some("36.25"),
            sell: None,
            mode: "MANUAL",
            fetched_hours_ago: None,
            source_date: None,
        },
        Line {
            currency: "GBP",
            group: "50-100",
            unit_size: "1",
            buy: None,
            sell: Some("43.125"),
            mode: "MANUAL",
            fetched_hours_ago: None,
            source_date: None,
        },
    ]
}

/// The whole answer for a seeded publication, key for key: ordered by currency, rates as 8-place strings or null,
/// the unit size normalised, every line's id, `stale` and `sourceDate` folded over the lines.
#[actix_web::test]
async fn the_staff_view_answers_exactly_the_seeded_publication() {
    let (desk, app, _owner, staff) = setup().await;
    let (publication, published_at) = publish(&desk.pool, &four_lines()).await;
    let eur = line_id(&desk.pool, &publication, "EUR").await;
    let gbp = line_id(&desk.pool, &publication, "GBP").await;
    let jpy = line_id(&desk.pool, &publication, "JPY").await;
    let usd = line_id(&desk.pool, &publication, "USD").await;

    let expected = json!({
        "publication": { "id": publication, "version": 1, "publishedAt": published_at.to_rfc3339() },
        "sourceDate": "2026-10-07",
        "stale": false,
        "bandHeadings": [],
        "lines": [
            { "currency": "EUR", "group": "ALL", "unitSize": "1", "buyRate": "36.25000000", "sellRate": null, "lineId": eur, "bands": [] },
            { "currency": "GBP", "group": "50-100", "unitSize": "1", "buyRate": null, "sellRate": "43.12500000", "lineId": gbp, "bands": [] },
            { "currency": "JPY", "group": "ALL", "unitSize": "100", "buyRate": "21.50000000", "sellRate": "22.75000000", "lineId": jpy, "bands": [] },
            { "currency": "USD", "group": "ALL", "unitSize": "1", "buyRate": "32.10000000", "sellRate": "32.50000000", "lineId": usd, "bands": [] },
        ],
    });
    assert_eq!(staff_view(&app, &staff).await, expected);
    drop(app);
    desk.drop_schema().await;
}

/// One line whose reference is past its 24 h limit makes the publication stale, with that line's date.
#[actix_web::test]
async fn the_staff_view_is_stale_when_a_line_carries_an_old_reference() {
    let (desk, app, _owner, staff) = setup().await;
    let mut lines = four_lines();
    lines.push(Line {
        currency: "CHF",
        group: "ALL",
        unit_size: "1",
        buy: Some("38"),
        sell: Some("39"),
        mode: "AUTO",
        fetched_hours_ago: Some(30),
        source_date: Some("2026-10-05"),
    });
    publish(&desk.pool, &lines).await;

    let data = staff_view(&app, &staff).await;
    assert_eq!(data["stale"], json!(true), "{data}");
    assert_eq!(data["sourceDate"], json!("2026-10-05"), "{data}");
    let order: Vec<&str> = data["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .map(|l| l["currency"].as_str().expect("a currency"))
        .collect();
    assert_eq!(order, ["CHF", "EUR", "GBP", "JPY", "USD"], "{data}");
    drop(app);
    desk.drop_schema().await;
}

/// Before the branch's first publication: no publication, no lines, not stale, no date.
#[actix_web::test]
async fn the_staff_view_before_the_first_publication_is_empty() {
    let (desk, app, _owner, staff) = setup().await;
    assert_eq!(
        staff_view(&app, &staff).await,
        json!({ "publication": null, "sourceDate": null, "stale": false, "lines": [], "bandHeadings": [] })
    );
    drop(app);
    desk.drop_schema().await;
}

/// The two views read the same branch the same way: for MAIN, the staff answer is the owner's answer.
#[actix_web::test]
async fn the_staff_view_and_the_owner_view_answer_the_same_for_one_branch() {
    let (desk, app, owner, staff) = setup().await;
    assert_eq!(
        staff_view(&app, &staff).await,
        owner_view(&app, &owner).await,
        "before a publication"
    );
    let mut lines = four_lines();
    lines[0].fetched_hours_ago = Some(30);
    publish(&desk.pool, &lines).await;
    let staff_answer = staff_view(&app, &staff).await;
    assert_eq!(staff_answer["stale"], json!(true), "{staff_answer}");
    assert_eq!(staff_answer, owner_view(&app, &owner).await);
    drop(app);
    desk.drop_schema().await;
}

// ── one loader, in the source ─────────────────────────────────────────────

fn source(path: &str) -> String {
    let full = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"))
}

/// The staff view keeps its own gate (`NAME`, `INHERITS`) and nothing else: no SQL, no table, no freshness fold, no
/// line mapping. It calls the loader the `published_rates` view and the public rates route call.
#[test]
fn the_staff_view_reads_through_the_one_loader() {
    let path = "src/presentation/http/v3/views/staff_published_rates.rs";
    let text = source(path);
    for copy in [
        "sqlx::query",
        "published_rate_lines",
        "rate_publications",
        "current_publication_id",
        "Freshness::",
        "PublishedLineDto {",
        "format!(\"{:.8}\"",
    ] {
        assert!(
            !text.contains(copy),
            "{path} still carries its own copy of the published-rates loader ({copy}); call published_rates_data"
        );
    }
    assert!(
        text.contains("published_rates_data("),
        "{path} reads through published_rates_data(pool, branch)"
    );
    assert!(
        text.contains("const NAME: &'static str = \"staff_published_rates\";")
            && text.contains("(EntryPath::StaffRpc, \"adminSnapshot\")"),
        "{path} keeps its own name and gate"
    );
    assert!(
        source("src/presentation/http/v3/views/published_rates.rs")
            .contains("pub async fn published_rates_data("),
        "the one loader lives in views/published_rates.rs"
    );
}
