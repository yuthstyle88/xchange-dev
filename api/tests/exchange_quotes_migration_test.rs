//! Ticket #669 (TX-1+TX-3), Part 1 (TX-1) — migration 1 (`api/migrations/20261014100000_quotes.sql`)
//! against a real PostgreSQL: the four tables, their CHECKs and keys, and the one
//! `exchange_settings` row per organisation.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §3 (every table carries `organization_id`,
//! `created_at`, `updated_at`; money `numeric(20,2)`, foreign quantities `numeric(20,4)`, rates
//! `numeric(20,8)`), §3.1 (`quotes`, `quote_lines`, every CHECK), §3.6 (`exchange_settings` with
//! `quote_ttl_seconds` 30–3600 DEFAULT 300; `branch_exchange_limits` with a NULL-able positive
//! limit), §8 item 1 (one `exchange_settings` row per organisation with 300; `branch_exchange_limits`
//! empty; no existing row touched), §9.1 TX-1 (*"`quote_ttl_seconds` outside 30–3600 is refused by
//! the CHECK"*, ticket §9).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own, so one test can stop the migrations just before
//! migration 1, add an organisation, and then apply the rest.
//!
//! Signatures this test assumes:
//! - The migration's version is `20261014100000` (§8 item 1, ticket file scope).
//! - Table and column names exactly as §3.1 and §3.6 (`quotes`, `quote_lines`, `exchange_settings`,
//!   `branch_exchange_limits`).
//! - `organization_id` in the four tables has the type of `organizations.id` (OR-1 made it `uuid`;
//!   §3.1 writes `text`, which cannot carry OR-1's foreign keys — see the open question in the
//!   ticket comment). Constraint names are not pinned: each rule is pinned by the row it refuses
//!   (SQLSTATE `23514` CHECK, `23503` foreign key, `23505` unique, `23502` NOT NULL).

mod quote_common;

use quote_common::*;

const CHECK: &str = "23514";
const FOREIGN_KEY: &str = "23503";
const UNIQUE: &str = "23505";
const NOT_NULL: &str = "23502";

/// (column, data_type, numeric precision and scale when numeric, nullable)
type Col = (&'static str, &'static str, Option<(i32, i32)>, bool);
/// A column as `information_schema.columns` describes it, in the shape of [`Col`].
type HaveCol = (String, String, Option<(i32, i32)>, bool);
/// (column_name, data_type, numeric_precision, numeric_scale, is_nullable)
type InfoRow = (String, String, Option<i32>, Option<i32>, String);

async fn columns(db: &TestDb, table: &str) -> Vec<HaveCol> {
    let rows: Vec<InfoRow> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, numeric_precision::int, numeric_scale::int, \
                is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("read the columns of {table}: {e}"));
    rows.into_iter()
        .map(|(name, ty, p, s, nullable)| {
            let ps = match (p, s) {
                (Some(p), Some(s)) if ty == "numeric" => Some((p, s)),
                _ => None,
            };
            (name, ty, ps, nullable == "YES")
        })
        .collect()
}

async fn assert_columns(db: &TestDb, table: &str, want: &[Col]) {
    let have = columns(db, table).await;
    assert!(!have.is_empty(), "{table} exists after migration 1");
    let org_type = db
        .text(
            "SELECT data_type::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'organizations' \
               AND column_name = 'id'",
        )
        .await
        .expect("organizations.id has a type");
    let org = have
        .iter()
        .find(|c| c.0 == "organization_id")
        .unwrap_or_else(|| panic!("{table}.organization_id (§3): {have:?}"));
    assert_eq!(
        org.1, org_type,
        "{table}.organization_id has the type of organizations.id"
    );
    assert!(!org.3, "{table}.organization_id is NOT NULL");
    for (name, ty, ps, nullable) in want {
        let col = have
            .iter()
            .find(|c| c.0 == *name)
            .unwrap_or_else(|| panic!("{table}.{name}: columns are {have:?}"));
        assert_eq!(col.1, *ty, "{table}.{name} type");
        if ps.is_some() {
            assert_eq!(col.2, *ps, "{table}.{name} precision and scale");
        }
        assert_eq!(col.3, *nullable, "{table}.{name} nullable");
    }
}

#[tokio::test]
async fn migration_1_creates_the_four_tables_with_the_designs_columns() {
    let db = TestDb::start().await;
    let num = |p, s| Some((p, s));
    assert_columns(
        &db,
        "quotes",
        &[
            ("id", "uuid", None, false),
            ("branch_id", "text", None, false),
            ("customer_id", "text", None, true),
            ("type", "text", None, false),
            ("channel", "text", None, false),
            ("status", "text", None, false),
            ("publication_id", "uuid", None, false),
            ("expires_at", "timestamp with time zone", None, false),
            ("benefit_snapshot", "jsonb", None, false),
            ("created_by", "text", None, false),
            ("accepted_by_transaction_id", "uuid", None, true),
            ("created_at", "timestamp with time zone", None, false),
            ("updated_at", "timestamp with time zone", None, false),
        ],
    )
    .await;
    assert_columns(
        &db,
        "quote_lines",
        &[
            ("id", "uuid", None, false),
            ("quote_id", "uuid", None, false),
            ("published_rate_line_id", "uuid", None, false),
            ("shop_side", "text", None, false),
            ("currency", "character", None, false),
            ("denomination_group", "text", None, false),
            ("quantity", "numeric", num(20, 4), false),
            ("unit_size", "numeric", num(20, 4), false),
            ("base_rate", "numeric", num(20, 8), false),
            ("final_rate", "numeric", num(20, 8), false),
            ("thb_amount", "numeric", num(20, 2), false),
            ("fee_thb", "numeric", num(20, 2), false),
            ("settle_thb", "numeric", num(20, 2), false),
            ("line_no", "integer", None, false),
            ("created_at", "timestamp with time zone", None, false),
            ("updated_at", "timestamp with time zone", None, false),
        ],
    )
    .await;
    assert_columns(
        &db,
        "exchange_settings",
        &[
            ("quote_ttl_seconds", "integer", None, false),
            ("updated_by", "text", None, false),
            ("updated_at", "timestamp with time zone", None, false),
        ],
    )
    .await;
    assert_columns(
        &db,
        "branch_exchange_limits",
        &[
            ("branch_id", "text", None, false),
            ("approval_limit_thb", "numeric", num(20, 2), true),
            ("updated_by", "text", None, false),
            ("updated_at", "timestamp with time zone", None, false),
        ],
    )
    .await;
    db.finish().await;
}

/// A publication with one JPY line on MAIN, and SQL for a quote and a line that every CHECK
/// admits, with one column replaced.
struct Fixture {
    publication: String,
    line: String,
}

impl Fixture {
    async fn new(db: &TestDb) -> Self {
        let p = db.publish("MAIN", &[JPY]).await;
        let line = p.line("JPY").to_owned();
        Self {
            publication: p.id,
            line,
        }
    }

    /// An INSERT of a quote; `over` replaces columns by name.
    fn quote(&self, id: &str, over: &[(&str, &str)]) -> String {
        let mut cols: Vec<(&str, String)> = vec![
            ("id", format!("'{id}'")),
            ("organization_id", format!("'{ORG_1}'")),
            ("branch_id", "'MAIN'".into()),
            ("customer_id", "NULL".into()),
            ("type", "'INDICATIVE'".into()),
            ("channel", "'DESK'".into()),
            ("status", "'ACTIVE'".into()),
            ("publication_id", format!("'{}'", self.publication)),
            ("expires_at", "now() + interval '300 seconds'".into()),
            ("created_by", "'employee-1'".into()),
            ("accepted_by_transaction_id", "NULL".into()),
            ("created_at", "now()".into()),
            ("updated_at", "now()".into()),
        ];
        replace(&mut cols, over);
        insert("quotes", &cols)
    }

    /// An INSERT of line `line_no` of quote `quote`; `over` replaces columns by name.
    fn line(&self, quote: &str, line_no: i32, over: &[(&str, &str)]) -> String {
        let mut cols: Vec<(&str, String)> = vec![
            ("id", "gen_random_uuid()".into()),
            ("organization_id", format!("'{ORG_1}'")),
            ("quote_id", format!("'{quote}'")),
            ("published_rate_line_id", format!("'{}'", self.line)),
            ("shop_side", "'BUY'".into()),
            ("currency", "'JPY'".into()),
            ("denomination_group", "'ALL'".into()),
            ("quantity", "1000".into()),
            ("unit_size", "100".into()),
            ("base_rate", "23.5".into()),
            ("final_rate", "23.5".into()),
            ("thb_amount", "235.00".into()),
            ("fee_thb", "5.00".into()),
            ("settle_thb", "230.00".into()),
            ("line_no", line_no.to_string()),
            ("created_at", "now()".into()),
            ("updated_at", "now()".into()),
        ];
        replace(&mut cols, over);
        insert("quote_lines", &cols)
    }
}

fn replace(cols: &mut [(&str, String)], over: &[(&str, &str)]) {
    for (name, value) in over {
        let col = cols
            .iter_mut()
            .find(|c| c.0 == *name)
            .unwrap_or_else(|| panic!("no column {name} in the fixture"));
        col.1 = (*value).to_owned();
    }
}

fn insert(table: &str, cols: &[(&str, String)]) -> String {
    let names: Vec<&str> = cols.iter().map(|c| c.0).collect();
    let values: Vec<&str> = cols.iter().map(|c| c.1.as_str()).collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        names.join(", "),
        values.join(", ")
    )
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tokio::test]
async fn the_quotes_checks_admit_the_design_values_and_refuse_the_rest() {
    let db = TestDb::start().await;
    let f = Fixture::new(&db).await;

    // every CHECK admits the fixture
    assert_eq!(db.outcome(&f.quote(&new_id(), &[])).await, "ok", "fixture");
    for channel in ["DESK", "KIOSK", "BOOKING", "APP"] {
        let v = format!("'{channel}'");
        assert_eq!(
            db.outcome(&f.quote(&new_id(), &[("channel", &v)])).await,
            "ok",
            "channel {channel}"
        );
    }
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("status", "'CANCELLED'")]))
            .await,
        "ok",
        "CANCELLED"
    );
    assert_eq!(
        db.outcome(&f.quote(
            &new_id(),
            &[
                ("status", "'ACCEPTED'"),
                ("accepted_by_transaction_id", "gen_random_uuid()")
            ]
        ))
        .await,
        "ok",
        "ACCEPTED with its transaction"
    );

    // EXPIRED is derived, never stored (D-5)
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("status", "'EXPIRED'")]))
            .await,
        CHECK,
        "status EXPIRED"
    );
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("status", "'PENDING'")]))
            .await,
        CHECK,
        "status PENDING"
    );
    // ACCEPTED iff accepted_by_transaction_id
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("status", "'ACCEPTED'")]))
            .await,
        CHECK,
        "ACCEPTED without a transaction"
    );
    assert_eq!(
        db.outcome(&f.quote(
            &new_id(),
            &[("accepted_by_transaction_id", "gen_random_uuid()")]
        ))
        .await,
        CHECK,
        "ACTIVE with a transaction"
    );
    // INDICATIVE only until #555
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("type", "'BINDING'")]))
            .await,
        CHECK,
        "type BINDING"
    );
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("channel", "'WEB'")]))
            .await,
        CHECK,
        "channel WEB"
    );
    // benefit_snapshot defaults to '{}' (§3.1)
    let id = new_id();
    db.exec(&f.quote(&id, &[])).await;
    assert_eq!(
        db.text(&format!(
            "SELECT benefit_snapshot::text FROM quotes WHERE id = '{id}'"
        ))
        .await
        .as_deref(),
        Some("{}"),
        "benefit_snapshot DEFAULT '{{}}'"
    );

    // foreign keys: the publication, the branch, the customer
    assert_eq!(
        db.outcome(&f.quote(
            &new_id(),
            &[("publication_id", "'00000000-0000-0000-0000-0000000000ff'")]
        ))
        .await,
        FOREIGN_KEY,
        "a publication that does not exist"
    );
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("branch_id", "'NOPE'")]))
            .await,
        FOREIGN_KEY,
        "a branch that does not exist"
    );
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("customer_id", "'nobody'")]))
            .await,
        FOREIGN_KEY,
        "a customer that does not exist"
    );
    db.customer("C-1", "MAIN").await;
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("customer_id", "'C-1'")]))
            .await,
        "ok",
        "an existing customer"
    );
    // the composite (organization_id, branch_id) key: MAIN is not the second organisation's
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.second_organisation(org2, "XC2", "B2").await;
    assert_eq!(
        db.outcome(&f.quote(&new_id(), &[("organization_id", &format!("'{org2}'"))]))
            .await,
        FOREIGN_KEY,
        "a quote of organisation 2 naming organisation 1's branch"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_quote_lines_checks_admit_the_design_values_and_refuse_the_rest() {
    let db = TestDb::start().await;
    let f = Fixture::new(&db).await;
    let quote = new_id();
    db.exec(&f.quote(&quote, &[])).await;

    assert_eq!(db.outcome(&f.line(&quote, 1, &[])).await, "ok", "fixture");
    // a fee equal to the THB amount is admitted (settle 0.00)
    assert_eq!(
        db.outcome(&f.line(&quote, 2, &[("fee_thb", "235.00"), ("settle_thb", "0.00")]))
            .await,
        "ok",
        "fee = thb_amount"
    );
    // fee_thb DEFAULT 0, denomination_group DEFAULT 'ALL'
    let line = new_id();
    db.exec(&format!(
        "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, \
             shop_side, currency, quantity, unit_size, base_rate, final_rate, thb_amount, \
             settle_thb, line_no, created_at, updated_at) \
         VALUES ('{line}', '{ORG_1}', '{quote}', '{}', 'SELL', 'JPY', 1000, 100, 24, 24, 240.00, \
             240.00, 3, now(), now())",
        f.line
    ))
    .await;
    assert_eq!(
        db.text(&format!(
            "SELECT fee_thb::text || ' ' || denomination_group FROM quote_lines WHERE id = '{line}'"
        ))
        .await
        .as_deref(),
        Some("0.00 ALL"),
        "fee_thb DEFAULT 0, denomination_group DEFAULT 'ALL'"
    );

    // (what, the columns replaced, the SQLSTATE)
    type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, &'a str);
    let refused: Vec<Case> = vec![
        (
            "shop_side EXCHANGE",
            vec![("shop_side", "'EXCHANGE'")],
            CHECK,
        ),
        ("quantity 0", vec![("quantity", "0")], CHECK),
        ("unit_size 0", vec![("unit_size", "0")], CHECK),
        ("base_rate 0", vec![("base_rate", "0")], CHECK),
        ("final_rate 0", vec![("final_rate", "0")], CHECK),
        ("thb_amount -0.01", vec![("thb_amount", "-0.01")], CHECK),
        ("fee_thb -0.01", vec![("fee_thb", "-0.01")], CHECK),
        (
            "fee_thb above thb_amount",
            vec![("fee_thb", "235.01"), ("settle_thb", "0.00")],
            CHECK,
        ),
        ("settle_thb -0.01", vec![("settle_thb", "-0.01")], CHECK),
        (
            "a quote that does not exist",
            vec![("quote_id", "gen_random_uuid()")],
            FOREIGN_KEY,
        ),
        (
            "a published line that does not exist",
            vec![("published_rate_line_id", "gen_random_uuid()")],
            FOREIGN_KEY,
        ),
        (
            "a currency that does not exist",
            vec![("currency", "'ZZZ'")],
            FOREIGN_KEY,
        ),
        ("quantity NULL", vec![("quantity", "NULL")], NOT_NULL),
    ];
    for (what, over, code) in refused {
        assert_eq!(db.outcome(&f.line(&quote, 10, &over)).await, code, "{what}");
    }
    // UNIQUE (quote_id, line_no)
    assert_eq!(
        db.outcome(&f.line(&quote, 1, &[])).await,
        UNIQUE,
        "line 1 of the same quote twice"
    );
    db.finish().await;
}

#[tokio::test]
async fn every_organisation_gets_one_settings_row_of_300_and_the_limits_start_empty() {
    let db = TestDb::empty().await;
    db.migrate_through(Some(MIGRATION_1 - 1)).await;
    // a second organisation that exists before migration 1
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{org2}', 'XC2', NULL, 'Asia/Bangkok', now())"
    ))
    .await;
    let organisations = db.count("SELECT count(*) FROM organizations").await;
    assert_eq!(organisations, 2, "the setup");
    db.migrate_through(None).await;

    assert_eq!(
        db.count("SELECT count(*) FROM exchange_settings").await,
        organisations,
        "one exchange_settings row per organisation (§8 item 1)"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM exchange_settings s JOIN organizations o \
               ON o.id = s.organization_id WHERE s.quote_ttl_seconds = 300"
        )
        .await,
        organisations,
        "each organisation's row is 300 s (T1)"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM branch_exchange_limits")
            .await,
        0,
        "branch_exchange_limits starts empty: no limit, nothing waits (T2)"
    );
    // one row per organisation: a second row for the same organisation is refused
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO exchange_settings (organization_id, quote_ttl_seconds, updated_by, updated_at) \
             VALUES ('{org2}', 300, 'test', now())"
        ))
        .await,
        UNIQUE,
        "exchange_settings is keyed by organisation"
    );
    db.finish().await;
}

#[tokio::test]
async fn quote_ttl_seconds_outside_30_to_3600_is_refused_by_the_check() {
    let db = TestDb::start().await;
    let set = |ttl: i32| {
        format!(
            "UPDATE exchange_settings SET quote_ttl_seconds = {ttl} WHERE organization_id = '{ORG_1}'"
        )
    };
    for (ttl, want) in [
        (29, CHECK),
        (30, "ok"),
        (300, "ok"),
        (3600, "ok"),
        (3601, CHECK),
        (0, CHECK),
        (-300, CHECK),
    ] {
        assert_eq!(db.outcome(&set(ttl)).await, want, "quote_ttl_seconds {ttl}");
    }
    // DEFAULT 300 for a row that names no TTL
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.second_organisation(org2, "XC2", "B2").await;
    db.exec(&format!(
        "INSERT INTO exchange_settings (organization_id, updated_by, updated_at) \
         VALUES ('{org2}', 'test', now())"
    ))
    .await;
    assert_eq!(
        db.count(&format!(
            "SELECT quote_ttl_seconds::bigint FROM exchange_settings WHERE organization_id = '{org2}'"
        ))
        .await,
        300,
        "DEFAULT 300"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_branch_limit_is_null_or_positive_and_one_per_branch() {
    let db = TestDb::start().await;
    let row = |limit: &str| {
        format!(
            "INSERT INTO branch_exchange_limits (organization_id, branch_id, approval_limit_thb, \
                 updated_by, updated_at) VALUES ('{ORG_1}', 'MAIN', {limit}, 'test', now()) \
             ON CONFLICT DO NOTHING"
        )
    };
    assert_eq!(db.outcome(&row("0")).await, CHECK, "a limit of 0");
    assert_eq!(db.outcome(&row("-1.00")).await, CHECK, "a negative limit");
    assert_eq!(db.outcome(&row("NULL")).await, "ok", "no limit (NULL)");
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO branch_exchange_limits (organization_id, branch_id, approval_limit_thb, \
                 updated_by, updated_at) VALUES ('{ORG_1}', 'MAIN', 50000.00, 'test', now())"
        ))
        .await,
        UNIQUE,
        "PRIMARY KEY (organization_id, branch_id)"
    );
    assert_eq!(
        db.outcome(
            "UPDATE branch_exchange_limits SET approval_limit_thb = 50000.00 WHERE branch_id = 'MAIN'"
        )
        .await,
        "ok",
        "a positive limit"
    );
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO branch_exchange_limits (organization_id, branch_id, approval_limit_thb, \
                 updated_by, updated_at) VALUES ('{ORG_1}', 'NOPE', 1.00, 'test', now())"
        ))
        .await,
        FOREIGN_KEY,
        "a branch that does not exist"
    );
    db.finish().await;
}
