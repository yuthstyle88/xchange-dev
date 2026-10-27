//! RF-1: a refusal of the customer's details names the field at fault
//! (`docs/design/rpc-refusal-field.md` §4 rows R1–R13, decisions RF-D2, RF-D3, RF-D6).
//!
//! Written before the implementation by the sub-controller (RF-0). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! The DB rows (R10, R12, R13's `reserve`) need `TEST_DATABASE_URL`; they fail without it, they do not skip.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use pos_error::AppError;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use xc_customer::{
    BAD_DATE, BAD_MODE, BAD_TEXT, CustomerDetails, CustomerField, CustomerRefusal,
    CustomerService, INVALID_NATIONALITY, NO_SUCH_BRANCH, ReserveCustomer, ReserveError,
    install_pii_keys,
};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::input::InputError;
use xc_kernel::pii::PiiKeys;

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// The payload of `engine_actions.json` #77, which v2 accepts.
fn valid() -> ReserveCustomer {
    ReserveCustomer {
        name: "Customer 1".to_owned(),
        passport_no: "P1000001".to_owned(),
        nationality: "DEU".to_owned(),
        birth_date: "1990-01-02".to_owned(),
        passport_expiry: "2031-06-11".to_owned(),
        mode: "LIVE".to_owned(),
    }
}

fn with(edit: impl FnOnce(&mut ReserveCustomer)) -> ReserveCustomer {
    let mut cmd = valid();
    edit(&mut cmd);
    cmd
}

fn refusal_of(cmd: &ReserveCustomer) -> CustomerRefusal {
    match CustomerDetails::check(cmd) {
        Err(refusal) => refusal,
        Ok(details) => panic!("expected a refusal for {cmd:?}, got Ok({details:?})"),
    }
}

fn assert_names(cmd: &ReserveCustomer, field: CustomerField, sentence: &'static str) {
    assert_eq!(refusal_of(cmd), CustomerRefusal { field, sentence }, "payload {cmd:?}");
}

/// Every refusal case of R1–R11: (payload, field, sentence).
fn refused_cases() -> Vec<(ReserveCustomer, CustomerField, &'static str)> {
    let date_format = InputError::DateFormat.sentence();
    let date = InputError::Date.sentence();
    vec![
        // R1 (#68: blank name, 2-letter nationality)
        (with(|c| { c.name = String::new(); c.nationality = "DE".into() }), CustomerField::Name, BAD_TEXT),
        (with(|c| c.name = "   ".into()), CustomerField::Name, BAD_TEXT),
        // R2 (#69)
        (with(|c| c.name = "n".repeat(101)), CustomerField::Name, BAD_TEXT),
        // R3 (#70) and blank
        (with(|c| c.passport_no = "P".repeat(31)), CustomerField::PassportNo, BAD_TEXT),
        (with(|c| c.passport_no = String::new()), CustomerField::PassportNo, BAD_TEXT),
        // R4 (#71: missing ⇒ empty after reserve_command) and 61 units
        (with(|c| c.nationality = String::new()), CustomerField::Nationality, BAD_TEXT),
        (with(|c| c.nationality = "N".repeat(61)), CustomerField::Nationality, BAD_TEXT),
        // R5
        (with(|c| c.nationality = "DE".into()), CustomerField::Nationality, INVALID_NATIONALITY),
        // R6 (#74)
        (with(|c| c.birth_date = "01/02/1990".into()), CustomerField::BirthDate, date_format),
        // R7 (#75) and > 10 units
        (with(|c| c.birth_date = "1990-02-30".into()), CustomerField::BirthDate, date),
        (with(|c| c.birth_date = "1990-02-011".into()), CustomerField::BirthDate, BAD_TEXT),
        // R8 (#76) and before the floor
        (with(|c| c.passport_expiry = "2031-13-01".into()), CustomerField::PassportExpiry, date),
        (with(|c| c.passport_expiry = "1920-04-01".into()), CustomerField::PassportExpiry, date),
        // R9 (#72, #73)
        (with(|c| c.mode = "REAL".into()), CustomerField::Mode, BAD_MODE),
        (with(|c| c.mode = String::new()), CustomerField::Mode, BAD_MODE),
        // R11
        (with(|c| { c.name = String::new(); c.birth_date = "1990-02-30".into() }), CustomerField::Name, BAD_TEXT),
    ]
}

// ── R1–R9, R11: CustomerDetails::check ──────────────────────────────────────

// R1
#[test]
fn blank_name_names_name() {
    assert_names(&with(|c| { c.name = String::new(); c.nationality = "DE".into() }), CustomerField::Name, BAD_TEXT);
    assert_names(&with(|c| c.name = "   ".into()), CustomerField::Name, BAD_TEXT);
}

// R2
#[test]
fn long_name_names_name() {
    assert_names(&with(|c| c.name = "n".repeat(101)), CustomerField::Name, BAD_TEXT);
    let ok = CustomerDetails::check(&with(|c| c.name = "n".repeat(100))).expect("100 units is allowed");
    assert_eq!(ok.name.len(), 100);
}

// R3
#[test]
fn long_or_blank_passport_names_passport_no() {
    assert_names(&with(|c| c.passport_no = "P".repeat(31)), CustomerField::PassportNo, BAD_TEXT);
    assert_names(&with(|c| c.passport_no = String::new()), CustomerField::PassportNo, BAD_TEXT);
}

// R4
#[test]
fn missing_nationality_names_nationality() {
    assert_names(&with(|c| c.nationality = String::new()), CustomerField::Nationality, BAD_TEXT);
    assert_names(&with(|c| c.nationality = "N".repeat(61)), CustomerField::Nationality, BAD_TEXT);
}

// R5
#[test]
fn two_letter_nationality_names_nationality() {
    assert_names(&with(|c| c.nationality = "DE".into()), CustomerField::Nationality, INVALID_NATIONALITY);
    assert_names(&with(|c| c.nationality = "D3U".into()), CustomerField::Nationality, INVALID_NATIONALITY);
}

// R6
#[test]
fn birth_date_format_names_birth_date() {
    assert_names(
        &with(|c| c.birth_date = "01/02/1990".into()),
        CustomerField::BirthDate,
        InputError::DateFormat.sentence(),
    );
}

// R7
#[test]
fn bad_birth_date_names_birth_date() {
    assert_names(&with(|c| c.birth_date = "1990-02-30".into()), CustomerField::BirthDate, BAD_DATE);
    assert_names(&with(|c| c.birth_date = "1990-02-011".into()), CustomerField::BirthDate, BAD_TEXT);
}

// R8
#[test]
fn bad_expiry_names_passport_expiry() {
    assert_names(&with(|c| c.passport_expiry = "2031-13-01".into()), CustomerField::PassportExpiry, BAD_DATE);
    assert_names(&with(|c| c.passport_expiry = "1920-04-01".into()), CustomerField::PassportExpiry, BAD_DATE);
    assert_names(
        &with(|c| c.passport_expiry = "11/06/2031".into()),
        CustomerField::PassportExpiry,
        InputError::DateFormat.sentence(),
    );
}

// R9
#[test]
fn bad_mode_names_mode() {
    assert_names(&with(|c| c.mode = "REAL".into()), CustomerField::Mode, BAD_MODE);
    assert_names(&with(|c| c.mode = String::new()), CustomerField::Mode, BAD_MODE);
}

// R11 (RF-D6: the first fault in payload order)
#[test]
fn first_fault_wins() {
    assert_names(
        &with(|c| { c.name = String::new(); c.birth_date = "1990-02-30".into() }),
        CustomerField::Name,
        BAD_TEXT,
    );
    assert_names(
        &with(|c| { c.passport_no = String::new(); c.mode = "REAL".into() }),
        CustomerField::PassportNo,
        BAD_TEXT,
    );
    // the 3-letter rule comes after every per-key rule
    assert_names(
        &with(|c| { c.nationality = "DE".into(); c.mode = "REAL".into() }),
        CustomerField::Mode,
        BAD_MODE,
    );
}

#[test]
fn check_accepts_what_parse_accepts() {
    let cmd = with(|c| { c.name = "  Customer One  ".into(); c.passport_no = " p1000001 ".into(); c.nationality = "deu".into() });
    assert_eq!(
        CustomerDetails::check(&cmd).expect("valid"),
        CustomerDetails::parse(&cmd).expect("valid"),
    );
}

#[test]
fn every_field_has_its_payload_key() {
    let keys: Vec<&str> = [
        CustomerField::Name,
        CustomerField::PassportNo,
        CustomerField::Nationality,
        CustomerField::BirthDate,
        CustomerField::PassportExpiry,
        CustomerField::Mode,
    ]
    .into_iter()
    .map(CustomerField::wire)
    .collect();
    assert_eq!(keys, ["name", "passportNo", "nationality", "birthDate", "passportExpiry", "mode"]);
}

// ── DB rows: R10, R12, R13 ──────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    admin_url: String,
    schema: String,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL must be set, e.g. postgres://postgres@localhost:5432/xchange_test")
        });
        let schema = xc_testdb::unique_schema_name("rf1");
        let admin = PgPoolOptions::new().max_connections(1).connect(&url).await.expect("admin connect");
        sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;
        let options: sqlx::postgres::PgConnectOptions = url.parse().expect("a PostgreSQL URL");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect schema pool");
        let mut entries: Vec<_> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations"))
            .expect("read migrations dir")
            .map(|r| r.expect("read entry").path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "sql"))
            .collect();
        entries.sort();
        for path in entries {
            let sql = std::fs::read_to_string(&path).expect("read a migration");
            sqlx::raw_sql(AssertSqlSafe(sql))
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        sqlx::query("INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING")
            .execute(&pool)
            .await
            .expect("insert MAIN");
        let _ = install_pii_keys(Arc::new(PiiKeys::parse(TEST_KEYS).expect("test keys")));
        Db { pool, admin_url: url, schema }
    }

    fn service(&self) -> CustomerService {
        CustomerService::new(BranchTx::new(pos_infra::transactions::Transactor::new(self.pool.clone())))
    }

    async fn customers(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM customers")
            .fetch_one(&self.pool)
            .await
            .expect("count customers")
    }

    async fn stop(self) {
        self.pool.close().await;
        let admin = PgPoolOptions::new().max_connections(1).connect(&self.admin_url).await.expect("admin connect");
        sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {} CASCADE", self.schema)))
            .execute(&admin)
            .await
            .expect("drop schema");
        admin.close().await;
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap()
}

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

// R10
#[test]
fn passport_of_only_dashes_names_passport_no() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        let cmd = with(|c| c.passport_no = "---".into());
        match db.service().reserve_checked(&main_branch(), cmd, now()).await {
            Err(ReserveError::Input(refusal)) => assert_eq!(
                refusal,
                CustomerRefusal { field: CustomerField::PassportNo, sentence: BAD_TEXT }
            ),
            other => panic!("expected Input(PassportNo, BAD_TEXT), got {other:?}"),
        }
        assert_eq!(db.customers().await, 0, "nothing is written");
        db.stop().await;
    });
}

// R12
#[test]
fn unknown_branch_has_no_field() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        match db.service().reserve_checked(&BranchId::new("NOPE"), valid(), now()).await {
            Err(ReserveError::Other(AppError::Domain(sentence))) => assert_eq!(sentence, NO_SUCH_BRANCH),
            other => panic!("expected Other(Domain(NO_SUCH_BRANCH)), got {other:?}"),
        }
        let reserved = db
            .service()
            .reserve_checked(&main_branch(), valid(), now())
            .await
            .expect("a valid payload is reserved");
        assert!(!reserved.existing);
        assert_eq!(db.customers().await, 1);
        db.stop().await;
    });
}

// R13: callers that want no field see today's sentence
#[test]
fn parse_and_reserve_answer_the_same_sentence() {
    for (cmd, _field, sentence) in refused_cases() {
        match CustomerDetails::parse(&cmd) {
            Err(AppError::Domain(got)) => assert_eq!(got, sentence, "parse of {cmd:?}"),
            other => panic!("parse of {cmd:?}: expected Domain({sentence}), got {other:?}"),
        }
        let refusal = refusal_of(&cmd);
        assert_eq!(AppError::from(refusal).to_string(), AppError::Domain(sentence.to_owned()).to_string());
    }
    sqlx::test_block_on(async {
        let db = Db::start().await;
        let mut cases = refused_cases();
        cases.push((with(|c| c.passport_no = "---".into()), CustomerField::PassportNo, BAD_TEXT));
        for (cmd, field, sentence) in cases {
            match db.service().reserve(&main_branch(), cmd.clone(), now()).await {
                Err(AppError::Domain(got)) => assert_eq!(got, sentence, "reserve of {cmd:?}"),
                other => panic!("reserve of {cmd:?}: expected Domain({sentence}), got {other:?}"),
            }
            match db.service().reserve_checked(&main_branch(), cmd.clone(), now()).await {
                Err(ReserveError::Input(refusal)) => {
                    assert_eq!(refusal, CustomerRefusal { field, sentence }, "reserve_checked of {cmd:?}")
                }
                other => panic!("reserve_checked of {cmd:?}: expected Input, got {other:?}"),
            }
        }
        match db.service().reserve(&BranchId::new("NOPE"), valid(), now()).await {
            Err(AppError::Domain(got)) => assert_eq!(got, NO_SUCH_BRANCH),
            other => panic!("expected Domain(NO_SUCH_BRANCH), got {other:?}"),
        }
        assert_eq!(db.customers().await, 0, "no refused payload is written");
        db.stop().await;
    });
}
