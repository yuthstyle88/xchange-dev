//! Booking part in the save of an exchange (T15-h, design §5.6.3, §5.6.5).

use pos_error::AppError;
use sqlx::{Postgres, Transaction};
use xc_exchange::application::booking::service::BookingService;
use xc_exchange::domain::booking::commands::redeem_booking::RedeemBooking;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction as ExchangeTransaction;
use xc_kernel::money::CurrencyCode;

use crate::application::workflows::save_step::{SaveContext, SaveStep, StepFuture};

pub const FORM_MISMATCH: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";

/// Booking save step: redeems a booking before the save and marks it used after.
#[derive(Clone)]
pub struct BookingSaveStep {
    bookings: BookingService,
}

impl BookingSaveStep {
    pub fn new(bookings: BookingService) -> Self {
        Self { bookings }
    }
}

impl SaveStep for BookingSaveStep {
    fn vouches_for_rate(&self, ctx: &SaveContext<'_>) -> bool {
        ctx.booking.is_some()
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
            let Ok(currency) = CurrencyCode::parse(&ctx.form.currency) else {
                return Err(AppError::Domain(FORM_MISMATCH.to_owned()));
            };
            let cmd = RedeemBooking {
                booking_id: claim.booking_id.clone(),
                submission_id: ctx.form.submission_id.clone(),
                live: ctx.form.customer.mode == Mode::Live,
                side: ctx.form.side,
                currency,
                amount: ctx.form.amount,
                rate: ctx.form.client_rate,
                form_booking: ctx.form.booking.clone(),
                customer_id: ctx.form.customer_id.clone(),
                customer_confirmed: claim.customer_confirmed,
            };
            self.bookings
                .redeem_in(tx, ctx.branch, cmd, ctx.now)
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
            if let (None, Some(claim)) = (ctx.prior, ctx.booking) {
                self.bookings
                    .use_in(
                        tx,
                        ctx.branch,
                        &claim.booking_id,
                        &ctx.form.submission_id,
                        &saved.document_ref,
                    )
                    .await?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pos_infra::transactions::Transactor;
    use sqlx::migrate::Migrator;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::{AssertSqlSafe, PgPool};
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Arc;
    use xc_auth::domain::policy::value_objects::role::Role;
    use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
    use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
    use xc_exchange::infrastructure::booking::booking_ids::UuidBookingIds;
    use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
    use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;
    use xc_kernel::branch_tx::BranchTx;
    use xc_kernel::ids::BranchId;
    use xc_shift::application::shift::service::ShiftService;
    use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

    use crate::application::workflows::save_step::{BookingClaim, SaveActor, SaveExtras};
    use crate::application::workflows::save_transaction::{RATE_MISMATCH, SaveTransactionWorkflow};
    use crate::infrastructure::bookings::booking_ports::PgBookingPorts;
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
            sqlx::query(
                "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
                 SELECT id, 'test', 'PENDING', 'test', 'test' FROM organizations WHERE code = 'XC' \
                 ON CONFLICT (legacy_employee_id) DO NOTHING",
            )
            .execute(&self.pool)
            .await
            .expect("seed the opener's user (CA-1 #617)");

            sqlx::query(
                "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
                 VALUES ('shift-1', 'MAIN', 'OPEN', now(), 'test', 'open-1', \
                 (SELECT id FROM users WHERE legacy_employee_id = 'test'))",
            )
            .execute(&self.pool)
            .await
            .expect("insert shift");

            sqlx::query(
                "INSERT INTO shift_balances (shift_id, code, opening, expected) \
                 VALUES ('shift-1', 'THB', 1000000, 1000000), ('shift-1', 'USD', 10000, 10000)",
            )
            .execute(&self.pool)
            .await
            .expect("insert shift balances");

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

        fn booking_service(&self) -> BookingService {
            let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
            let ports = Arc::new(PgBookingPorts::new(self.pool.clone()));
            let bookings = Arc::new(PgBookingRepository::new(self.pool.clone()));
            let members = Arc::new(PgMemberRepository::new(self.pool.clone()));
            let ids = Arc::new(UuidBookingIds);
            BookingService::new(branch_tx, bookings, members, ports, ids)
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
            role: Role::Owner,
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
    async fn unpinned_currency_refuses_with_form_mismatch_never_500() {
        let db = TestDb::new("unpinned").await;
        db.seed_setup().await;

        let booking_service = db.booking_service();
        let step = Arc::new(BookingSaveStep::new(booking_service));
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
            role: Role::Owner,
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

        // Must answer FORM_MISMATCH domain error, NEVER a 500
        match result {
            Err(AppError::Domain(msg)) => assert_eq!(msg, FORM_MISMATCH),
            other => panic!("expected Domain({FORM_MISMATCH}), got {other:?}"),
        }

        db.teardown().await;
    }
}
