//! Ticket #672 (TX-4) — `CreateTransaction::run_in` against a real PostgreSQL: a DRAFT is made
//! from an ACTIVE quote of this branch, every figure comes from the quote, and each refusal of
//! design §4.4 writes nothing.
//! Written before the implementation (claude/14cc24bd, on the owner's order, 2026-10-10). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §2 D-3 (`document_ref` NULL until
//! COMPLETED), D-5 (validity), D-9 (one payment), §4.2 (`POST /api/transactions` body: no figure is
//! read from it), §4.4 `POST /api/transactions` steps 1–4, §7. Ticket #672 G6 (create reads the
//! customer by `customerId`, taking its **mode**), §9 (*"Create refuses an expired quote
//! (`quoteExpired`) and another branch's quote (`outOfScope`)"*).
//!
//! Not locked here, on purpose:
//! - how create reads the customer beyond what is asserted here: the lane controller's ruling Q1 = B
//!   (#672 issuecomment-6094343896) has create use the non-decrypting
//!   `CustomerService::exchange_customer_in` instead of `confirm_in`; the read itself is locked in
//!   `tx4_customer_read_test.rs`;
//! - the shift rule beyond "LIVE needs the branch's open shift" as main has it today: CA-3 #619
//!   moves shifts to one per teller, and these two tests are re-expected by the ticket author if it
//!   lands first.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Every call is committed whatever it answers, and `transactions` is counted.
//!
//! Signatures this test assumes ([required] in ticket §7 unless marked "choice"):
//! - `xchange::application::workflows::exchange::create::{CreateTransaction, CreateRequest,
//!   CreatePayment}`:
//!   - choice: `CreateTransaction::new(pool: PgPool, files: FileService)`;
//!   - `run_in(&self, tx, req: CreateRequest, who: &SaveActor, now) ->
//!     AppResult<ExchangeTransaction>` (async), answering the stored DRAFT;
//!   - choice: `CreateRequest { branch_id: BranchId, quote_id: Uuid, customer_id: String,
//!     booking_id: Option<String>, kiosk_request_id: Option<String>, staff_draft_id:
//!     Option<String>, passport_key: String, payment: CreatePayment, value_date: NaiveDate, notes:
//!     String, count_confirmed: bool }` (§4.2's fields; `branch_id` is the command seam's branch);
//!   - choice: `CreatePayment { method: PaymentMethod, reference: String, account: String,
//!     slip_key: Option<String>, bank_fee_thb: Money, paid_at: DateTime<Utc> }`.
//! - Refusals as `Err(AppError::Domain(m))`: codes first for `quoteExpired`, `outOfScope`,
//!   `staleVersion` (choice: the quote is not ACTIVE), `validationFailed` (choice: the quote names
//!   another customer); today's sentences, unchanged, for the shift and the files (§4.4 steps 2–3
//!   *"as today"*, `save_transaction.rs`).
//! - G7 (moved here from TX-7 #675, which merged as `895f151`, migration
//!   `20261031100000_kiosk_quote_id.sql`): with `kioskRequestId`, create reads the request's
//!   `kiosk_requests.quote_id` at the request's branch. **The check lives in create** (ticket author's
//!   choice, the ticket left it open): a body `quoteId` that differs from a non-NULL `quote_id` is
//!   refused with code `validationFailed` and nothing is written. A request with `quote_id NULL`
//!   (quoted before TX-7) is not checked at create; the kiosk step's existing rule applies at confirm.
//! - #672 Q2/Q3 (lane controller, issuecomment-6094343896): create stores the body's passport key as
//!   `passport_file_key` and its `kioskRequestId`, `staffDraftId` and `countConfirmed` on the exchange.
//! - The stored payment's `amount_thb` is the line's `settle_thb` (the THB that changes hands, as
//!   #670's `tx_store_test.rs` draft has it).

mod quote_common;
mod tx4_common;

use std::sync::{Arc, Once};

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use quote_common::*;
use tx4_common::*;
use uuid::Uuid;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionStatus};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::Side;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::workflows::exchange::create::{
    CreatePayment, CreateRequest, CreateTransaction,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::infrastructure::files::file_service::FileService;

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];
const PDF: &[u8] = b"%PDF-1.7 a slip\n";
/// Today's sentences (`save_common`): no open shift, and a file that is not this branch's.
const NO_OPEN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
const ATTACHMENT_NOT_FOUND: &str = "ไม่พบไฟล์แนบของสาขานี้";
/// Today's sentence (`xc-customer` `NOT_THIS_PASSPORT`), narrowed by Q1 = B to "no active passport".
const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";
/// CI-4 (#651): the test PII key, as `save_common::TEST_PII_KEYS`.
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

struct Counter {
    desk: Desk,
    files: FileService,
    customers: CustomerService,
    shifts: ShiftService,
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
        let scratch = std::env::temp_dir().join(unique("xc-tx4-create"));
        std::fs::create_dir_all(&scratch).unwrap();
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        Self {
            files: FileService::new(pool.clone(), scratch.join("uploads")),
            customers: CustomerService::new(branch_tx.clone()),
            shifts: ShiftService::new(Arc::new(PgShiftRepository::new(pool.clone())), branch_tx),
            desk,
            scratch,
        }
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.desk.db.finish().await;
    }

    /// A customer reserved at `branch` through CI-4's service, in `mode`.
    async fn customer(&self, branch: &str, passport: &str, mode: &str) -> String {
        self.customers
            .reserve(
                &BranchId::new(branch),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: passport.to_owned(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: mode.to_owned(),
                },
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("reserve: {e}"))
            .customer_id
    }

    /// A file of the teller at `branch`.
    async fn upload(&self, branch: &str, mime: &str, bytes: &[u8]) -> String {
        self.files
            .upload(
                &BranchId::new(branch),
                TELLER,
                mime,
                bytes,
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("upload: {e}"))
    }

    /// #2964: a file uploaded at `branch` by `who` (a `users.id`), so a save as `who` may attach it.
    async fn upload_as(&self, branch: &str, who: &str, mime: &str, bytes: &[u8]) -> String {
        self.files
            .upload(&BranchId::new(branch), who, mime, bytes, whole_second_now())
            .await
            .unwrap_or_else(|e| panic!("upload: {e}"))
    }

    /// #2964: the teller of the shifts `open_shift` opens (the `test` user), as a v3 session carries them: `users.id`.
    async fn shift_teller(&self) -> SaveActor {
        let id: String =
            sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = 'test'")
                .fetch_one(&self.desk.db.pool)
                .await
                .expect("the shift's teller (open_shift seeds it)");
        SaveActor {
            id,
            username: "test".to_owned(),
            employee_id: None,
            role: xc_auth::domain::policy::value_objects::role::Role::Front,
        }
    }

    /// #2964: `run_in` as `who` (the other helpers run it as the non-UUID `teller()`).
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

    /// #2964: an open shift of the `test` teller at `branch`.
    async fn open_shift_at(&self, branch: &str) -> String {
        sqlx::query(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             SELECT id, 'test', 'PENDING', 'test', 'test' FROM organizations WHERE code = 'XC' \
             ON CONFLICT (legacy_employee_id) DO NOTHING",
        )
        .execute(&self.desk.db.pool)
        .await
        .expect("seed the opener");
        self.shifts
            .open(
                &BranchId::new(branch),
                OpenShift {
                    request_id: unique("open-shift").replace('_', "-"),
                    opening: Balances::of(&[]),
                },
                "test",
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("open a shift at {branch}: {e}"))
            .id
    }

    /// Open MAIN's shift as main does today; answers its id.
    async fn open_shift(&self) -> String {
        sqlx::query(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             SELECT id, 'test', 'PENDING', 'test', 'test' FROM organizations WHERE code = 'XC' \
             ON CONFLICT (legacy_employee_id) DO NOTHING",
        )
        .execute(&self.desk.db.pool)
        .await
        .expect("seed the opener");
        self.shifts
            .open(
                &BranchId::new("MAIN"),
                OpenShift {
                    request_id: unique("open-shift").replace('_', "-"),
                    opening: Balances::of(&[]),
                },
                "test",
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("open a shift: {e}"))
            .id
    }

    fn request(&self, branch: &str, q: &Quote, customer: &str, passport: &str) -> CreateRequest {
        CreateRequest {
            branch_id: BranchId::new(branch),
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
            value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
            notes: "a note".to_owned(),
            count_confirmed: true,
        }
    }

    async fn create(
        &self,
        req: CreateRequest,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        let service = CreateTransaction::new(self.desk.db.pool.clone(), self.files.clone());
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, req, &teller(), now).await;
        tx.commit().await.unwrap();
        got
    }

    async fn transactions(&self) -> i64 {
        self.desk
            .db
            .count("SELECT count(*) FROM transactions")
            .await
    }
}

fn sentence<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

#[tokio::test]
async fn a_draft_is_made_from_the_quote_with_every_figure_from_the_quote() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let slip = c.upload("MAIN", "application/pdf", PDF).await;
    let q = c
        .desk
        .quote("MAIN", Side::Buy, "100", Some("10.00"), now)
        .await;
    let mut req = c.request("MAIN", &q, &customer, &passport);
    req.staff_draft_id = Some("draft-7".to_owned());
    req.payment.method = PaymentMethod::BankTransfer;
    req.payment.slip_key = Some(slip.clone());
    req.payment.bank_fee_thb = money("1.50");

    let made = c.create(req, later(now, 5)).await.expect("a draft");
    let stored = c.desk.stored(made.id).await;
    assert_eq!(made, stored, "create answers the DRAFT as stored");

    assert_eq!(stored.status, TransactionStatus::Draft);
    assert_eq!(stored.document_ref, None, "D-3");
    assert_eq!(stored.quote_id, Some(q.id));
    assert_eq!(stored.organization_id, q.organization_id);
    assert_eq!(stored.branch_id.as_str(), "MAIN");
    assert_eq!(stored.mode, Mode::Demo, "G6: the customer row's mode");
    assert_eq!(stored.shift_id, None, "DEMO: no shift");
    assert_eq!(stored.customer_id, customer);
    assert_eq!(
        stored.customer_name, "Somchai Jaidee",
        "Q1 = B: the customer's display name"
    );
    assert_eq!(
        stored.nationality, "THA",
        "Q1 = B: the ACTIVE passport's clear nationality"
    );
    assert_eq!(
        stored.value_date,
        NaiveDate::from_ymd_opt(2026, 10, 6).unwrap()
    );
    assert_eq!(stored.notes, "a note");
    assert_eq!(stored.recorded_by, TELLER, "the identity `SaveActor::id`");
    assert_eq!(stored.recorded_at, later(now, 5));
    assert_eq!(stored.approval_request_id, None);
    assert!(stored.receipts.is_empty());
    assert_eq!(
        stored.passport_file_key, passport,
        "Q2: the passport scan is the exchange's, stored at create"
    );
    assert_eq!(
        (
            stored.kiosk_request_id.as_deref(),
            stored.staff_draft_id.as_deref(),
            stored.count_confirmed
        ),
        (None, Some("draft-7"), true),
        "Q3: the save extras of the body are stored at create (confirm and complete rebuild them)"
    );

    assert_eq!(stored.lines.len(), 1);
    let (line, quoted) = (&stored.lines[0], &q.lines[0]);
    assert_eq!(
        line.quote_line_id,
        Some(quoted.id),
        "§4.4 step 4: copied from the quote line"
    );
    assert_eq!(line.shop_side, quoted.shop_side);
    assert_eq!(line.currency, quoted.currency);
    assert_eq!(line.quantity, quoted.quantity);
    assert_eq!(line.unit_size, quoted.unit_size);
    assert_eq!(line.rate_snapshot, quoted.final_rate);
    assert_eq!(line.thb_amount, money("3210.00"));
    assert_eq!(line.fee_thb, money("10.00"));
    assert_eq!(line.settle_thb, money("3200.00"));
    assert_eq!(line.line_no, 1);

    let p = &stored.payment;
    assert_eq!(p.payment_no, 1, "D-9");
    assert_eq!(p.method, PaymentMethod::BankTransfer);
    assert_eq!(p.reference, "R-1");
    assert_eq!(p.account, "Cash drawer 1");
    assert_eq!(p.slip_file_key.as_deref(), Some(slip.as_str()));
    assert_eq!(p.amount_thb, money("3200.00"), "the THB that changes hands");
    assert_eq!(p.bank_fee_thb, money("1.50"));
    assert_eq!(p.paid_at, q.created_at);

    assert_eq!(
        c.desk.quote_row(&q).await,
        "ACTIVE|",
        "§4.4: the quote is accepted at confirm, not at create"
    );
    c.finish().await;
}

#[tokio::test]
async fn a_quote_past_its_expiry_or_its_publication_is_refused_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let expired = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;

    assert_eq!(
        code(
            c.create(
                c.request("MAIN", &expired, &customer, &passport),
                expired.expires_at
            )
            .await
        ),
        "quoteExpired",
        "D-5: now < expires_at"
    );

    let superseded = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    c.desk.db.publish("MAIN", &[USD]).await;
    assert_eq!(
        code(
            c.create(
                c.request("MAIN", &superseded, &customer, &passport),
                later(now, 5)
            )
            .await
        ),
        "quoteExpired",
        "D-5: the branch has published since"
    );
    assert_eq!(c.transactions().await, 0);
    c.finish().await;
}

#[tokio::test]
async fn another_branchs_quote_is_out_of_scope_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let theirs = c.desk.quote(BR2, Side::Buy, "100", None, now).await;

    assert_eq!(
        code(
            c.create(
                c.request("MAIN", &theirs, &customer, &passport),
                later(now, 5)
            )
            .await
        ),
        "outOfScope",
        "§9: a BR2 quote at MAIN"
    );
    assert_eq!(c.transactions().await, 0);
    assert_eq!(c.desk.quote_row(&theirs).await, "ACTIVE|");
    c.finish().await;
}

#[tokio::test]
async fn a_quote_that_is_not_active_is_refused_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let cancelled = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    c.desk
        .db
        .exec(&format!(
            "UPDATE quotes SET status = 'CANCELLED' WHERE id = '{}'",
            cancelled.id
        ))
        .await;
    let (accepted, first) = c.desk.main_draft(now).await;
    c.desk.confirm(first.id, later(now, 5)).await.unwrap();

    for q in [&cancelled, &accepted] {
        assert_eq!(
            code(
                c.create(c.request("MAIN", q, &customer, &passport), later(now, 10))
                    .await
            ),
            "staleVersion",
            "§4.4 step 1: the quote is ACTIVE; an accepted quote makes no second exchange"
        );
    }
    assert_eq!(c.transactions().await, 1, "only the first exchange");
    c.finish().await;
}

#[tokio::test]
async fn a_quote_for_another_customer_is_refused_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let named = c.customer("MAIN", "AA1234567", "DEMO").await;
    let other = c.customer("MAIN", "BB7654321", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let mut tx = c.desk.db.pool.begin().await.unwrap();
    let q =
        xchange::application::workflows::exchange::quote::QuoteService::new(c.desk.db.pool.clone())
            .quote_in(
                &mut tx,
                xchange::application::workflows::exchange::quote::QuoteRequest {
                    branch_id: BranchId::new("MAIN"),
                    customer_id: Some(named.clone()),
                    channel: xc_exchange::domain::quote::quote::QuoteChannel::Desk,
                    lines: vec![
                        xchange::application::workflows::exchange::quote::QuoteRequestLine {
                            currency: xc_kernel::money::CurrencyCode::parse("USD").unwrap(),
                            shop_side: Side::Buy,
                            quantity: xc_kernel::money::ForeignAmount::from_decimal(100.into()),
                            fee_thb: None,
                            denomination_group:
                                xc_rate::domain::currency::denomination_groups::GroupCode::all(),
                        },
                    ],
                },
                &teller(),
                now,
            )
            .await
            .expect("a quote for a named customer");
    tx.commit().await.unwrap();

    assert_eq!(
        code(
            c.create(c.request("MAIN", &q, &other, &passport), later(now, 5))
                .await
        ),
        "validationFailed",
        "§4.4 step 1: a quote that names a customer is that customer's"
    );
    assert_eq!(c.transactions().await, 0);
    let made = c
        .create(c.request("MAIN", &q, &named, &passport), later(now, 5))
        .await
        .expect("the named customer's own exchange");
    assert_eq!(made.customer_id, named);
    c.finish().await;
}

/// #2964 (restated): a LIVE customer needs the saver's **own** OPEN shift at the branch. With no shift ⇒ refused; with
/// another teller's open shift ⇒ still refused (no fallback to "the branch's shift", #626 Q4), nothing written; as that
/// shift's own teller (`users.id`) ⇒ a LIVE draft on it.
#[tokio::test]
async fn a_live_customer_needs_the_tellers_own_open_shift() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "LIVE").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;

    assert_eq!(
        sentence(
            c.create(c.request("MAIN", &q, &customer, &passport), later(now, 5))
                .await
        ),
        NO_OPEN_SHIFT,
        "§4.4 step 2: LIVE needs an open shift, refused as today"
    );
    assert_eq!(c.transactions().await, 0);

    let shift = c.open_shift().await;
    assert_eq!(
        sentence(
            c.create(c.request("MAIN", &q, &customer, &passport), later(now, 5))
                .await
        ),
        NO_OPEN_SHIFT,
        "#2964: a saver who owns no shift is refused beside another teller's open shift"
    );
    assert_eq!(c.transactions().await, 0, "nothing written");

    let owner = c.shift_teller().await;
    let own_passport = c.upload_as("MAIN", &owner.id, "image/jpeg", JPEG).await;
    let made = c
        .create_as(
            c.request("MAIN", &q, &customer, &own_passport),
            &owner,
            later(now, 5),
        )
        .await
        .expect("a LIVE draft on the teller's own shift");
    assert_eq!(made.mode, Mode::Live, "G6: the customer row's mode");
    assert_eq!(
        made.shift_id.as_deref(),
        Some(shift.as_str()),
        "shift_id = the saver's own open shift"
    );
    c.finish().await;
}

/// #2964: the saver's own OPEN shift at another branch is not used: a LIVE save at MAIN is refused, nothing written.
#[tokio::test]
async fn a_tellers_open_shift_at_another_branch_is_not_used() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "LIVE").await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;

    c.open_shift_at(BR2).await;
    let owner = c.shift_teller().await;
    let own_passport = c.upload_as("MAIN", &owner.id, "image/jpeg", JPEG).await;
    assert_eq!(
        sentence(
            c.create_as(
                c.request("MAIN", &q, &customer, &own_passport),
                &owner,
                later(now, 5),
            )
            .await
        ),
        NO_OPEN_SHIFT,
        "#2964: the own shift must be at the request's branch"
    );
    assert_eq!(c.transactions().await, 0);
    c.finish().await;
}

#[tokio::test]
async fn a_passport_file_that_is_not_the_tellers_at_this_branch_is_refused() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let elsewhere = c.upload(BR2, "image/jpeg", JPEG).await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;

    for key in [elsewhere, Uuid::new_v4().to_string()] {
        assert_eq!(
            sentence(
                c.create(c.request("MAIN", &q, &customer, &key), later(now, 5))
                    .await
            ),
            ATTACHMENT_NOT_FOUND,
            "§4.4 step 3: files as today"
        );
    }
    assert_eq!(c.transactions().await, 0);
    c.finish().await;
}

/// Q1 = B: create refuses a customer the non-decrypting read refuses, and writes nothing.
#[tokio::test]
async fn a_customer_without_an_active_passport_is_refused_and_nothing_is_written() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    c.desk
        .db
        .exec(&format!(
            "UPDATE customer_identities SET status = 'REVOKED' WHERE customer_id = '{customer}'"
        ))
        .await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;

    assert_eq!(
        sentence(
            c.create(c.request("MAIN", &q, &customer, &passport), later(now, 5))
                .await
        ),
        NOT_THIS_PASSPORT
    );
    assert_eq!(c.transactions().await, 0);
    c.finish().await;
}

// ---- G7: the kiosk request's quote ----

/// A kiosk request at MAIN, ID-checked by the teller, for `q` (its line and figures), with `quote_id`.
async fn kiosk_request(c: &Counter, q: &Quote, quote_id: Option<Uuid>) -> String {
    let id = unique("KQ");
    let line = &q.lines[0];
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
             rate, foreign_amount, thb, input_amount, payment_method, client_ip, unit_size, \
             published_rate_line_id, quote_id) \
         VALUES ('MAIN', $1, 'ID_CHECKED', now(), $2, 'BUY', 'USD', 32.10, 100, 3210, 100, 'Cash', \
             '203.0.113.7', 1, $3, $4)",
    )
    .bind(&id)
    .bind(TELLER)
    .bind(line.published_rate_line_id)
    .bind(quote_id)
    .execute(&c.desk.db.pool)
    .await
    .expect("seed a kiosk request");
    id
}

#[tokio::test]
async fn a_kiosk_request_whose_quote_is_another_refuses_create_and_writes_nothing() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let the_kiosks = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let another = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let request = kiosk_request(&c, &the_kiosks, Some(the_kiosks.id)).await;

    let mut req = c.request("MAIN", &another, &customer, &passport);
    req.kiosk_request_id = Some(request);
    assert_eq!(
        code(c.create(req, later(now, 5)).await),
        "validationFailed",
        "G7: the body's quoteId must be the kiosk request's quote_id"
    );
    assert_eq!(c.transactions().await, 0);
    assert_eq!(c.desk.quote_row(&another).await, "ACTIVE|");
    c.finish().await;
}

#[tokio::test]
async fn a_kiosk_request_with_its_own_quote_creates_and_the_draft_names_the_request() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let request = kiosk_request(&c, &q, Some(q.id)).await;

    let mut req = c.request("MAIN", &q, &customer, &passport);
    req.kiosk_request_id = Some(request.clone());
    let made = c
        .create(req, later(now, 5))
        .await
        .expect("the kiosk's own quote");
    assert_eq!(made.quote_id, Some(q.id));
    assert_eq!(
        made.kiosk_request_id.as_deref(),
        Some(request.as_str()),
        "Q3: stored at create"
    );
    c.finish().await;
}

/// A request quoted before TX-7 has no `quote_id`: create does not check it (the kiosk step's
/// pre-TX-7 rule applies at confirm, unchanged).
#[tokio::test]
async fn a_kiosk_request_without_a_quote_id_is_not_checked_at_create() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let customer = c.customer("MAIN", "AA1234567", "DEMO").await;
    let passport = c.upload("MAIN", "image/jpeg", JPEG).await;
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let request = kiosk_request(&c, &q, None).await;

    let mut req = c.request("MAIN", &q, &customer, &passport);
    req.kiosk_request_id = Some(request.clone());
    let made = c
        .create(req, later(now, 5))
        .await
        .expect("no quote_id: nothing to match");
    assert_eq!(made.kiosk_request_id.as_deref(), Some(request.as_str()));
    c.finish().await;
}
