//! Up-front tests for ticket CS-7 (docs/design/configurable-shifts.md §2.3, §4 rows C1–C6, K1–K2): an OPEN count
//! carries the shift the teller opens as, every count carries a business date, and the verified OPEN count opens the
//! shift with that template, its name and its business date. Written by the ticket author claude/35271802 — do not
//! edit, delete or weaken. Needs CS-1, CS-2 and CS-3 merged into `feat/configurable-shifts`, and `TEST_DATABASE_URL`.
//!
//! Every branch here uses an overlapping 24-hour plan (owner: every plan covers 24 hours): A 06–15, B 14–23, C 22–07,
//! cutover 06:00, Bangkok.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CashCount, CountKind, CountLine};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// Bangkok wall-clock instant on 2026-11-`d`.
fn bkk(d: u32, h: u32, m: u32) -> DateTime<Utc> {
    FixedOffset::east_opt(7 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, d, h, m, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn nov(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

struct Floor {
    pool: PgPool,
    main: BranchId,
    teller: Actor,
    manager: Actor,
}

async fn floor() -> Floor {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| panic!("TEST_DATABASE_URL is not set; these tests do not skip"));
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(options.get_database().unwrap_or("").ends_with("_test"));
    let schema = unique("cs7");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("connect");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("schema");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    Migrator::new(dir.as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");

    let id = unique("main");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&id)
        .execute(&pool)
        .await
        .expect("insert a branch");
    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1::uuid, $2, 500::numeric, 'test', now())",
    )
    .bind(ORG)
    .bind(&id)
    .execute(&pool)
    .await
    .expect("a count-difference limit");
    // The overlapping plan replaces the defaults the branch trigger seeded.
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .expect("retire the defaults");
    for (i, (name, start, end)) in [
        ("A", "06:00", "15:00"),
        ("B", "14:00", "23:00"),
        ("C", "22:00", "07:00"),
    ]
    .iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, position) \
             VALUES ($1::uuid, $2, $3, $4::time, $5::time, $6)",
        )
        .bind(ORG)
        .bind(&id)
        .bind(name)
        .bind(start)
        .bind(end)
        .bind(i as i16)
        .execute(&pool)
        .await
        .expect("a template");
    }
    let main = BranchId::new(id);
    let mut tx = pool.begin().await.unwrap();
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(main.clone()),
                currency: CurrencyCode::THB,
                quantity: ForeignAmount::from_decimal(Decimal::from_str("10000.00").unwrap()),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: unique("fund"),
                    source_key: "thb".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("fund the safe: {e:?}"));
    tx.commit().await.unwrap();
    let teller = person(&pool, "T", &[Role::Teller], &main).await;
    let manager = person(&pool, "M", &[Role::BranchManager], &main).await;
    Floor {
        pool,
        main,
        teller,
        manager,
    }
}

async fn person(pool: &PgPool, name: &str, roles: &[Role], branch: &BranchId) -> Actor {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert a user");
    for role in roles {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.as_str())
        .execute(pool)
        .await
        .expect("give the user a role");
    }
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        &id,
        ORG,
        roles
            .iter()
            .map(|r| ScopedRole::new(*r, Some(branch.clone()), from, None))
            .collect(),
        None,
    )
}

impl Floor {
    async fn template_id(&self, name: &str) -> String {
        sqlx::query_scalar(
            "SELECT id::text FROM shift_templates WHERE branch_id = $1 AND name = $2 AND retired_at IS NULL",
        )
        .bind(self.main.as_str())
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("a live template")
    }

    async fn lines(&self) -> Vec<CountLine> {
        let face: i64 = sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 1000::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("THB 1000");
        vec![CountLine::new("THB", face, 2)]
    }

    async fn submit(
        &self,
        who: &Actor,
        kind: CountKind,
        shift: Option<String>,
        template: Option<&str>,
        now: DateTime<Utc>,
    ) -> pos_error::AppResult<CashCount> {
        let cmd = SubmitCount {
            branch: self.main.clone(),
            shift_id: shift,
            kind,
            lines: self.lines().await,
            note: None,
            idempotency_key: unique("count"),
        };
        let counts = CashCountService::pg(self.pool.clone());
        let mut tx = self.pool.begin().await.unwrap();
        let got = counts
            .submit_chosen_in(&mut tx, who, cmd, template, now)
            .await;
        if got.is_ok() {
            tx.commit().await.unwrap();
        }
        got
    }

    async fn verify_open(&self, count_id: &str, now: DateTime<Utc>) -> String {
        let counts = CashCountService::pg(self.pool.clone());
        let mut tx = self.pool.begin().await.unwrap();
        let got = counts
            .verify_in(&mut tx, &self.manager, count_id, now)
            .await
            .expect("M verifies");
        tx.commit().await.unwrap();
        match got {
            CountVerified::Verified(c) => c.shift_id.expect("the new shift"),
            other => panic!("the OPEN count verifies: {other:?}"),
        }
    }

    async fn count_row(&self, id: &str) -> (Option<String>, NaiveDate) {
        sqlx::query_as(
            "SELECT shift_template_id::text, business_date FROM cash_counts WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("the count row")
    }

    async fn shift_row(&self, id: &str) -> (Option<String>, Option<String>, NaiveDate) {
        sqlx::query_as(
            "SELECT template_id::text, template_name, business_date FROM shifts WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("the shift row")
    }

    async fn counts(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM cash_counts WHERE branch_id = $1")
            .bind(self.main.as_str())
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn refused_with<T: std::fmt::Debug>(got: pos_error::AppResult<T>, why: ShiftError) {
    match got {
        Err(e) => assert!(
            e.to_string().contains(&why.to_string()),
            "expected {why:?}, got {e}"
        ),
        Ok(v) => panic!("expected {why:?}, got {v:?}"),
    }
}

#[tokio::test]
async fn c1_c3_a_chosen_shift_that_does_not_cover_now_is_refused_and_nothing_is_written() {
    let f = floor().await;
    let b = f.template_id("B").await;
    let c = f.template_id("C").await;
    refused_with(
        f.submit(&f.teller, CountKind::Open, None, Some(&b), bkk(4, 10, 0))
            .await,
        ShiftError::OutsideShiftWindow,
    );
    refused_with(
        f.submit(&f.teller, CountKind::Open, None, Some(&c), bkk(4, 12, 0))
            .await,
        ShiftError::OutsideShiftWindow,
    );
    assert_eq!(f.counts().await, 0);
}

#[tokio::test]
async fn c2_c5_a_chosen_covering_shift_is_stored_and_opens_the_shift_with_its_name_and_date() {
    let f = floor().await;
    let b = f.template_id("B").await;
    let count = f
        .submit(&f.teller, CountKind::Open, None, Some(&b), bkk(4, 14, 30))
        .await
        .expect("an OPEN count as B at 14:30");
    assert_eq!(f.count_row(&count.id).await, (Some(b.clone()), nov(4)));
    // C5: verified after A has ended (15:00), still inside B
    let shift = f.verify_open(&count.id, bkk(4, 15, 30)).await;
    assert_eq!(
        f.shift_row(&shift).await,
        (Some(b), Some("B".to_owned()), nov(4))
    );
}

#[tokio::test]
async fn c4_no_choice_takes_the_only_covering_shift_and_refuses_when_two_cover() {
    let f = floor().await;
    refused_with(
        f.submit(&f.teller, CountKind::Open, None, None, bkk(4, 14, 30))
            .await,
        ShiftError::ShiftNotChosen,
    );
    let a = f.template_id("A").await;
    let count = f
        .submit(&f.teller, CountKind::Open, None, None, bkk(4, 10, 0))
        .await
        .expect("only A covers 10:00");
    assert_eq!(f.count_row(&count.id).await, (Some(a), nov(4)));
}

#[tokio::test]
async fn c6_k1_the_night_shift_and_its_close_count_belong_to_the_day_it_started() {
    let f = floor().await;
    let c = f.template_id("C").await;
    let open = f
        .submit(&f.teller, CountKind::Open, None, Some(&c), bkk(4, 23, 50))
        .await
        .expect("an OPEN count as C at 23:50");
    let shift = f.verify_open(&open.id, bkk(5, 1, 0)).await;
    assert_eq!(
        f.shift_row(&shift).await,
        (Some(c), Some("C".to_owned()), nov(4)),
        "C6: verified after midnight, still C of 4 Nov"
    );
    // K1: the CLOSE count at 07:10, after C has ended and after the 06:00 cutover, carries the shift's business date.
    let close = f
        .submit(
            &f.teller,
            CountKind::Close,
            Some(shift),
            None,
            bkk(5, 7, 10),
        )
        .await
        .expect("a CLOSE count");
    assert_eq!(f.count_row(&close.id).await, (None, nov(4)));
}

#[tokio::test]
async fn k2_a_safe_count_takes_the_business_date_of_its_moment() {
    let f = floor().await;
    let night = f
        .submit(&f.manager, CountKind::Safe, None, None, bkk(5, 5, 0))
        .await
        .expect("a SAFE count needs no shift");
    assert_eq!(
        f.count_row(&night.id).await,
        (None, nov(4)),
        "05:00 is before the 06:00 cutover"
    );
    let morning = f
        .submit(&f.manager, CountKind::Safe, None, None, bkk(5, 6, 30))
        .await
        .expect("a SAFE count needs no shift");
    assert_eq!(f.count_row(&morning.id).await, (None, nov(5)));
}
