//! Up-front tests for ticket CS-19 (docs/design/configurable-shifts.md §2.3 "Handover", §4 row H1). Written by the
//! ticket author claude/35271802 — do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Accepting a hand-over opens the receiver's drawer, so it follows rule 3: the accept carries the **receiver's**
//! chosen template, checked against the accept time with the OPEN count's refusals, and the receiver's new shift gets
//! that instance's template, name and business date. Seam: `HandoverService::accept_chosen_or_await_in(tx, actor, id,
//! shift_template_id, now)`.
//!
//! Fixture "overlap24", stored by SQL: A 06:00–15:00, B 14:00–23:00, C 22:00–07:00, cutover 06:00, Asia/Bangkok. The
//! giver's own shift is opened at a moment only A covers, so its OPEN count needs no choice before or after CS-7.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverAccepted, HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::handover::Handover;
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

/// One branch on overlap24 with a giver (G), a receiver (R) and a manager (M).
struct Floor {
    pool: PgPool,
    main: BranchId,
    giver: Actor,
    receiver: Actor,
    manager: Actor,
}

async fn floor() -> Floor {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| panic!("TEST_DATABASE_URL is not set; these tests do not skip"));
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(options.get_database().unwrap_or("").ends_with("_test"));
    let schema = unique("cs19");
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
    sqlx::query("INSERT INTO branches (id, name, timezone) VALUES ($1, $1, 'Asia/Bangkok')")
        .bind(&id)
        .execute(&pool)
        .await
        .expect("insert a branch");
    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1::uuid, $2, 1000000::numeric, 'test', now())",
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
    let plan = [
        ("A", "06:00", "15:00"),
        ("B", "14:00", "23:00"),
        ("C", "22:00", "07:00"),
    ];
    for (i, (name, start, end)) in plan.iter().enumerate() {
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
    let giver = person(&pool, "G", &[Role::Teller], &main).await;
    let receiver = person(&pool, "R", &[Role::Teller], &main).await;
    let manager = person(&pool, "M", &[Role::BranchManager], &main).await;
    Floor {
        pool,
        main,
        giver,
        receiver,
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

    /// THB 1000 × 2.
    async fn lines(&self) -> Vec<CountLine> {
        let face: i64 = sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 1000::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("THB 1000");
        vec![CountLine::new("THB", face, 2)]
    }

    /// The giver's own shift: G counts OPEN at `at` (only A covers it), M verifies a minute later.
    async fn giver_opens(&self, at: DateTime<Utc>) -> String {
        let counts = CashCountService::pg(self.pool.clone());
        let count = counts
            .submit(
                &self.giver,
                SubmitCount {
                    branch: self.main.clone(),
                    shift_id: None,
                    kind: CountKind::Open,
                    lines: self.lines().await,
                    note: None,
                    idempotency_key: unique("open"),
                },
                at,
            )
            .await
            .expect("G submits the OPEN count");
        counts
            .verify(&self.manager, &count.id, at + chrono::Duration::minutes(1))
            .await
            .expect("M verifies it")
            .shift_id
            .expect("a verified OPEN count names its shift")
    }

    /// G hands the whole drawer (THB 2,000) to R at `at`.
    async fn hand_over(&self, from_shift: &str, at: DateTime<Utc>) -> Handover {
        HandoverService::pg(self.pool.clone())
            .start(
                &self.giver,
                StartHandover {
                    branch: self.main.clone(),
                    from_shift_id: from_shift.to_owned(),
                    receiver_id: self.receiver.user_id.clone(),
                    lines: self.lines().await,
                    note: None,
                    idempotency_key: unique("handover"),
                },
                at,
            )
            .await
            .expect("G starts the hand-over to R")
    }

    /// R accepts choosing `template` at `now`; commits only on success.
    async fn accept(
        &self,
        handover: &Handover,
        template: Option<&str>,
        now: DateTime<Utc>,
    ) -> pos_error::AppResult<HandoverAccepted> {
        let svc = HandoverService::pg(self.pool.clone());
        let mut tx = self.pool.begin().await.unwrap();
        let got = svc
            .accept_chosen_or_await_in(&mut tx, &self.receiver, &handover.id, template, now)
            .await;
        if got.is_ok() {
            tx.commit().await.unwrap();
        }
        got
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

    /// The hand-over's status and R's OPEN shifts.
    async fn after_refusal(&self, handover: &Handover) -> (String, i64) {
        let status: String =
            sqlx::query_scalar("SELECT status FROM handovers WHERE id = $1::uuid")
                .bind(&handover.id)
                .fetch_one(&self.pool)
                .await
                .expect("the hand-over");
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(&self.receiver.user_id)
        .fetch_one(&self.pool)
        .await
        .expect("R's open shifts");
        (status, open)
    }
}

fn accepted_shift(got: pos_error::AppResult<HandoverAccepted>) -> String {
    match got {
        Ok(HandoverAccepted::Accepted(h)) => h.to_shift_id.expect("an accepted hand-over names R's shift"),
        Ok(other) => panic!("the hand-over is accepted, got {other:?}"),
        Err(e) => panic!("the hand-over is accepted, got {e}"),
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
async fn h1_accepting_at_1430_choosing_b_opens_the_receivers_shift_as_b_of_4_nov() {
    let f = floor().await;
    let from = f.giver_opens(bkk(4, 10, 0)).await;
    let h = f.hand_over(&from, bkk(4, 14, 25)).await;
    let b = f.template_id("B").await;
    let shift = accepted_shift(f.accept(&h, Some(&b), bkk(4, 14, 30)).await);
    assert_eq!(
        f.shift_row(&shift).await,
        (Some(b), Some("B".to_owned()), nov(4)),
        "H1: the receiver's own choice, not the giver's A"
    );
}

#[tokio::test]
async fn h1b_choosing_a_shift_that_does_not_cover_the_accept_is_refused_and_nothing_moves() {
    let f = floor().await;
    let from = f.giver_opens(bkk(4, 10, 0)).await;
    let h = f.hand_over(&from, bkk(4, 14, 25)).await;
    let c = f.template_id("C").await;
    refused_with(
        f.accept(&h, Some(&c), bkk(4, 14, 30)).await,
        ShiftError::OutsideShiftWindow,
    );
    assert_eq!(
        f.after_refusal(&h).await,
        ("PENDING".to_owned(), 0),
        "H1b: still pending, R holds no shift"
    );
}

#[tokio::test]
async fn h1c_no_choice_takes_the_only_covering_shift() {
    let f = floor().await;
    let from = f.giver_opens(bkk(4, 8, 0)).await;
    let h = f.hand_over(&from, bkk(4, 9, 55)).await;
    let a = f.template_id("A").await;
    let shift = accepted_shift(f.accept(&h, None, bkk(4, 10, 0)).await);
    assert_eq!(
        f.shift_row(&shift).await,
        (Some(a), Some("A".to_owned()), nov(4)),
        "H1c: only A covers 10:00"
    );
}

#[tokio::test]
async fn h1d_no_choice_while_two_shifts_cover_is_refused_as_not_chosen() {
    let f = floor().await;
    let from = f.giver_opens(bkk(4, 10, 0)).await;
    let h = f.hand_over(&from, bkk(4, 14, 25)).await;
    refused_with(
        f.accept(&h, None, bkk(4, 14, 30)).await,
        ShiftError::ShiftNotChosen,
    );
    assert_eq!(
        f.after_refusal(&h).await,
        ("PENDING".to_owned(), 0),
        "§2.3: the same refusals as the OPEN count; nothing moves"
    );
}
