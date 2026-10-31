//! C1: `reserveCustomer` with a `bookingId` answers the booking's customer
//! (`docs/design/walkin-queue.md` §12.3, ruled C1 by the owner, relayed on #564 issuecomment-6543033385).
//!
//! Written before the implementation by the ticket author (claude/01TxaRJG). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use xc_customer::{
    BAD_TEXT, BOOKING_NOT_THIS_CUSTOMER, BOOKING_NOT_USABLE, CustomerField, CustomerRefusal,
    CustomerService, Reservation, ReserveCustomer, ReserveError, install_pii_keys,
};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn scanned(passport_no: &str) -> ReserveCustomer {
    ReserveCustomer {
        name: "Booked Customer".to_owned(),
        passport_no: passport_no.to_owned(),
        nationality: "DEU".to_owned(),
        birth_date: "1990-01-02".to_owned(),
        passport_expiry: "2031-06-11".to_owned(),
        mode: "LIVE".to_owned(),
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 9, 0, 0).unwrap()
}

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

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
        let schema = xc_testdb::unique_schema_name("c1bk");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("admin connect");
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
        let mut entries: Vec<_> =
            std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations"))
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
        for id in ["MAIN", "OTHER"] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(id)
            .execute(&pool)
            .await
            .expect("insert branch");
        }
        let _ = install_pii_keys(Arc::new(PiiKeys::parse(TEST_KEYS).expect("test keys")));
        Db {
            pool,
            admin_url: url,
            schema,
        }
    }

    fn service(&self) -> CustomerService {
        CustomerService::new(BranchTx::new(pos_infra::transactions::Transactor::new(
            self.pool.clone(),
        )))
    }

    async fn exec(&self, sql: &str) {
        sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A customer with no identity at all: an app booker registered by phone.
    async fn phone_booker(&self, id: &str) {
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT '{id}', organization_id, '{id}', 'Phone Booker', 'LIVE', now() FROM branches WHERE id = 'MAIN'"
        ))
        .await;
    }

    /// A customer with `passport_no` on file, made the way the desk makes one today (no booking).
    async fn passport_holder(&self, passport_no: &str) -> String {
        let made = self
            .service()
            .reserve_checked(&main_branch(), scanned(passport_no), now())
            .await
            .expect("today's reserve makes the customer");
        assert!(!made.existing, "{passport_no} was not on file yet");
        made.customer_id
    }

    /// A booking of `customer_id` at `branch` in `status`.
    async fn booking(&self, id: &str, branch: &str, customer_id: &str, status: &str) {
        let checked_in = if status == "CONFIRMED" {
            "NULL"
        } else {
            "now()"
        };
        self.exec(&format!(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
                 capacity, reserved, status) \
             SELECT gen_random_uuid(), organization_id, id, current_date, \
                 now() + make_interval(mins => (SELECT count(*)::int FROM booking_slots) * 30 + 60), \
                 now() + make_interval(mins => (SELECT count(*)::int FROM booking_slots) * 30 + 90), \
                 4, 1, 'OPEN' FROM branches WHERE id = '{branch}'"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
                 requested_currency, requested_quantity, source_channel, status, qr_token_hash, checked_in_at, \
                 created_by) \
             SELECT '{id}', s.organization_id, s.branch_id, 'B-{id}', '{customer_id}', s.id, 'BUY', 'USD', 100, \
                 'APP', '{status}', convert_to('qr-{id}', 'UTF8'), {checked_in}, 'test' \
             FROM booking_slots s WHERE s.branch_id = '{branch}' ORDER BY s.created_at DESC, s.start_at DESC LIMIT 1"
        ))
        .await;
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn customers(&self) -> i64 {
        self.count("SELECT count(*) FROM customers").await
    }

    async fn passports_of(&self, customer_id: &str) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM customer_identities \
             WHERE customer_id = '{customer_id}' AND type = 'PASSPORT' AND status = 'ACTIVE'"
        ))
        .await
    }

    async fn identities(&self) -> i64 {
        self.count("SELECT count(*) FROM customer_identities").await
    }

    async fn stop(self) {
        self.pool.close().await;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.admin_url)
            .await
            .expect("admin connect");
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop schema");
        admin.close().await;
    }
}

fn refused_on_passport(result: Result<Reservation, ReserveError>, what: &str) {
    match result {
        Err(ReserveError::Input(refusal)) => assert_eq!(
            refusal,
            CustomerRefusal {
                field: CustomerField::PassportNo,
                sentence: BOOKING_NOT_THIS_CUSTOMER
            },
            "{what}"
        ),
        other => {
            panic!("{what}: expected Input(PassportNo, BOOKING_NOT_THIS_CUSTOMER), got {other:?}")
        }
    }
}

fn refused_unusable(result: Result<Reservation, ReserveError>, what: &str) {
    match result {
        Err(ReserveError::Other(pos_error::AppError::Domain(sentence))) => {
            assert_eq!(sentence, BOOKING_NOT_USABLE, "{what}")
        }
        other => panic!("{what}: expected Other(Domain(BOOKING_NOT_USABLE)), got {other:?}"),
    }
}

// C1 row 1: the booker has no passport on file ⇒ the scanned one is attached to the booker.
#[test]
fn a_booker_with_no_passport_gets_the_scanned_one() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        db.phone_booker("CU-APP-1").await;
        db.booking("BK-1", "MAIN", "CU-APP-1", "IN_SERVICE").await;
        let customers = db.customers().await;

        let answer = db
            .service()
            .reserve_for_booking(&main_branch(), scanned("P2000001"), "BK-1", now())
            .await
            .expect("the booker is answered");
        assert_eq!(
            answer,
            Reservation {
                customer_id: "CU-APP-1".to_owned(),
                existing: true
            }
        );
        assert_eq!(db.customers().await, customers, "no customer is made");
        assert_eq!(
            db.passports_of("CU-APP-1").await,
            1,
            "the passport is on the booker"
        );

        // The attached passport is the one today's lookup finds.
        let again = db
            .service()
            .reserve_checked(&main_branch(), scanned("P2000001"), now())
            .await
            .expect("today's reserve");
        assert_eq!(
            again,
            Reservation {
                customer_id: "CU-APP-1".to_owned(),
                existing: true
            }
        );
        db.stop().await;
    });
}

// C1 row 1, CHECKED_IN: a booking checked in but not yet started is usable too.
#[test]
fn a_checked_in_booking_is_usable() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        db.phone_booker("CU-APP-1").await;
        db.booking("BK-1", "MAIN", "CU-APP-1", "CHECKED_IN").await;
        let answer = db
            .service()
            .reserve_for_booking(&main_branch(), scanned("P2000001"), "BK-1", now())
            .await
            .expect("the booker is answered");
        assert_eq!(answer.customer_id, "CU-APP-1");
        assert_eq!(db.passports_of("CU-APP-1").await, 1);
        db.stop().await;
    });
}

// C1 row 2: the booker's passport on file equals the scanned one ⇒ accepted, nothing written.
#[test]
fn the_same_passport_as_on_file_is_accepted() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        let holder = db.passport_holder("P2000001").await;
        db.booking("BK-1", "MAIN", &holder, "IN_SERVICE").await;
        let (customers, identities) = (db.customers().await, db.identities().await);

        let answer = db
            .service()
            .reserve_for_booking(&main_branch(), scanned("p2000001"), "BK-1", now())
            .await
            .expect("the booker is answered");
        assert_eq!(
            answer,
            Reservation {
                customer_id: holder.clone(),
                existing: true
            }
        );
        assert_eq!(db.customers().await, customers);
        assert_eq!(db.identities().await, identities, "no identity is added");
        db.stop().await;
    });
}

// C1 row 3: the scanned passport is another customer's ⇒ refused, nothing written.
#[test]
fn another_customers_passport_is_refused() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        let other = db.passport_holder("P2000001").await;
        db.phone_booker("CU-APP-1").await;
        db.booking("BK-1", "MAIN", "CU-APP-1", "IN_SERVICE").await;
        let (customers, identities) = (db.customers().await, db.identities().await);

        let result = db
            .service()
            .reserve_for_booking(&main_branch(), scanned("P2000001"), "BK-1", now())
            .await;
        refused_on_passport(result, "P2000001 is on another customer");
        assert_eq!(db.customers().await, customers);
        assert_eq!(db.identities().await, identities);
        assert_eq!(
            db.passports_of("CU-APP-1").await,
            0,
            "the booker gets nothing"
        );
        assert_eq!(
            db.passports_of(&other).await,
            1,
            "the other customer keeps theirs"
        );
        db.stop().await;
    });
}

// C1 row 4: the booker has a different passport on file ⇒ refused, nothing written.
#[test]
fn a_passport_other_than_the_one_on_file_is_refused() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        let holder = db.passport_holder("P2000001").await;
        db.booking("BK-1", "MAIN", &holder, "IN_SERVICE").await;
        let (customers, identities) = (db.customers().await, db.identities().await);

        let result = db
            .service()
            .reserve_for_booking(&main_branch(), scanned("P2000002"), "BK-1", now())
            .await;
        refused_on_passport(result, "the booker holds P2000001");
        assert_eq!(
            db.customers().await,
            customers,
            "no customer is made for P2000002"
        );
        assert_eq!(db.identities().await, identities);
        assert_eq!(db.passports_of(&holder).await, 1);
        db.stop().await;
    });
}

// C1 row 5: no such booking, another branch's, or one not CHECKED_IN / IN_SERVICE ⇒ BOOKING_NOT_USABLE.
#[test]
fn a_booking_the_desk_cannot_use_is_refused() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        db.phone_booker("CU-APP-1").await;
        db.booking("BK-OTHER", "OTHER", "CU-APP-1", "IN_SERVICE")
            .await;
        db.booking("BK-CONF", "MAIN", "CU-APP-1", "CONFIRMED").await;
        let (customers, identities) = (db.customers().await, db.identities().await);

        for (booking, what) in [
            ("BK-NONE", "no such booking"),
            ("BK-OTHER", "another branch's booking"),
            ("BK-CONF", "a booking not checked in"),
        ] {
            let result = db
                .service()
                .reserve_for_booking(&main_branch(), scanned("P2000001"), booking, now())
                .await;
            refused_unusable(result, what);
        }
        assert_eq!(db.customers().await, customers);
        assert_eq!(db.identities().await, identities);
        db.stop().await;
    });
}

// C1 row 6: the details are checked first, as today, and name their field.
#[test]
fn bad_details_are_refused_before_the_booking_is_read() {
    sqlx::test_block_on(async {
        let db = Db::start().await;
        let result = db
            .service()
            .reserve_for_booking(&main_branch(), scanned("---"), "BK-NONE", now())
            .await;
        match result {
            Err(ReserveError::Input(refusal)) => assert_eq!(
                refusal,
                CustomerRefusal {
                    field: CustomerField::PassportNo,
                    sentence: BAD_TEXT
                }
            ),
            other => panic!("expected Input(PassportNo, BAD_TEXT), got {other:?}"),
        }
        assert_eq!(db.identities().await, 0);
        db.stop().await;
    });
}
