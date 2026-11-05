//! Up-front tests for CS-8c (integration review of PR 6274, lane controller claude/022a7453): since CS-8 a LIVE sale is
//! dated by its shift's `business_date`, so a shift left open past FINANCE's certification would date new sales into
//! a closed day. `CreateTransaction` must refuse with `dayClosed` before it writes anything, as backdated keying
//! (`backdate.rs`, `ClosedDays::is_closed_in`) does. Written by the ticket author claude/35271802 — the implementer must
//! not edit, delete or weaken these.
//!
//! The harness is a copy of `tx4_create_test.rs`'s `Counter` (CS-8 row D1's setup). Needs `TEST_DATABASE_URL`.

mod quote_common;
mod tx4_common;

use std::sync::{Arc, Once};

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use tx4_common::*;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::ExchangeTransaction;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::create::{
    CreatePayment, CreateRequest, CreateTransaction,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::infrastructure::files::file_service::FileService;

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

struct Counter {
    desk: Desk,
    files: FileService,
    customers: CustomerService,
    scratch: std::path::PathBuf,
}

impl Counter {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            xc_customer::install_pii_keys(keys).expect("one test PII key per process");
        });
        let desk = Desk::start().await;
        let pool = desk.db.pool.clone();
        let scratch = std::env::temp_dir().join(unique("xc-cs8c"));
        std::fs::create_dir_all(&scratch).unwrap();
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        Self {
            files: FileService::new(pool.clone(), scratch.join("uploads")),
            customers: CustomerService::new(branch_tx),
            desk,
            scratch,
        }
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.desk.db.finish().await;
    }

    async fn customer(&self) -> String {
        self.customers
            .reserve(
                &BranchId::new("MAIN"),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: "AA1234567".to_owned(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "LIVE".to_owned(),
                },
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("reserve: {e}"))
            .customer_id
    }

    async fn upload_as(&self, who: &str) -> String {
        self.files
            .upload(
                &BranchId::new("MAIN"),
                who,
                "image/jpeg",
                JPEG,
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("upload: {e}"))
    }

    async fn shift_teller(&self) -> SaveActor {
        let id: String = ensure_test_user(&self.desk.db.pool).await;
        SaveActor {
            id,
            username: "test".to_owned(),
            employee_id: None,
            standing: xc_auth::domain::access::StandingKind::Staff(
                xc_auth::domain::access::Role::Teller,
            ),
        }
    }

    /// MAIN's open shift of the `test` teller, dated `business_date`.
    async fn open_shift_dated(&self, business_date: &str) -> String {
        ensure_test_user(&self.desk.db.pool).await;
        let shift = ca10_seed_open(
            &self.desk.db.pool,
            "MAIN",
            unique("open-shift").replace('_', "-"),
            &[],
            whole_second_now(),
        )
        .await;
        sqlx::query("UPDATE shifts SET business_date = $1::date WHERE id = $2")
            .bind(business_date)
            .bind(&shift)
            .execute(&self.desk.db.pool)
            .await
            .expect("date the open shift");
        shift
    }

    /// FINANCE certifies MAIN's `date` (the row `ClosedDays::is_closed_in` reads).
    async fn certify(&self, date: &str) {
        self.desk
            .db
            .exec(&format!(
                "INSERT INTO reconciliation_certifications (id, organization_id, branch_id, business_date, \
                     figures_hash, figures, certified_by, certified_at) \
                 VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{date}', 'h', '{{}}', gen_random_uuid(), now())"
            ))
            .await;
    }

    fn request(&self, q: &Quote, customer: &str, passport: &str) -> CreateRequest {
        CreateRequest {
            branch_id: BranchId::new("MAIN"),
            quote_id: q.id,
            customer_id: customer.to_owned(),
            booking_id: None,
            kiosk_request_id: None,
            staff_draft_id: None,
            passport_key: passport.to_owned(),
            payment: CreatePayment {
                method: PaymentMethod::Cash,
                reference: "R-1".to_owned(),
                account: "Cash drawer 1".to_owned(),
                slip_key: None,
                bank_fee_thb: money("0.00"),
                paid_at: q.created_at,
            },
            value_date: NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(),
            notes: "a note".to_owned(),
            count_confirmed: true,
        }
    }

    async fn create_as(
        &self,
        req: CreateRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        let service = CreateTransaction::new(self.desk.db.pool.clone(), self.files.clone());
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, req, who, now).await;
        tx.commit().await.unwrap();
        got
    }

    async fn transactions(&self) -> i64 {
        self.desk
            .db
            .count("SELECT count(*) FROM transactions")
            .await
    }

    /// A LIVE create as the shift's teller, on a shift dated `shift_date`, after FINANCE certified `certified`.
    async fn create_on(&self, shift_date: &str, certified: &str) -> AppResult<ExchangeTransaction> {
        let now = whole_second_now();
        let customer = self.customer().await;
        let q = self.desk.quote("MAIN", Side::Buy, "100", None, now).await;
        self.open_shift_dated(shift_date).await;
        self.certify(certified).await;
        let owner = self.shift_teller().await;
        let passport = self.upload_as(&owner.id).await;
        let req = self.request(&q, &customer, &passport);
        self.create_as(req, &owner, later(now, 5)).await
    }
}

/// K1: the open shift is dated 4 Nov and FINANCE has certified 4 Nov: the LIVE create is refused with `dayClosed`
/// naming the date, and no transaction is written.
#[tokio::test]
async fn k1_a_live_sale_into_a_certified_day_is_refused() {
    let c = Counter::start().await;
    let before = c.transactions().await;
    let got = c.create_on("2026-11-04", "2026-11-04").await;
    match &got {
        Err(AppError::Domain(m)) => {
            assert!(m.starts_with("dayClosed"), "code dayClosed: {m}");
            assert!(m.contains("2026-11-04"), "names the closed date: {m}");
        }
        other => panic!("expected Err(Domain(\"dayClosed: …\")), got {other:?}"),
    }
    assert_eq!(c.transactions().await, before, "nothing is written");
    c.finish().await;
}

/// K2: another day is certified (3 Nov): the sale goes through, dated by its shift, 4 Nov.
#[tokio::test]
async fn k2_a_certification_of_another_day_does_not_refuse() {
    let c = Counter::start().await;
    let made = c
        .create_on("2026-11-04", "2026-11-03")
        .await
        .expect("4 Nov is still open");
    assert_eq!(
        made.value_date,
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    );
    c.finish().await;
}

// Copied from `tx4_create_test.rs` (private there).
async fn ensure_test_user(pool: &sqlx::PgPool) -> String {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT u.id::text FROM users u JOIN organizations o ON o.id = u.organization_id \
         WHERE o.code = 'XC' AND u.name = 'test' AND u.created_by = 'test' \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .expect("look up test user")
    {
        return id;
    }
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text",
    )
    .fetch_one(pool)
    .await
    .expect("seed test user")
}

/// CA-10 (#626, #2955): v2's `ShiftService::open` is deleted with the core. The OPEN shift it wrote (opener `test`,
/// the given opening), written straight into the tables in one branch transaction (setup only). Answers its id.
async fn ca10_seed_open(
    pool: &sqlx::PgPool,
    branch: &str,
    request_id: String,
    opening: &[(&str, &str)],
    opened_at: chrono::DateTime<chrono::Utc>,
) -> String {
    let shift = format!("SH-{}", uuid::Uuid::new_v4());
    let (id, b) = (shift.clone(), branch.to_owned());
    let opening: Vec<(String, String)> = opening
        .iter()
        .map(|(c, a)| ((*c).to_owned(), (*a).to_owned()))
        .collect();
    xc_kernel::branch_tx::BranchTx::new(pos_infra::transactions::Transactor::new(pool.clone()))
        .run(&xc_kernel::ids::BranchId::new(branch), move |tx| {
            Box::pin(async move {
                let teller_id: uuid::Uuid = if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
                    "SELECT u.id FROM users u JOIN organizations o ON o.id = u.organization_id \
                     WHERE o.code = 'XC' AND u.name = 'test' AND u.created_by = 'test' \
                     ORDER BY u.created_at, u.id LIMIT 1",
                )
                .fetch_optional(&mut **tx)
                .await?
                {
                    id
                } else {
                    sqlx::query_scalar(
                        "INSERT INTO users (organization_id, name, status, created_by) \
                         SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
                         RETURNING id",
                    )
                    .fetch_one(&mut **tx)
                    .await?
                };
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, \
                     teller_id) VALUES ($1, $2, 'OPEN', $3, 'test', '', $4, $5)",
                )
                .bind(&id)
                .bind(&b)
                .bind(opened_at)
                .bind(&request_id)
                .bind(teller_id)
                .execute(&mut **tx)
                .await?;
                for (code, amount) in &opening {
                    let quantity: Decimal = amount.parse().expect("an opening amount");
                    if quantity <= Decimal::ZERO {
                        continue;
                    }
                    LedgerCashPostings::new()
                        .post_in(
                            &mut *tx,
                            &[PostingRequest {
                                from: CashPlace::Bank,
                                to: CashPlace::Drawer(id.clone()),
                                currency: xc_kernel::money::CurrencyCode::parse(code)
                                    .expect("a register currency"),
                                quantity: xc_kernel::money::ForeignAmount::from_decimal(quantity),
                                source: SourceRef {
                                    source_type: SourceType::Opening,
                                    source_id: format!("r1-fund:{id}:{code}"),
                                    source_key: "fund".to_string(),
                                },
                                line: None,
                                reverses: None,
                            }],
                            None,
                        )
                        .await
                        .expect("fund the open shift's drawer through the ledger");
                }
                Ok::<(), pos_error::AppError>(())
            })
        })
        .await
        .unwrap_or_else(|e| panic!("open a shift at {branch}: {e}"));
    shift
}
