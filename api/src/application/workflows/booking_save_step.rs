//! Booking part in the save of an exchange (T15-h, design §5.6.3, §5.6.5, BK-7 #1405).

use std::sync::Arc;

use sqlx::{Postgres, Transaction};
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_exchange::domain::exchange::entities::transaction::Transaction as ExchangeTransaction;
use xc_kernel::mode::Mode;

use crate::application::workflows::save_step::{SaveContext, SaveStep, StepFuture};

pub const FORM_MISMATCH: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";

/// Booking save step: completes a booking after the save.
#[derive(Clone)]
pub struct BookingSaveStep {
    lifecycle: Arc<BookingLifecycle>,
}

impl BookingSaveStep {
    pub fn new(lifecycle: Arc<BookingLifecycle>) -> Self {
        Self { lifecycle }
    }
}

impl SaveStep for BookingSaveStep {
    fn vouches_for_rate(&self, _ctx: &SaveContext<'_>) -> bool {
        false
    }

    fn before_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        ctx: &'a SaveContext<'_>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            let Some(claim) = ctx.booking else {
                return Ok(());
            };

            // DEMO save cannot use a booking (design B10, ruling R9)
            if ctx.form.customer.mode != Mode::Live {
                return Err(pos_error::AppError::from(BookingError::BookingNotUsable));
            }

            let prior_tx_id = if let Some(prior) = ctx.prior {
                sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
                    .bind(&prior.document_ref)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(pos_error::AppError::Database)?
            } else {
                None
            };

            self.lifecycle
                .check_usable_in(
                    tx,
                    ctx.branch,
                    &claim.booking_id,
                    &ctx.form.customer_id,
                    prior_tx_id,
                )
                .await?;

            Ok(())
        })
    }

    fn after_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        ctx: &'a SaveContext<'_>,
        saved: &'a ExchangeTransaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            let Some(claim) = ctx.booking else {
                return Ok(());
            };

            let tx_id: uuid::Uuid = sqlx::query_scalar(
                "UPDATE transactions SET booking_id = COALESCE(booking_id, $2) WHERE document_ref = $1 RETURNING id",
            )
            .bind(&saved.document_ref)
            .bind(&claim.booking_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            self.lifecycle
                .complete_in(
                    tx,
                    ctx.branch,
                    &claim.booking_id,
                    &saved.customer_id,
                    tx_id,
                    &ctx.actor.id,
                    ctx.now,
                )
                .await?;

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pos_error::AppError;
    use pos_infra::transactions::Transactor;
    use sqlx::migrate::Migrator;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::{AssertSqlSafe, PgPool};
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Arc;
    use xc_auth::domain::access::StandingKind;
    use xc_booking::infrastructure::booking_repository::PgBookingRepository;
    use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
    use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
    use xc_kernel::branch_tx::BranchTx;
    use xc_kernel::ids::BranchId;
    use xc_shift::application::shift::service::ShiftService;
    use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

    use crate::application::workflows::save_step::{BookingClaim, SaveActor, SaveExtras};
    use crate::application::workflows::save_transaction::{RATE_MISMATCH, SaveTransactionWorkflow};
    use crate::infrastructure::files::file_service::FileService;

    struct TestDb {
        pool: PgPool,
        schema: String,
        scratch: std::path::PathBuf,
    }

    impl TestDb {
        async fn new(prefix: &str) -> Self {
            let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
                panic!("TEST_DATABASE_URL must be set for database tests");
            });
            let options = PgConnectOptions::from_str(&url).expect("invalid TEST_DATABASE_URL");
            let schema = format!(
                "{}_{}",
                prefix,
                uuid::Uuid::new_v4().to_string().replace('-', "")
            );
            let admin = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options.clone())
                .await
                .expect("connect admin");
            sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .expect("create schema");
            admin.close().await;

            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect_with(options.options([("search_path", schema.as_str())]))
                .await
                .expect("connect pool");

            let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
            Migrator::new(migrations.as_path())
                .await
                .expect("migrator")
                .run(&pool)
                .await
                .expect("run migrations");

            let scratch = std::env::temp_dir().join(format!("xc-{}", schema));
            std::fs::create_dir_all(&scratch).expect("create scratch");

            Self {
                pool,
                schema,
                scratch,
            }
        }

        async fn seed_setup(&self) {
            // No counter rate is seeded: since RE-4c the counter rate is the branch's current
            // publication, and MAIN has none here; the `rates` row this used to write was read
            // by nothing and is gone with the table (RE-7 #604, rates.md §8 migration 6).

            // Open shift in MAIN
            let teller_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
                 RETURNING id",
            )
            .fetch_one(&self.pool)
            .await
            .expect("seed the opener's user (CA-1 #617)");

            sqlx::query(
                "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
                 VALUES ('shift-1', 'MAIN', 'OPEN', now(), 'test', 'open-1', $1)",
            )
            .bind(teller_id)
            .execute(&self.pool)
            .await
            .expect("insert shift");

            {
                use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
                use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
                let mut tx = self.pool.begin().await.expect("begin");
                for (code, amount) in [("THB", 1_000_000), ("USD", 10_000)] {
                    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
                        .post_in(
                            &mut tx,
                            &[PostingRequest {
                                from: CashPlace::Bank,
                                to: CashPlace::Drawer("shift-1".to_owned()),
                                currency: xc_kernel::money::CurrencyCode::parse(code)
                                    .expect("currency"),
                                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                                    rust_decimal::Decimal::from(amount),
                                ),
                                source: SourceRef {
                                    source_type: SourceType::Opening,
                                    source_id: format!("r1-fund:shift-1:{code}"),
                                    source_key: "fund".to_string(),
                                },
                                line: None,
                                reverses: None,
                            }],
                            None,
                        )
                        .await
                        .expect("fund shift-1 drawer through the ledger");
                }
                tx.commit().await.expect("commit");
            }

            // Store passport attachment on disk and in database
            let key = "00000000-0000-0000-0000-000000000001";
            let upload_dir = self.scratch.join("uploads");
            std::fs::create_dir_all(&upload_dir).expect("create uploads dir");
            std::fs::write(upload_dir.join(key), b"passport bytes").expect("write passport");
            sqlx::query(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, 'MAIN', 'owner', 'image/png', 'sha256-dummy', now())",
            )
            .bind(key)
            .execute(&self.pool)
            .await
            .expect("insert file");

            // Seed customer in MAIN
            sqlx::query(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 SELECT 'CUST-1', organization_id, 'CUST-1', 'Test Customer', 'LIVE', now() FROM branches WHERE id = 'MAIN'",
            )
            .execute(&self.pool)
            .await
            .expect("insert customer");

            // CI-4 (#651): the customer's passport is an encrypted identity found by its lookup hash (#554 §4.1).
            let keys = std::sync::Arc::new(
                xc_kernel::pii::PiiKeys::parse(
                    "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=",
                )
                .expect("the test PII key parses"),
            );
            xc_customer::install_pii_keys(keys.clone()).expect("one test PII key per process");
            let normalized = xc_kernel::pii::normalize(
                xc_kernel::pii::IdentityType::Passport,
                "P1234567",
                Some("THA"),
            )
            .expect("normalise the passport");
            let hash = xc_kernel::pii::LookupHasher::new(&keys)
                .hash(xc_kernel::pii::IdentityType::Passport, &normalized);
            let sealed = xc_kernel::pii::PiiCipher::new(keys).encrypt(
                b"CUST-1PASSPORT",
                br#"{"number":"P1234567","nationality":"THA","birthDate":"1990-01-01","expiry":"2030-01-01","nameOnDocument":"Test Customer"}"#,
            );
            sqlx::query(
                "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
                 lookup_hash, nationality) \
                 SELECT gen_random_uuid(), organization_id, 'CUST-1', 'PASSPORT', $1, $2, $3, 'THA' \
                 FROM branches WHERE id = 'MAIN'",
            )
            .bind(&sealed.bytes)
            .bind(&sealed.key_id)
            .bind(hash.as_slice())
            .execute(&self.pool)
            .await
            .expect("insert the customer's passport identity");
        }

        fn workflow(&self) -> SaveTransactionWorkflow {
            let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
            let files = FileService::new(self.pool.clone(), self.scratch.join("uploads"));
            let shifts = ShiftService::new(
                Arc::new(PgShiftRepository::new(self.pool.clone())),
                branch_tx.clone(),
            );
            SaveTransactionWorkflow::new(branch_tx, shifts, files)
        }

        fn lifecycle(&self) -> Arc<BookingLifecycle> {
            Arc::new(BookingLifecycle::new(Arc::new(PgBookingRepository::new(
                self.pool.clone(),
            ))))
        }

        async fn teardown(self) {
            let _ = std::fs::remove_dir_all(&self.scratch);
            let _ = sqlx::query(AssertSqlSafe(format!(
                "DROP SCHEMA {} CASCADE",
                self.schema
            )))
            .execute(&self.pool)
            .await;
        }
    }

    #[tokio::test]
    async fn m13_save_as_booked_with_no_vouching_step_keeps_counter_rate_check() {
        let db = TestDb::new("m13").await;
        db.seed_setup().await;

        // Workflow has NO registered steps (no BookingSaveStep to vouch for the rate)
        let workflow = db.workflow();

        let cmd = SaveTransaction {
            customer: ReserveCustomer {
                name: "Test Customer".to_string(),
                passport_no: "P1234567".to_string(),
                nationality: "THA".to_string(),
                birth_date: "1990-01-01".to_string(),
                passport_expiry: "2030-01-01".to_string(),
                mode: "LIVE".to_string(),
            },
            submission_id: "00000000-0000-4000-8000-000000000001".to_string(),
            customer_id: "CUST-1".to_string(),
            side: "BUY".to_string(),
            currency: "USD".to_string(),
            amount: "100".to_string(),
            rate: "32.1".to_string(), // booked rate 32.1: MAIN publishes no counter rate to match it
            fee: "0".to_string(),
            bank_fee: "0".to_string(),
            method: "Cash".to_string(),
            payment_ref: String::new(),
            account: "Cash Drawer".to_string(),
            staff: "owner".to_string(),
            booking: "BK-1".to_string(),
            notes: String::new(),
            value_date: "2026-10-06".to_string(),
            paid_at: "2026-10-06T07:00:00.000Z".to_string(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some("00000000-0000-0000-0000-000000000001".to_string()),
            slip: None,
            reference_rate: None,
        };

        let extras = SaveExtras {
            staff_draft_id: None,
            count_confirmed: false,
            kiosk_request_id: None,
        };
        let actor = SaveActor {
            id: "owner".to_string(),
            username: "owner".to_string(),
            employee_id: None,
            standing: StandingKind::Owner,
        };
        let booking = Some(BookingClaim {
            booking_id: "BK-1".to_string(),
            customer_confirmed: true,
        });

        // Calling save_as_booked with a booking claim but without a vouching step
        // MUST keep the counter-rate check and refuse with RATE_MISMATCH.
        let result = workflow
            .save_as_booked(
                &BranchId::new("MAIN"),
                cmd,
                &extras,
                booking,
                &actor,
                Utc::now(),
            )
            .await;

        match result {
            Err(AppError::Domain(msg)) => assert_eq!(msg, RATE_MISMATCH),
            other => panic!("expected Domain({RATE_MISMATCH}), got {other:?}"),
        }

        db.teardown().await;
    }

    #[tokio::test]
    /// Re-pinned for TX-6 by the test-writer, decision A3 (design quote-transaction.md §4.6.1, #674;
    /// lane controller's ruling, Fact 3 option (i)): the save is quoted before `BookingSaveStep`,
    /// so a currency nobody publishes is refused at the quote, still never a 500.
    async fn unpinned_currency_is_refused_at_the_quote_never_500() {
        let db = TestDb::new("unpinned").await;
        db.seed_setup().await;

        let lifecycle = db.lifecycle();
        let step = Arc::new(BookingSaveStep::new(lifecycle));
        let workflow = db.workflow().with_steps(vec![step]);

        // Form currency passes TransactionForm check (3 uppercase ascii chars "ZZZ")
        // but CurrencyCode::parse("ZZZ") refuses it.
        let cmd = SaveTransaction {
            customer: ReserveCustomer {
                name: "Test Customer".to_string(),
                passport_no: "P1234567".to_string(),
                nationality: "THA".to_string(),
                birth_date: "1990-01-01".to_string(),
                passport_expiry: "2030-01-01".to_string(),
                mode: "LIVE".to_string(),
            },
            submission_id: "00000000-0000-4000-8000-000000000002".to_string(),
            customer_id: "CUST-1".to_string(),
            side: "BUY".to_string(),
            currency: "ZZZ".to_string(),
            amount: "100".to_string(),
            rate: "30.0".to_string(),
            fee: "0".to_string(),
            bank_fee: "0".to_string(),
            method: "Cash".to_string(),
            payment_ref: String::new(),
            account: "Cash Drawer".to_string(),
            staff: "owner".to_string(),
            booking: "BK-1".to_string(),
            notes: String::new(),
            value_date: "2026-10-06".to_string(),
            paid_at: "2026-10-06T07:00:00.000Z".to_string(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some("00000000-0000-0000-0000-000000000001".to_string()),
            slip: None,
            reference_rate: None,
        };

        let extras = SaveExtras {
            staff_draft_id: None,
            count_confirmed: false,
            kiosk_request_id: None,
        };
        let actor = SaveActor {
            id: "owner".to_string(),
            username: "owner".to_string(),
            employee_id: None,
            standing: StandingKind::Owner,
        };
        let booking = Some(BookingClaim {
            booking_id: "BK-1".to_string(),
            customer_confirmed: true,
        });

        let result = workflow
            .save_as_booked(
                &BranchId::new("MAIN"),
                cmd,
                &extras,
                booking,
                &actor,
                Utc::now(),
            )
            .await;

        // Must answer the quote's RATE_MISMATCH domain error (decision A3), NEVER a 500
        match result {
            Err(AppError::Domain(msg)) => assert_eq!(msg, RATE_MISMATCH),
            other => panic!("expected Domain({RATE_MISMATCH}), got {other:?}"),
        }

        db.teardown().await;
    }
}
