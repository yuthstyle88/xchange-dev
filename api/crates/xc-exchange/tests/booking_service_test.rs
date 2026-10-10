//! Written before the implementation (ticket T15-c, #409). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! `BookingService` and its five use-case files, design `docs/design/xchange-v3.md` §5.6.3 (the
//! action table and *Using a booking*), §5.6.4 (*Per branch everywhere*), §5.6.5 (`BookingService`,
//! `BookingReads`, `BookingCreate`, `BookingChanges`, `BookingMembers`, `BookingRedeem`) and
//! §5.6.8 (rows marked T15-c), and the ticket's §7–§9.
//!
//! What is pinned here:
//! - Every action of a branch the ports do not know answers `ไม่พบสาขาที่เลือก กรุณารีเฟรช`
//!   before anything else.
//! - `create`: a non-member's booking is stored CONFIRMED until now + the branch's minutes (30
//!   when the branch has no rules); a member's is stored PENDING with the member and no expiry
//!   (owner 2026-10-08). The rate is the counter rate. The id is `BookingIds::booking_id()`. A
//!   repeated `requestId` with the same payload answers the stored booking and checks nothing
//!   else; another payload answers `CHANGED`. Refusal 8 needs a LIVE customer of the branch with
//!   a member row of the branch.
//! - `list`/`members`: newest booking first (`created_at` desc, then `id` desc), members in
//!   `verified_at`, `id` order, the rules of the branch or none; more than 1,000 bookings refused,
//!   1,000 listed. `get` answers the booking and its member's customer id.
//! - `confirm`/`cancel`: found by (branch, id) only; the counter rate of the booking's branch,
//!   currency and side is locked with the sent expiry; a second confirm with the same expiry and a
//!   cancel after a cancel write nothing; a refusal writes nothing.
//! - `issue_member`: rule 1 (a LIVE customer of the branch and `identity_confirmed`), the
//!   customer's name, the SHA-256 of the secret; a re-issue keeps the member id; a code another
//!   member holds answers `รหัสสมาชิกไม่ถูกต้อง` and writes nothing. That refusal only rolls the
//!   transaction back: nothing is retried inside it (controller, #409, after the #408 review).
//! - `redeem_in`/`use_in` run on the caller's transaction: rolled back, nothing stays.
//! - Every write is one `BranchTx::run` of its branch: that branch's version rises by exactly
//!   one, every other branch's not at all, and a refusal leaves it where it was.
//!
//! The signatures the tests assume (the ticket's §7 names them; the types are fixed here):
//!
//! ```text
//! // application/booking/service.rs
//! BookingService::new(BranchTx, Arc<dyn BookingRepository>, Arc<dyn MemberRepository>,
//!                     Arc<dyn BookingPorts>, Arc<dyn BookingIds>) -> BookingService
//! branch_exists(&self, &BranchId) -> AppResult<bool>
//! // application/booking/list.rs — both derive at least Debug (a refusal is checked through
//! // `common::refusal`, which prints what it got)
//! pub struct BookingList { pub bookings: Vec<Booking>, pub members: Vec<Member>,
//!                          pub rules: Option<BookingRules> }
//! pub struct BookingView { pub booking: Booking, pub customer_id: Option<String> }
//! list(&self, &BranchId, now: DateTime<Utc>) -> AppResult<BookingList>
//! get(&self, &BranchId, id: &str) -> AppResult<BookingView>
//! // create.rs
//! create(&self, &BranchId, CreateBooking, actor: &str, now) -> AppResult<Booking>
//! // change.rs
//! confirm(&self, &BranchId, ConfirmBooking, actor: &str, now) -> AppResult<()>
//! cancel(&self, &BranchId, id: &str) -> AppResult<()>
//! // members.rs
//! issue_member(&self, &BranchId, IssueMember, actor: &str, now) -> AppResult<Member>
//! members(&self, &BranchId) -> AppResult<Vec<Member>>
//! // redeem.rs
//! redeem_in(&self, &mut Transaction<'_, Postgres>, &BranchId, RedeemBooking, now)
//!     -> AppResult<RedeemStep>
//! use_in(&self, &mut Transaction<'_, Postgres>, &BranchId, id: &str, submission_id: &str,
//!        document_ref: &str) -> AppResult<()>
//! ```
//!
//! A refusal is `AppError::Domain(<v2 sentence>)`, as every service of the crate answers one.
//! Commands arrive parsed: refusals 1–6 of `create`, 3's unreadable time of `confirm` and 2–4 of
//! `member` belong to whoever parses the payload, not to this service.
//!
//! The repositories are T15-b's Postgres ones; `BookingPorts` and `BookingIds` are fakes defined
//! below, so the rules, the open shift, the counter rate and the live customers are what each
//! test sets. Times are fixed instants (`common::at`) passed as `now`, never a clock; an order by
//! time is checked on distinct instants and, on equal ones, by an id the test chose.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Amended for #758 (RE-4d) by the ticket author (claude/e5ab6c), `docs/design/blueprint/rates.md`
//! §4.8 and §4.9 R1, R4, R6:
//! - `BookingPorts::counter_rate_in(tx, branch, currency, side, amount: Option<Money>)` answers
//!   the current published line (`Some(amount)` at create, whose THB R5 checks; `None` at confirm,
//!   ruling 5 on #758): a `CounterLine` (its id, its rate per `unit_size`, its unit), no longer
//!   a bare rate. Every rate the fake answers is a real `published_rate_lines` row of its own
//!   (`Shop::set_rate`), because `bookings.published_rate_line_id` is a foreign key. Every line
//!   here is unit 1, so the rates read as before.
//! - A booking holds its `unit_size` and the line it was made at; `create` stores the line.
//! - `confirm` compares the stored line with the current one and never re-locks a rate:
//!   `confirm_locks_the_counter_rate_and_the_sent_expiry` became
//!   `confirm_keeps_the_line_and_rate_of_create_and_sets_the_sent_expiry`, and
//!   `confirm_refusals_write_nothing` puts its PENDING booking on MAIN's current EUR line and adds
//!   a moved line at the same rate.
//! - The commands' expected rate is the v2 wire's per-1 rate, a `Decimal` of up to 11 places
//!   (R3 and the ticket author's ruling 4 on #758), no longer a `Rate` (`wire`).

// `common` re-exports the customer types its own tests name; this file names none of them.
#[allow(unused_imports)]
mod common;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use common::{MAIN, NO_SUCH_BRANCH, TestDb, at, branch, millis, refusal};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use xc_exchange::application::booking::list::{BookingList, BookingView};
use xc_exchange::application::booking::service::BookingService;
use xc_exchange::domain::booking::commands::confirm_booking::ConfirmBooking;
use xc_exchange::domain::booking::commands::create_booking::CreateBooking;
use xc_exchange::domain::booking::commands::issue_member::IssueMember;
use xc_exchange::domain::booking::commands::redeem_booking::RedeemBooking;
use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::entities::booking_steps::{LiveCustomer, RedeemStep};
use xc_exchange::domain::booking::entities::member::Member;
use xc_exchange::domain::booking::repositories::booking_ids::BookingIds;
use xc_exchange::domain::booking::repositories::booking_ports::{BookingPorts, CounterLine};
use xc_exchange::domain::booking::repositories::booking_repository::BookingRepository;
use xc_exchange::domain::booking::repositories::member_repository::MemberRepository;
use xc_exchange::domain::booking::value_objects::booking_input::token_hash;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;

const AIRPORT: &str = "AIRPORT";
/// A branch nobody created.
const NOWHERE: &str = "NOWHERE";

// The sentences of §5.6.3 this row answers.
const NOT_FOUND: &str = "ไม่พบการจอง";
const NOT_FOUND_HERE: &str = "ไม่พบการจองของสาขานี้";
const HISTORY_TOO_LONG: &str = "ประวัติจองเกิน 1,000 รายการ กรุณาติดต่อผู้ดูแลเพื่อเพิ่มการแบ่งหน้า";
const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";
const NO_MEMBER: &str = "ไม่พบ Member ของลูกค้านี้ในสาขานี้";
const CLOSED: &str = "ปิดรับจองใหม่";
const NO_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
const RATE_MOVED: &str = "เรทเปลี่ยน กรุณาโหลดใหม่";
const EXPIRY_IN_FUTURE: &str = "วันหมดอายุต้องอยู่ในอนาคต";
const ONLY_PENDING_MEMBER: &str = "ยืนยันได้เฉพาะคำขอ Member ที่รอยืนยัน";
const ALREADY_USED: &str = "การจองถูกใช้ทำรายการแล้ว";
const USED_BY_ANOTHER: &str = "การจองนี้ถูกใช้ทำรายการแล้ว";
const MEMBER_MISMATCH: &str = "ลูกค้าไม่ตรงกับ Member ที่จอง";
const NOT_IDENTIFIED: &str = "ต้องเลือกลูกค้าและยืนยันตรวจตัวตนหน้าร้านก่อน";
const BAD_MEMBER_CODE: &str = "รหัสสมาชิกไม่ถูกต้อง";

const SECRET_A: &str = "aaaaaaaa-1111-4111-8111-111111111111";
const SECRET_B: &str = "bbbbbbbb-2222-4222-8222-222222222222";
const SECRET_C: &str = "cccccccc-3333-4333-8333-333333333333";

// ── values ───────────────────────────────────────────────────────────────────

fn money(text: &str) -> Money {
    Money::parse(text).expect("an amount")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a rate")
}

/// A per-1 rate as the v2 wire carries it (RE-4d ruling 4: up to 11 places, not a `Rate`).
fn wire(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency")
}

fn minutes(n: i64) -> Duration {
    Duration::minutes(n)
}

/// A non-member's order for 1,000.50 USD sold at `expected`.
fn order(request_id: &str, expected: &str) -> CreateBooking {
    CreateBooking {
        request_id: request_id.to_string(),
        name: "สมชาย ใจดี".to_string(),
        phone: "+66 (81) 234-5678".to_string(),
        side: Side::Sell,
        currency: code("USD"),
        amount: money("1000.50"),
        expected_rate: wire(expected),
        customer_id: None,
    }
}

/// A member's order for 250 EUR bought at `expected`, naming `customer_id`.
fn member_order(request_id: &str, customer_id: &str, expected: &str) -> CreateBooking {
    CreateBooking {
        request_id: request_id.to_string(),
        name: "Jane Doe".to_string(),
        phone: "0812345678".to_string(),
        side: Side::Buy,
        currency: code("EUR"),
        amount: money("250"),
        expected_rate: wire(expected),
        customer_id: Some(customer_id.to_string()),
    }
}

/// A non-member's booking as `create` stores it: CONFIRMED until `created` + 30 minutes.
fn confirmed(id: &str, branch_id: &str, created: i64) -> Booking {
    Booking {
        id: id.to_string(),
        branch_id: branch(branch_id),
        name: format!("Booker {id}"),
        phone: "+66 (81) 234-5678".to_string(),
        member_id: None,
        side: Side::Sell,
        currency: code("USD"),
        amount: money("1000.50"),
        rate: rate("32.6"),
        unit_size: UnitSize::ONE,
        published_rate_line_id: None,
        status: BookingStatus::Confirmed,
        created_at: at(created),
        created_by: "owner".to_string(),
        expires_at: Some(at(created + 1800)),
        confirmed_at: None,
        confirmed_by: None,
        submission_id: None,
        transaction_ref: None,
    }
}

/// A member's booking as `create` stores it: PENDING, no expiry, bought EUR at 35.
fn pending(id: &str, branch_id: &str, member_id: &str, created: i64) -> Booking {
    Booking {
        id: id.to_string(),
        branch_id: branch(branch_id),
        name: "Jane Doe".to_string(),
        phone: "0812345678".to_string(),
        member_id: Some(member_id.to_string()),
        side: Side::Buy,
        currency: code("EUR"),
        amount: money("250"),
        rate: rate("35"),
        unit_size: UnitSize::ONE,
        published_rate_line_id: None,
        status: BookingStatus::Pending,
        created_at: at(created),
        created_by: "FRONT-1".to_string(),
        expires_at: None,
        confirmed_at: None,
        confirmed_by: None,
        submission_id: None,
        transaction_ref: None,
    }
}

fn member_row(id: &str, branch_id: &str, customer_id: &str, secret: &str, verified: i64) -> Member {
    Member {
        id: id.to_string(),
        branch_id: branch(branch_id),
        customer_id: customer_id.to_string(),
        name: format!("Customer {customer_id}"),
        phone: "0899999999".to_string(),
        token_hash: token_hash(secret),
        verified_at: at(verified),
        verified_by: "owner".to_string(),
    }
}

fn issue(customer_id: &str, identity_confirmed: bool, secret: &str) -> IssueMember {
    IssueMember {
        customer_id: customer_id.to_string(),
        identity_confirmed,
        phone: "0811111111".to_string(),
        secret: secret.to_string(),
    }
}

fn confirm_cmd(
    id: &str,
    expires_at: Option<DateTime<Utc>>,
    expected: Option<&str>,
) -> ConfirmBooking {
    ConfirmBooking {
        id: id.to_string(),
        expires_at,
        expected_rate: expected.map(wire),
    }
}

/// The save's claim on `booking`, matching it in every field.
fn claim(booking: &Booking, submission_id: &str, customer_id: &str) -> RedeemBooking {
    RedeemBooking {
        booking_id: booking.id.clone(),
        submission_id: submission_id.to_string(),
        live: true,
        side: booking.side,
        currency: booking.currency,
        amount: booking.amount,
        // amended for #757 (RE-4c, rates.md §4.9 R7): the form's rate is the exact per-1 Decimal
        // (the bookings here are unit 1: the stored rate's number)
        rate: booking.rate.value(),
        form_booking: booking.id.clone(),
        customer_id: customer_id.to_string(),
        customer_confirmed: true,
    }
}

fn ids(bookings: &[Booking]) -> Vec<&str> {
    bookings.iter().map(|b| b.id.as_str()).collect()
}

fn member_ids(members: &[Member]) -> Vec<&str> {
    members.iter().map(|m| m.id.as_str()).collect()
}

// ── the fakes ────────────────────────────────────────────────────────────────

/// `BookingPorts` answering what the test set, the same in and out of a transaction.
#[derive(Default)]
struct FakePorts {
    branches: HashSet<String>,
    /// (branch, customer id) → name, LIVE customers only.
    live: Mutex<HashMap<(String, String), String>>,
    rules: Mutex<HashMap<String, BookingRules>>,
    open_shifts: Mutex<HashSet<String>>,
    counter: Mutex<HashMap<(String, CurrencyCode, Side), CounterLine>>,
}

impl FakePorts {
    fn set_line(&self, branch_id: &str, currency: &str, side: Side, line: CounterLine) {
        self.counter
            .lock()
            .expect("lock")
            .insert((branch_id.to_string(), code(currency), side), line);
    }

    fn line(&self, branch_id: &str, currency: &str, side: Side) -> Uuid {
        self.counter
            .lock()
            .expect("lock")
            .get(&(branch_id.to_string(), code(currency), side))
            .map(|l| l.line_id)
            .unwrap_or_else(|| panic!("{branch_id} has a {currency} line"))
    }

    fn set_rules(&self, branch_id: &str, enabled: bool, minutes: u16) {
        self.rules
            .lock()
            .expect("lock")
            .insert(branch_id.to_string(), BookingRules { enabled, minutes });
    }

    fn close_shift(&self, branch_id: &str) {
        self.open_shifts.lock().expect("lock").remove(branch_id);
    }

    fn the_rules(&self, branch_id: &BranchId) -> Option<BookingRules> {
        self.rules
            .lock()
            .expect("lock")
            .get(branch_id.as_str())
            .copied()
    }
}

impl BookingPorts for FakePorts {
    fn branch_exists<'a>(&'a self, branch_id: &'a BranchId) -> BoxFuture<'a, AppResult<bool>> {
        let known = self.branches.contains(branch_id.as_str());
        Box::pin(async move { Ok(known) })
    }

    fn live_customer<'a>(
        &'a self,
        branch_id: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<LiveCustomer>>> {
        let found = self
            .live
            .lock()
            .expect("lock")
            .get(&(branch_id.as_str().to_string(), id.to_string()))
            .map(|name| LiveCustomer {
                id: id.to_string(),
                name: name.clone(),
            });
        Box::pin(async move { Ok(found) })
    }

    fn rules<'a>(
        &'a self,
        branch_id: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<BookingRules>>> {
        let rules = self.the_rules(branch_id);
        Box::pin(async move { Ok(rules) })
    }

    fn rules_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        branch_id: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<BookingRules>>> {
        let rules = self.the_rules(branch_id);
        Box::pin(async move { Ok(rules) })
    }

    fn has_open_shift_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        branch_id: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>> {
        let open = self
            .open_shifts
            .lock()
            .expect("lock")
            .contains(branch_id.as_str());
        Box::pin(async move { Ok(open) })
    }

    fn counter_rate_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        branch_id: &'a BranchId,
        currency: &'a CurrencyCode,
        side: Side,
        _amount: Option<Money>,
    ) -> BoxFuture<'a, AppResult<Option<CounterLine>>> {
        let found = self
            .counter
            .lock()
            .expect("lock")
            .get(&(branch_id.as_str().to_string(), *currency, side))
            .copied();
        Box::pin(async move { Ok(found) })
    }
}

/// `BK-T-0001`, `BK-T-0002`, … and `MB-T-0001`, … in the order they are asked for.
#[derive(Default)]
struct CountingIds {
    bookings: AtomicU32,
    members: AtomicU32,
}

impl BookingIds for CountingIds {
    fn booking_id(&self) -> String {
        format!(
            "BK-T-{:04}",
            self.bookings.fetch_add(1, Ordering::Relaxed) + 1
        )
    }

    fn member_id(&self) -> String {
        format!(
            "MB-T-{:04}",
            self.members.fetch_add(1, Ordering::Relaxed) + 1
        )
    }
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Shop {
    db: TestDb,
    ports: Arc<FakePorts>,
    bookings: Arc<dyn BookingRepository>,
    members: Arc<dyn MemberRepository>,
    service: BookingService,
}

impl Shop {
    /// `MAIN` and `AIRPORT`, both with an open shift. `MAIN` has no booking rules (the default
    /// applies); `AIRPORT` takes bookings for 45 minutes. Live customers `C-MAIN-1..3` in `MAIN`
    /// and `C-AIR-1` in `AIRPORT`; `C-MAIN-DEMO` is a DEMO customer of `MAIN`. The counter rates
    /// differ per branch: USD sold at 32.6 / 32.7, EUR bought at 35.1234 / 36.5.
    async fn open() -> Shop {
        let db = TestDb::start().await;
        db.seed_branch(AIRPORT, "Airport").await;

        let ports = FakePorts {
            branches: [MAIN, AIRPORT].iter().map(|b| b.to_string()).collect(),
            ..FakePorts::default()
        };
        ports
            .open_shifts
            .lock()
            .expect("lock")
            .extend([MAIN.to_string(), AIRPORT.to_string()]);
        ports.set_rules(AIRPORT, true, 45);
        for (branch_id, currency, side, to) in [
            (MAIN, "USD", Side::Sell, "32.6"),
            (MAIN, "EUR", Side::Buy, "35.1234"),
            (AIRPORT, "USD", Side::Sell, "32.7"),
            (AIRPORT, "EUR", Side::Buy, "36.5"),
        ] {
            let line = publish_line(&db, branch_id, currency, to).await;
            ports.set_line(branch_id, currency, side, line);
        }
        let ports = Arc::new(ports);

        for (id, branch_id, mode) in [
            ("C-MAIN-1", MAIN, "LIVE"),
            ("C-MAIN-2", MAIN, "LIVE"),
            ("C-MAIN-3", MAIN, "LIVE"),
            ("C-MAIN-DEMO", MAIN, "DEMO"),
            ("C-AIR-1", AIRPORT, "LIVE"),
        ] {
            seed_customer(&db, &ports, id, branch_id, mode).await;
        }

        let bookings: Arc<dyn BookingRepository> =
            Arc::new(PgBookingRepository::new(db.pool.clone()));
        let members: Arc<dyn MemberRepository> = Arc::new(PgMemberRepository::new(db.pool.clone()));
        let service = BookingService::new(
            db.branch_tx(),
            bookings.clone(),
            members.clone(),
            ports.clone(),
            Arc::new(CountingIds::default()),
        );
        Shop {
            db,
            ports,
            bookings,
            members,
            service,
        }
    }

    async fn begin(&self) -> Transaction<'static, Postgres> {
        self.db.pool.begin().await.expect("begin a transaction")
    }

    /// A new current line of (`branch_id`, `currency`, `side`) at `to` per 1 (RE-4d): a real
    /// `published_rate_lines` row, answered by the fake ports from now on.
    async fn set_rate(&self, branch_id: &str, currency: &str, side: Side, to: &str) -> Uuid {
        let line = publish_line(&self.db, branch_id, currency, to).await;
        self.ports.set_line(branch_id, currency, side, line);
        line.line_id
    }

    /// A booking stored by T15-b's repository, not by the service under test.
    async fn seed(&self, booking: &Booking) {
        let mut tx = self.begin().await;
        self.bookings
            .insert_in(&mut tx, booking, &format!("REQ-{}", booking.id), "seeded")
            .await
            .unwrap_or_else(|e| panic!("insert {}: {e:?}", booking.id));
        tx.commit().await.expect("commit");
    }

    /// A member stored by T15-b's repository, not by the service under test.
    async fn seed_member(&self, member: &Member) -> Member {
        let mut tx = self.begin().await;
        let stored = self
            .members
            .upsert_in(&mut tx, member)
            .await
            .expect("upsert_in")
            .unwrap_or_else(|_| panic!("the seeded member {} took a used code", member.id));
        tx.commit().await.expect("commit");
        stored
    }

    async fn found(&self, branch_id: &str, id: &str) -> Option<Booking> {
        self.bookings
            .find(&branch(branch_id), id)
            .await
            .expect("find")
    }

    async fn stored(&self, branch_id: &str, id: &str) -> Booking {
        self.found(branch_id, id)
            .await
            .unwrap_or_else(|| panic!("booking {id} is stored in {branch_id}"))
    }

    async fn stored_members(&self, branch_id: &str) -> Vec<Member> {
        self.members
            .list(&branch(branch_id))
            .await
            .expect("list members")
    }

    async fn version(&self, branch_id: &str) -> i64 {
        self.db
            .branch_tx()
            .version(&branch(branch_id))
            .await
            .expect("read the branch version")
    }

    async fn versions(&self) -> (i64, i64) {
        (self.version(MAIN).await, self.version(AIRPORT).await)
    }

    async fn count(&self, table: &str) -> i64 {
        self.db.count(table).await
    }

    fn close_shift_in(&self, branch_id: &str) {
        self.ports.close_shift(branch_id);
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// One MANUAL line of `currency` at `to` per 1, buy and sell, in a publication of its own of
/// `branch_id` (RE-4d: `bookings.published_rate_line_id` is a foreign key to it).
async fn publish_line(db: &TestDb, branch_id: &str, currency: &str, to: &str) -> CounterLine {
    let request = Uuid::new_v4().to_string();
    let publication = Uuid::new_v4().to_string();
    let line = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1::uuid, organization_id, 'RATE_PUBLICATION', id, $2, 'test', '1', \
             '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $4",
    )
    .bind(&request)
    .bind(&publication)
    .bind(format!("booking-service-{publication}"))
    .bind(branch_id)
    .execute(&db.pool)
    .await
    .expect("insert into approval_requests");
    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
         SELECT $1::uuid, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
             now(), $2::uuid, NULL, 'test' \
         FROM branches b WHERE b.id = $3",
    )
    .bind(&publication)
    .bind(&request)
    .bind(branch_id)
    .execute(&db.pool)
    .await
    .expect("insert into rate_publications");
    sqlx::query(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT $1::uuid, organization_id, $2::uuid, $3, 'ALL', 1, $4::numeric, $4::numeric, \
             '{}'::jsonb \
         FROM rate_publications WHERE id = $2::uuid",
    )
    .bind(line.to_string())
    .bind(&publication)
    .bind(currency)
    .bind(to)
    .execute(&db.pool)
    .await
    .expect("insert into published_rate_lines");
    CounterLine {
        line_id: line,
        rate: rate(to),
        unit_size: UnitSize::ONE,
    }
}

async fn seed_customer(db: &TestDb, ports: &FakePorts, id: &str, branch_id: &str, mode: &str) {
    let name = format!("ลูกค้า {id}");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $3, $4, $5 FROM branches b WHERE b.id = $2",
    )
    .bind(id)
    .bind(branch_id)
    .bind(&name)
    .bind(mode)
    .bind(at(0))
    .execute(&db.pool)
    .await
    .expect("insert into customers");
    if mode == "LIVE" {
        ports
            .live
            .lock()
            .expect("lock")
            .insert((branch_id.to_string(), id.to_string()), name);
    }
}

/// A stored DEMO exchange (a DEMO one needs no shift) that a USED booking can name.
async fn seed_transaction(db: &TestDb, branch_id: &str, document_ref: &str, customer_id: &str) {
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
             customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by) \
         VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'SELL', 'USD', \
             1000.50, 32.6, 32616.30, 0, 0, 32616.30, 0, 'Cash', '', '', 'owner', '', '', \
             DATE '2026-10-06', $5, $5, 'owner')",
    )
    .bind(document_ref)
    .bind(branch_id)
    .bind(format!("submission-of-{document_ref}"))
    .bind(customer_id)
    .bind(at(0))
    .execute(&db.pool)
    .await
    .expect("insert into transactions");
}

/// CONFIRMED bookings `BK-BULK-<branch>-<nnnn>` in `branch_id` for `n` in `first..=last`, booking
/// `n` created `n` seconds after `at(0)`.
async fn bulk_bookings(db: &TestDb, branch_id: &str, first: i32, last: i32) {
    sqlx::query(
        "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, type, code, \
             amount, rate, status, created_at, created_by, expires_at) \
         SELECT 'BK-BULK-' || $1::text || '-' || lpad(n::text, 4, '0'), $1::text, \
             'REQ-BULK-' || n, 'bulk', \
             'Bulk ' || n, '0812345678', 'SELL', 'USD', 100, 32.6, 'CONFIRMED', \
             $2::timestamptz + n * interval '1 second', 'owner', \
             $2::timestamptz + n * interval '1 second' + interval '30 minutes' \
         FROM generate_series($3::int, $4::int) AS n",
    )
    .bind(branch_id)
    .bind(at(0))
    .bind(first)
    .bind(last)
    .execute(&db.pool)
    .await
    .expect("insert the bulk bookings");
}

// ── every action: the branch ─────────────────────────────────────────────────

/// Ticket §8 row *any*: a branch the ports do not know answers `NO_SUCH_BRANCH` before
/// anything else — before a booking is looked for, a customer is judged or a fact is read.
#[tokio::test]
async fn every_action_refuses_an_unknown_branch() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let nowhere = branch(NOWHERE);
    shop.seed(&confirmed("BK-MAIN-1", MAIN, 0)).await;

    assert!(s.branch_exists(&branch(MAIN)).await.expect("branch_exists"));
    assert!(
        s.branch_exists(&branch(AIRPORT))
            .await
            .expect("branch_exists")
    );
    assert!(!s.branch_exists(&nowhere).await.expect("branch_exists"));

    assert_eq!(
        refusal(s.list(&nowhere, at(10)).await),
        NO_SUCH_BRANCH,
        "list"
    );
    assert_eq!(
        refusal(s.get(&nowhere, "BK-MAIN-1").await),
        NO_SUCH_BRANCH,
        "get"
    );
    assert_eq!(
        refusal(
            s.create(&nowhere, order("R-1", "32.6"), "FRONT-1", at(10))
                .await
        ),
        NO_SUCH_BRANCH,
        "create"
    );
    assert_eq!(
        refusal(
            s.confirm(
                &nowhere,
                confirm_cmd("BK-MAIN-1", Some(at(5000)), Some("32.6")),
                "FRONT-1",
                at(10)
            )
            .await
        ),
        NO_SUCH_BRANCH,
        "confirm"
    );
    assert_eq!(
        refusal(s.cancel(&nowhere, "BK-MAIN-1").await),
        NO_SUCH_BRANCH,
        "cancel"
    );
    assert_eq!(
        refusal(
            s.issue_member(&nowhere, issue("C-MAIN-1", true, SECRET_A), "owner", at(10))
                .await
        ),
        NO_SUCH_BRANCH,
        "member"
    );
    assert_eq!(
        refusal(s.members(&nowhere).await),
        NO_SUCH_BRANCH,
        "members"
    );

    assert_eq!(shop.count("bookings").await, 1, "nothing was stored");
    assert_eq!(shop.count("members").await, 0, "nothing was stored");
    assert_eq!(
        shop.stored(MAIN, "BK-MAIN-1").await,
        confirmed("BK-MAIN-1", MAIN, 0),
        "the booking is untouched"
    );
    shop.finish().await;
}

// ── create ───────────────────────────────────────────────────────────────────

/// §5.6.3 `create`, not a member (owner 2026-10-08): CONFIRMED, the counter rate, expiring after
/// the branch's minutes — 30 for a branch with no rules, the stored minutes otherwise.
#[tokio::test]
async fn create_confirms_a_non_member_booking_for_the_branch_minutes() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let now = at(100);
    let before = shop.versions().await;

    let made = s
        .create(&branch(MAIN), order("R-MAIN", "32.6"), "FRONT-1", now)
        .await
        .expect("create in MAIN");
    assert_eq!(made.id, "BK-T-0001", "the id is BookingIds::booking_id()");
    assert_eq!(made.branch_id, branch(MAIN));
    assert_eq!(made.status, BookingStatus::Confirmed);
    assert_eq!(made.member_id, None);
    assert_eq!(made.name, "สมชาย ใจดี");
    assert_eq!(made.phone, "+66 (81) 234-5678");
    assert_eq!(made.side, Side::Sell);
    assert_eq!(made.currency, code("USD"));
    assert_eq!(made.amount, money("1000.50"));
    assert_eq!(made.rate, rate("32.6"), "the counter rate");
    assert_eq!(
        made.published_rate_line_id,
        Some(shop.ports.line(MAIN, "USD", Side::Sell)),
        "the line it was made at (RE-4d R1)"
    );
    assert_eq!(made.unit_size, UnitSize::ONE, "the line's unit (RE-4d R1)");
    assert_eq!(made.created_by, "FRONT-1", "the actor");
    assert_eq!(millis(made.created_at), millis(now));
    assert_eq!(
        made.expires_at.map(millis),
        Some(millis(now + minutes(30))),
        "MAIN has no rules: the default 30 minutes"
    );
    assert_eq!(made.confirmed_at, None);
    assert_eq!(made.confirmed_by, None);
    assert_eq!(made.submission_id, None);
    assert_eq!(made.transaction_ref, None);
    assert_eq!(
        shop.stored(MAIN, &made.id).await,
        made,
        "the answer is what was stored"
    );
    assert_eq!(
        shop.versions().await,
        (before.0 + 1, before.1),
        "one BranchTx::run of MAIN, none of AIRPORT"
    );

    let there = s
        .create(&branch(AIRPORT), order("R-AIR", "32.7"), "MANAGER-1", now)
        .await
        .expect("create in AIRPORT");
    assert_eq!(there.branch_id, branch(AIRPORT));
    assert_eq!(there.status, BookingStatus::Confirmed);
    assert_eq!(there.rate, rate("32.7"), "AIRPORT's counter rate");
    assert_eq!(there.created_by, "MANAGER-1");
    assert_eq!(
        there.expires_at.map(millis),
        Some(millis(now + minutes(45))),
        "AIRPORT's rules: 45 minutes"
    );
    assert_eq!(shop.stored(AIRPORT, &there.id).await, there);
    assert_eq!(
        shop.found(MAIN, &there.id).await,
        None,
        "stored in AIRPORT only"
    );
    assert_eq!(
        shop.versions().await,
        (before.0 + 1, before.1 + 1),
        "one BranchTx::run of AIRPORT"
    );
    shop.finish().await;
}

/// §5.6.3 `create` for a member (owner 2026-10-08): PENDING with the member of the customer in
/// this branch, the counter rate, no expiry.
#[tokio::test]
async fn create_holds_a_member_booking_pending() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let member = shop
        .seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;

    let made = s
        .create(
            &branch(MAIN),
            member_order("R-M", "C-MAIN-1", "35.1234"),
            "BACKEND-1",
            at(100),
        )
        .await
        .expect("create a member's booking");
    assert_eq!(made.status, BookingStatus::Pending);
    assert_eq!(made.member_id.as_deref(), Some(member.id.as_str()));
    assert!(made.is_member());
    assert_eq!(
        made.expires_at, None,
        "the counter sets the expiry on confirm"
    );
    assert_eq!(made.rate, rate("35.1234"), "the counter rate");
    assert_eq!(made.side, Side::Buy);
    assert_eq!(made.currency, code("EUR"));
    assert_eq!(made.amount, money("250"));
    assert_eq!(made.created_by, "BACKEND-1");
    assert_eq!(shop.stored(MAIN, &made.id).await, made);
    shop.finish().await;
}

/// §5.6.3 `create` refusal 8: `customerId` given but no LIVE customer of the branch with a member
/// row of the branch. Nothing is stored and the branch version stays.
#[tokio::test]
async fn create_refuses_a_customer_without_a_member_of_this_branch() {
    let shop = Shop::open().await;
    let s = &shop.service;
    // C-AIR-1 is a member of AIRPORT only.
    shop.seed_member(&member_row("MB-AIR", AIRPORT, "C-AIR-1", SECRET_A, 0))
        .await;
    // C-MAIN-3 has a member row but the ports no longer know it as a live customer.
    shop.seed_member(&member_row("MB-GONE", MAIN, "C-MAIN-3", SECRET_B, 0))
        .await;
    shop.ports
        .live
        .lock()
        .expect("lock")
        .remove(&(MAIN.to_string(), "C-MAIN-3".to_string()));
    let before = shop.versions().await;

    for (customer, why) in [
        ("C-MAIN-2", "a live customer of MAIN with no member row"),
        ("C-AIR-1", "a member of AIRPORT, asked in MAIN"),
        ("C-MAIN-3", "a member row whose customer is not live"),
        ("C-MAIN-DEMO", "a DEMO customer"),
        ("C-NOBODY", "no such customer"),
    ] {
        let got = s
            .create(
                &branch(MAIN),
                member_order(&format!("R-{customer}"), customer, "35.1234"),
                "FRONT-1",
                at(100),
            )
            .await;
        assert_eq!(refusal(got), NO_MEMBER, "{why}");
    }
    assert_eq!(shop.count("bookings").await, 0, "nothing was stored");
    assert_eq!(shop.versions().await, before, "every refusal rolled back");
    shop.finish().await;
}

/// §5.6.3 `create` refusals 9–11 read the rules, the open shift and the counter rate of the
/// branch the booking is made in, inside its transaction — never another branch's.
#[tokio::test]
async fn create_reads_the_rules_shift_and_rate_of_its_own_branch() {
    let shop = Shop::open().await;
    let s = &shop.service;

    // MAIN's rate offered in AIRPORT, whose counter differs.
    assert_eq!(
        refusal(
            s.create(&branch(AIRPORT), order("R-1", "32.6"), "FRONT-1", at(100))
                .await
        ),
        RATE_MOVED
    );
    // No counter rate at all for the side asked.
    let mut buy_usd = order("R-2", "32.6");
    buy_usd.side = Side::Buy;
    assert_eq!(
        refusal(s.create(&branch(MAIN), buy_usd, "FRONT-1", at(100)).await),
        RATE_MOVED
    );

    shop.close_shift_in(AIRPORT);
    assert_eq!(
        refusal(
            s.create(&branch(AIRPORT), order("R-3", "32.7"), "FRONT-1", at(100))
                .await
        ),
        NO_SHIFT
    );
    shop.ports.set_rules(AIRPORT, false, 45);
    assert_eq!(
        refusal(
            s.create(&branch(AIRPORT), order("R-4", "32.7"), "FRONT-1", at(100))
                .await
        ),
        CLOSED
    );
    assert_eq!(shop.count("bookings").await, 0);

    // MAIN, with its shift open and no rules, still takes bookings.
    let made = s
        .create(&branch(MAIN), order("R-5", "32.6"), "FRONT-1", at(100))
        .await
        .expect("MAIN is open");
    assert_eq!(made.branch_id, branch(MAIN));
    assert_eq!(shop.count("bookings").await, 1);
    shop.finish().await;
}

/// §5.6.8 `create_is_idempotent`; §5.6.3 refusal 7: the same `requestId` and payload answer the
/// stored booking with nothing else checked and no second row; another payload answers
/// `CHANGED`. A request id is per branch.
#[tokio::test]
async fn create_is_idempotent() {
    let shop = Shop::open().await;
    let s = &shop.service;

    let first = s
        .create(&branch(MAIN), order("R-1", "32.6"), "FRONT-1", at(100))
        .await
        .expect("the first create");

    // Since then the counter moved and the branch stopped taking bookings: a repeat checks
    // nothing of that.
    shop.set_rate(MAIN, "USD", Side::Sell, "33.9").await;
    shop.ports.set_rules(MAIN, false, 30);
    shop.close_shift_in(MAIN);
    let again = s
        .create(&branch(MAIN), order("R-1", "32.6"), "FRONT-2", at(500))
        .await
        .expect("the same request again");
    assert_eq!(again, first, "the stored booking, as stored");
    assert_eq!(shop.count("bookings").await, 1, "no second row");

    let mut other = order("R-1", "32.6");
    other.amount = money("1000.51");
    assert_eq!(
        refusal(s.create(&branch(MAIN), other, "FRONT-1", at(600)).await),
        CHANGED
    );
    let mut renamed = order("R-1", "32.6");
    renamed.name = "สมหญิง ใจดี".to_string();
    assert_eq!(
        refusal(s.create(&branch(MAIN), renamed, "FRONT-1", at(600)).await),
        CHANGED
    );
    assert_eq!(shop.count("bookings").await, 1);
    assert_eq!(
        shop.stored(MAIN, &first.id).await,
        first,
        "the stored one is untouched"
    );

    // The same request id in AIRPORT is AIRPORT's own booking.
    let mut theirs = order("R-1", "32.7");
    theirs.amount = money("75");
    let there = s
        .create(&branch(AIRPORT), theirs, "FRONT-9", at(700))
        .await
        .expect("a request id of another branch is not a repeat");
    assert_ne!(there.id, first.id);
    assert_eq!(there.amount, money("75"));
    assert_eq!(shop.count("bookings").await, 2);
    shop.finish().await;
}

// ── list, get, members ───────────────────────────────────────────────────────

/// Ticket §8 `list`: the branch's bookings newest first (`created_at` desc, then `id` desc), its
/// members in `verified_at`, `id` order, and its rules or none. `members` answers the same
/// members.
#[tokio::test]
async fn list_answers_the_branchs_bookings_members_and_rules() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed(&confirmed("BK-A", MAIN, 10)).await;
    shop.seed(&confirmed("BK-C", MAIN, 20)).await;
    shop.seed(&confirmed("BK-B", MAIN, 20)).await;
    shop.seed(&confirmed("BK-Z", AIRPORT, 30)).await;
    shop.seed_member(&member_row("MB-2", MAIN, "C-MAIN-1", SECRET_A, 5))
        .await;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-2", SECRET_B, 5))
        .await;
    shop.seed_member(&member_row("MB-3", MAIN, "C-MAIN-3", SECRET_C, 1))
        .await;
    shop.seed_member(&member_row(
        "MB-9",
        AIRPORT,
        "C-AIR-1",
        "dddddddd-4444-4444-8444-444444444444",
        0,
    ))
    .await;

    let main: BookingList = s.list(&branch(MAIN), at(40)).await.expect("list MAIN");
    assert_eq!(ids(&main.bookings), vec!["BK-C", "BK-B", "BK-A"]);
    assert_eq!(
        main.bookings[2],
        confirmed("BK-A", MAIN, 10),
        "every field as stored"
    );
    assert_eq!(member_ids(&main.members), vec!["MB-3", "MB-1", "MB-2"]);
    assert_eq!(
        main.members[0],
        member_row("MB-3", MAIN, "C-MAIN-3", SECRET_C, 1)
    );
    assert_eq!(main.rules, None, "MAIN has no rules stored");

    let air = s
        .list(&branch(AIRPORT), at(40))
        .await
        .expect("list AIRPORT");
    assert_eq!(ids(&air.bookings), vec!["BK-Z"]);
    assert_eq!(member_ids(&air.members), vec!["MB-9"]);
    assert_eq!(
        air.rules,
        Some(BookingRules {
            enabled: true,
            minutes: 45
        })
    );

    let members = s.members(&branch(MAIN)).await.expect("members of MAIN");
    assert_eq!(member_ids(&members), vec!["MB-3", "MB-1", "MB-2"]);
    let members = s
        .members(&branch(AIRPORT))
        .await
        .expect("members of AIRPORT");
    assert_eq!(member_ids(&members), vec!["MB-9"]);
    shop.finish().await;
}

/// §5.6.8 `list_caps_at_one_thousand`: 1,000 bookings are listed; the 1,001st makes `list` refuse
/// with v2's sentence. Only the branch's own bookings count.
#[tokio::test]
async fn list_caps_at_one_thousand() {
    let shop = Shop::open().await;
    let s = &shop.service;
    bulk_bookings(&shop.db, MAIN, 1, 1000).await;
    bulk_bookings(&shop.db, AIRPORT, 1, 3).await;

    let main = s
        .list(&branch(MAIN), at(5000))
        .await
        .expect("1,000 are listed");
    assert_eq!(main.bookings.len(), 1000);
    assert_eq!(main.bookings[0].id, "BK-BULK-MAIN-1000", "newest first");
    assert_eq!(main.bookings[999].id, "BK-BULK-MAIN-0001");

    bulk_bookings(&shop.db, MAIN, 1001, 1001).await;
    assert_eq!(
        refusal(s.list(&branch(MAIN), at(5000)).await),
        HISTORY_TOO_LONG
    );

    let air = s
        .list(&branch(AIRPORT), at(5000))
        .await
        .expect("AIRPORT has 3");
    assert_eq!(
        ids(&air.bookings),
        vec![
            "BK-BULK-AIRPORT-0003",
            "BK-BULK-AIRPORT-0002",
            "BK-BULK-AIRPORT-0001"
        ]
    );
    shop.finish().await;
}

/// Ticket §8 `get`: the booking and its member's customer id, or none for a non-member's.
#[tokio::test]
async fn get_answers_the_booking_and_its_members_customer() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-2", SECRET_A, 0))
        .await;
    shop.seed(&pending("BK-P", MAIN, "MB-1", 10)).await;
    shop.seed(&confirmed("BK-C", MAIN, 20)).await;

    let view: BookingView = s.get(&branch(MAIN), "BK-P").await.expect("get BK-P");
    assert_eq!(view.booking, pending("BK-P", MAIN, "MB-1", 10));
    assert_eq!(view.customer_id.as_deref(), Some("C-MAIN-2"));

    let view = s.get(&branch(MAIN), "BK-C").await.expect("get BK-C");
    assert_eq!(view.booking, confirmed("BK-C", MAIN, 20));
    assert_eq!(view.customer_id, None, "not a member's booking");

    assert_eq!(refusal(s.get(&branch(MAIN), "BK-NONE").await), NOT_FOUND);
    shop.finish().await;
}

/// §5.6.8 `other_branch_is_not_found`; §5.6.4 *Per branch everywhere*: a booking is found by
/// (branch, id), so another branch's id answers `ไม่พบการจอง` to `get`, `confirm` and `cancel`
/// and nothing of either branch changes.
#[tokio::test]
async fn other_branch_is_not_found() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;
    shop.seed_member(&member_row("MB-9", AIRPORT, "C-AIR-1", SECRET_B, 0))
        .await;
    let main_pending = pending("BK-MAIN-P", MAIN, "MB-1", 10);
    let main_confirmed = confirmed("BK-MAIN-C", MAIN, 20);
    let mut air_pending = pending("BK-AIR-P", AIRPORT, "MB-9", 30);
    air_pending.amount = money("999");
    air_pending.name = "Airport Booker".to_string();
    for b in [&main_pending, &main_confirmed, &air_pending] {
        shop.seed(b).await;
    }
    let before = shop.versions().await;

    for id in ["BK-MAIN-P", "BK-MAIN-C"] {
        assert_eq!(
            refusal(s.get(&branch(AIRPORT), id).await),
            NOT_FOUND,
            "get {id}"
        );
        assert_eq!(
            refusal(
                s.confirm(
                    &branch(AIRPORT),
                    confirm_cmd(id, Some(at(5000)), Some("36.5")),
                    "FRONT-1",
                    at(100)
                )
                .await
            ),
            NOT_FOUND,
            "confirm {id}"
        );
        assert_eq!(
            refusal(s.cancel(&branch(AIRPORT), id).await),
            NOT_FOUND,
            "cancel {id}"
        );
    }
    assert_eq!(refusal(s.get(&branch(MAIN), "BK-AIR-P").await), NOT_FOUND);
    assert_eq!(
        refusal(s.cancel(&branch(MAIN), "BK-AIR-P").await),
        NOT_FOUND
    );

    assert_eq!(shop.stored(MAIN, "BK-MAIN-P").await, main_pending);
    assert_eq!(shop.stored(MAIN, "BK-MAIN-C").await, main_confirmed);
    assert_eq!(shop.stored(AIRPORT, "BK-AIR-P").await, air_pending);
    assert_eq!(shop.versions().await, before, "every refusal rolled back");

    let view = s
        .get(&branch(AIRPORT), "BK-AIR-P")
        .await
        .expect("its own branch");
    assert_eq!(view.booking, air_pending);
    assert_eq!(view.customer_id.as_deref(), Some("C-AIR-1"));
    shop.finish().await;
}

// ── confirm ──────────────────────────────────────────────────────────────────

/// §5.6.3 `confirm`: PENDING → CONFIRMED, the expiry as sent, `confirmed_at`/`confirmed_by` set;
/// one `BranchTx::run` of the branch.
///
/// Amended for #758 (RE-4d, rates §4.9 R6): this was `confirm_locks_the_counter_rate_and_the_sent_expiry`
/// (made at 35, confirmed at the counter's 35.1234). R6: confirm compares the line stored at create
/// with the current one and never re-locks, so the booking is made on MAIN's current EUR line at
/// its 35.1234 and keeps that line, that rate and its unit.
#[tokio::test]
async fn confirm_keeps_the_line_and_rate_of_create_and_sets_the_sent_expiry() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;
    let eur = shop.ports.line(MAIN, "EUR", Side::Buy);
    let mut made = pending("BK-P", MAIN, "MB-1", 10);
    made.rate = rate("35.1234");
    made.published_rate_line_id = Some(eur);
    shop.seed(&made).await;
    let before = shop.versions().await;
    let expiry = at(100) + minutes(90);

    s.confirm(
        &branch(MAIN),
        confirm_cmd("BK-P", Some(expiry), Some("35.1234")),
        "MANAGER-1",
        at(100),
    )
    .await
    .expect("confirm");

    let got = shop.stored(MAIN, "BK-P").await;
    let mut want = made.clone();
    want.status = BookingStatus::Confirmed;
    want.expires_at = Some(expiry);
    want.confirmed_at = Some(at(100));
    want.confirmed_by = Some("MANAGER-1".to_string());
    assert_eq!(got.status, want.status);
    assert_eq!(
        (got.published_rate_line_id, got.rate, got.unit_size),
        (Some(eur), rate("35.1234"), UnitSize::ONE),
        "the line, the rate and the unit of create (R6)"
    );
    assert_eq!(got.expires_at.map(millis), want.expires_at.map(millis));
    assert_eq!(got.confirmed_at.map(millis), want.confirmed_at.map(millis));
    assert_eq!(got.confirmed_by, want.confirmed_by);
    assert_eq!(
        (got.name, got.member_id, got.amount, got.created_at),
        (want.name, want.member_id, want.amount, want.created_at),
        "nothing else changed"
    );
    assert_eq!(
        shop.versions().await,
        (before.0 + 1, before.1),
        "one BranchTx::run of MAIN, none of AIRPORT"
    );
    shop.finish().await;
}

/// §5.6.8 `double_confirm_writes_nothing`: a second confirm with the same expiry answers, writes
/// nothing, and checks no rate even though the counter moved.
///
/// Amended for #758 (RE-4d, rates §4.9 R6): the PENDING booking is made on MAIN's current EUR line
/// at its 35.1234, so the first confirm is taken; the move is a new line.
#[tokio::test]
async fn double_confirm_writes_nothing() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;
    let mut made = pending("BK-P", MAIN, "MB-1", 10);
    made.rate = rate("35.1234");
    made.published_rate_line_id = Some(shop.ports.line(MAIN, "EUR", Side::Buy));
    shop.seed(&made).await;
    let expiry = at(100) + minutes(90);

    s.confirm(
        &branch(MAIN),
        confirm_cmd("BK-P", Some(expiry), Some("35.1234")),
        "FRONT-1",
        at(100),
    )
    .await
    .expect("the first confirm");
    let first = shop.stored(MAIN, "BK-P").await;

    shop.set_rate(MAIN, "EUR", Side::Buy, "36.0").await;
    s.confirm(
        &branch(MAIN),
        confirm_cmd("BK-P", Some(expiry), Some("35.1234")),
        "FRONT-2",
        at(200),
    )
    .await
    .expect("the second confirm answers");
    assert_eq!(
        shop.stored(MAIN, "BK-P").await,
        first,
        "nothing written: rate, confirmed_at and confirmed_by are the first confirm's"
    );
    shop.finish().await;
}

/// §5.6.3 `confirm` refusals 3, 5 and 6 reach the caller as their sentences and write nothing.
///
/// Amended for #758 (RE-4d, rates §4.9 R4/R6): the PENDING booking is made on MAIN's current EUR
/// line at its 35.1234, so each row refuses for its own reason; and a new EUR line at the very
/// same 35.1234 refuses as a moved rate.
#[tokio::test]
async fn confirm_refusals_write_nothing() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;
    let mut waiting = pending("BK-P", MAIN, "MB-1", 10);
    waiting.rate = rate("35.1234");
    waiting.published_rate_line_id = Some(shop.ports.line(MAIN, "EUR", Side::Buy));
    let mut no_counter = pending("BK-P-GBP", MAIN, "MB-1", 11);
    no_counter.currency = code("GBP");
    let plain = confirmed("BK-C", MAIN, 20);
    for b in [&waiting, &no_counter, &plain] {
        shop.seed(b).await;
    }
    let before = shop.versions().await;
    let now = at(100);

    for (cmd, want, why) in [
        (
            confirm_cmd("BK-P", Some(now - minutes(1)), Some("35.1234")),
            EXPIRY_IN_FUTURE,
            "an expiry in the past",
        ),
        (
            confirm_cmd("BK-P", Some(now), Some("35.1234")),
            EXPIRY_IN_FUTURE,
            "an expiry of now",
        ),
        (
            confirm_cmd("BK-P", None, Some("35.1234")),
            EXPIRY_IN_FUTURE,
            "an unreadable expiry",
        ),
        (
            confirm_cmd("BK-C", Some(now + minutes(60)), Some("32.6")),
            ONLY_PENDING_MEMBER,
            "a non-member's CONFIRMED booking",
        ),
        (
            confirm_cmd("BK-P", Some(now + minutes(60)), Some("36.5")),
            RATE_MOVED,
            "AIRPORT's counter rate, not MAIN's",
        ),
        (
            confirm_cmd("BK-P", Some(now + minutes(60)), None),
            RATE_MOVED,
            "no expected rate",
        ),
        (
            confirm_cmd("BK-P-GBP", Some(now + minutes(60)), Some("35.1234")),
            RATE_MOVED,
            "no counter rate for GBP",
        ),
    ] {
        let got = s.confirm(&branch(MAIN), cmd, "FRONT-1", now).await;
        assert_eq!(refusal(got), want, "{why}");
    }
    shop.set_rate(MAIN, "EUR", Side::Buy, "35.1234").await;
    let got = s
        .confirm(
            &branch(MAIN),
            confirm_cmd("BK-P", Some(now + minutes(60)), Some("35.1234")),
            "FRONT-1",
            now,
        )
        .await;
    assert_eq!(refusal(got), RATE_MOVED, "another line at the same rate");
    assert_eq!(shop.stored(MAIN, "BK-P").await, waiting);
    assert_eq!(shop.stored(MAIN, "BK-P-GBP").await, no_counter);
    assert_eq!(shop.stored(MAIN, "BK-C").await, plain);
    assert_eq!(shop.versions().await, before, "every refusal rolled back");
    shop.finish().await;
}

// ── cancel ───────────────────────────────────────────────────────────────────

/// §5.6.3 `cancel`: PENDING and CONFIRMED (also one past its expiry) become CANCELLED and nothing
/// else changes; a cancel after a cancel answers and writes nothing.
#[tokio::test]
async fn cancel_ends_a_pending_or_confirmed_booking_once() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;
    let waiting = pending("BK-P", MAIN, "MB-1", 10);
    let plain = confirmed("BK-C", MAIN, 20);
    let mut lapsed = confirmed("BK-OLD", MAIN, -100_000);
    lapsed.expires_at = Some(at(-90_000));
    for b in [&waiting, &plain, &lapsed] {
        shop.seed(b).await;
    }

    for b in [&waiting, &plain, &lapsed] {
        let before = shop.versions().await;
        s.cancel(&branch(MAIN), &b.id).await.expect("cancel");
        let mut want = b.clone();
        want.status = BookingStatus::Cancelled;
        assert_eq!(
            shop.stored(MAIN, &b.id).await,
            want,
            "{} is CANCELLED",
            b.id
        );
        assert_eq!(
            shop.versions().await,
            (before.0 + 1, before.1),
            "one BranchTx::run of MAIN"
        );
    }

    let cancelled = shop.stored(MAIN, "BK-P").await;
    s.cancel(&branch(MAIN), "BK-P")
        .await
        .expect("a cancel after a cancel answers");
    assert_eq!(
        shop.stored(MAIN, "BK-P").await,
        cancelled,
        "and writes nothing"
    );
    shop.finish().await;
}

/// §5.6.3 `cancel` refusal 3: a USED booking answers `การจองถูกใช้ทำรายการแล้ว` and stays USED.
#[tokio::test]
async fn cancel_used_is_refused() {
    let shop = Shop::open().await;
    let s = &shop.service;
    seed_transaction(&shop.db, MAIN, "DOC-1", "C-MAIN-1").await;
    let mut used = confirmed("BK-U", MAIN, 10);
    used.status = BookingStatus::Used;
    used.submission_id = Some("SUB-1".to_string());
    used.transaction_ref = Some("DOC-1".to_string());
    shop.seed(&used).await;
    let before = shop.versions().await;

    assert_eq!(refusal(s.cancel(&branch(MAIN), "BK-U").await), ALREADY_USED);
    assert_eq!(shop.stored(MAIN, "BK-U").await, used);
    assert_eq!(shop.versions().await, before);
    shop.finish().await;
}

// ── member ───────────────────────────────────────────────────────────────────

/// §5.6.3 `member`: a member of the branch for a LIVE customer whose identity was confirmed —
/// id from `BookingIds::member_id()`, the customer's name, the phone sent, the SHA-256 of the
/// secret, verified now by the actor. A re-issue keeps the id and replaces the rest.
#[tokio::test]
async fn member_issues_and_reissues_a_member_of_the_branch() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let before = shop.versions().await;

    let made = s
        .issue_member(
            &branch(MAIN),
            issue("C-MAIN-1", true, SECRET_A),
            "owner",
            at(100),
        )
        .await
        .expect("issue a member");
    assert_eq!(made.id, "MB-T-0001", "the id is BookingIds::member_id()");
    assert_eq!(made.branch_id, branch(MAIN));
    assert_eq!(made.customer_id, "C-MAIN-1");
    assert_eq!(made.name, "ลูกค้า C-MAIN-1", "the customer's name");
    assert_eq!(made.phone, "0811111111");
    assert_eq!(made.token_hash, token_hash(SECRET_A));
    assert_eq!(made.verified_by, "owner");
    assert_eq!(millis(made.verified_at), millis(at(100)));
    assert_eq!(shop.stored_members(MAIN).await, vec![made.clone()]);
    assert_eq!(shop.stored_members(AIRPORT).await, vec![]);
    assert_eq!(
        shop.versions().await,
        (before.0 + 1, before.1),
        "one BranchTx::run of MAIN"
    );

    let mut again = issue("C-MAIN-1", true, SECRET_B);
    again.phone = "0822222222".to_string();
    let reissued = s
        .issue_member(&branch(MAIN), again, "owner-2", at(200))
        .await
        .expect("re-issue");
    assert_eq!(
        reissued.id, made.id,
        "the member id is kept, so its bookings still point at it"
    );
    assert_eq!(
        reissued.token_hash,
        token_hash(SECRET_B),
        "the old code no longer matches"
    );
    assert_eq!(reissued.phone, "0822222222");
    assert_eq!(reissued.verified_by, "owner-2");
    assert_eq!(millis(reissued.verified_at), millis(at(200)));
    assert_eq!(shop.stored_members(MAIN).await, vec![reissued.clone()]);

    let there = s
        .issue_member(
            &branch(AIRPORT),
            issue("C-AIR-1", true, SECRET_C),
            "owner",
            at(300),
        )
        .await
        .expect("a member of AIRPORT");
    assert_eq!(there.branch_id, branch(AIRPORT));
    assert_eq!(there.name, "ลูกค้า C-AIR-1");
    assert_eq!(shop.stored_members(AIRPORT).await, vec![there]);
    assert_eq!(
        shop.stored_members(MAIN).await,
        vec![reissued],
        "MAIN's are its own"
    );
    shop.finish().await;
}

/// §5.6.8 `member_needs_a_live_customer_and_true`; §5.6.3 `member` rule 1: no LIVE customer of
/// the branch with that id, or `identity_confirmed` false, answers
/// `ต้องเลือกลูกค้าและยืนยันตรวจตัวตนหน้าร้านก่อน` and stores nothing.
#[tokio::test]
async fn member_needs_a_live_customer_and_true() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let before = shop.versions().await;

    for (cmd, why) in [
        (issue("C-MAIN-DEMO", true, SECRET_A), "a DEMO customer"),
        (issue("C-MAIN-1", false, SECRET_A), "identity not confirmed"),
        (
            issue("C-AIR-1", true, SECRET_A),
            "a customer of AIRPORT, asked in MAIN",
        ),
        (issue("C-NOBODY", true, SECRET_A), "no such customer"),
    ] {
        let got = s.issue_member(&branch(MAIN), cmd, "owner", at(100)).await;
        assert_eq!(refusal(got), NOT_IDENTIFIED, "{why}");
    }
    assert_eq!(shop.count("members").await, 0, "nothing was stored");
    assert_eq!(shop.versions().await, before, "every refusal rolled back");
    shop.finish().await;
}

/// §5.6.3 `member` rule 5: a secret whose hash another member holds — in this branch or another,
/// `token_hash` is unique — answers `รหัสสมาชิกไม่ถูกต้อง`. The refusal rolls the transaction
/// back: the member asked for is not stored, the holder is untouched, the branch version stays,
/// and the service answers the next request normally.
#[tokio::test]
async fn member_refuses_a_code_another_member_holds() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let holder = s
        .issue_member(
            &branch(MAIN),
            issue("C-MAIN-1", true, SECRET_A),
            "owner",
            at(100),
        )
        .await
        .expect("the holder of SECRET_A");
    let before = shop.versions().await;

    assert_eq!(
        refusal(
            s.issue_member(
                &branch(MAIN),
                issue("C-MAIN-2", true, SECRET_A),
                "owner",
                at(200)
            )
            .await
        ),
        BAD_MEMBER_CODE,
        "another customer of MAIN"
    );
    assert_eq!(
        refusal(
            s.issue_member(
                &branch(AIRPORT),
                issue("C-AIR-1", true, SECRET_A),
                "owner",
                at(200)
            )
            .await
        ),
        BAD_MEMBER_CODE,
        "a customer of AIRPORT"
    );
    assert_eq!(shop.stored_members(MAIN).await, vec![holder.clone()]);
    assert_eq!(shop.stored_members(AIRPORT).await, vec![]);
    assert_eq!(shop.versions().await, before, "both refusals rolled back");

    let next = s
        .issue_member(
            &branch(MAIN),
            issue("C-MAIN-2", true, SECRET_B),
            "owner",
            at(300),
        )
        .await
        .expect("a code nobody holds is issued");
    assert_eq!(next.customer_id, "C-MAIN-2");
    assert_eq!(shop.stored_members(MAIN).await, vec![holder, next]);
    shop.finish().await;
}

// ── redeem (the save's transaction) ──────────────────────────────────────────

/// Ticket §7 `redeem.rs`: `redeem_in` and `use_in` run on the caller's transaction. A use rolled
/// back with it leaves the booking CONFIRMED; committed, it is USED by that submission; the same
/// submission is then a `Repeat`, another one `การจองนี้ถูกใช้ทำรายการแล้ว`.
#[tokio::test]
async fn redeem_and_use_run_on_the_callers_transaction() {
    let shop = Shop::open().await;
    let s = &shop.service;
    seed_transaction(&shop.db, MAIN, "DOC-1", "C-MAIN-1").await;
    let booking = confirmed("BK-C", MAIN, 0);
    shop.seed(&booking).await;
    let now = at(60);

    let mut tx = shop.begin().await;
    let step = s
        .redeem_in(
            &mut tx,
            &branch(MAIN),
            claim(&booking, "SUB-1", "C-MAIN-1"),
            now,
        )
        .await
        .expect("redeem_in");
    assert_eq!(step, RedeemStep::Use {});
    s.use_in(&mut tx, &branch(MAIN), "BK-C", "SUB-1", "DOC-1")
        .await
        .expect("use_in");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        shop.stored(MAIN, "BK-C").await,
        booking,
        "rolled back with the caller's transaction"
    );

    let mut tx = shop.begin().await;
    let step = s
        .redeem_in(
            &mut tx,
            &branch(MAIN),
            claim(&booking, "SUB-1", "C-MAIN-1"),
            now,
        )
        .await
        .expect("redeem_in");
    assert_eq!(step, RedeemStep::Use {});
    s.use_in(&mut tx, &branch(MAIN), "BK-C", "SUB-1", "DOC-1")
        .await
        .expect("use_in");
    tx.commit().await.expect("commit");
    let mut used = booking.clone();
    used.status = BookingStatus::Used;
    used.submission_id = Some("SUB-1".to_string());
    used.transaction_ref = Some("DOC-1".to_string());
    assert_eq!(shop.stored(MAIN, "BK-C").await, used);

    let mut tx = shop.begin().await;
    let step = s
        .redeem_in(
            &mut tx,
            &branch(MAIN),
            claim(&booking, "SUB-1", "C-MAIN-1"),
            at(120),
        )
        .await
        .expect("the same submission again");
    assert_eq!(step, RedeemStep::Repeat {});
    assert_eq!(
        refusal(
            s.redeem_in(
                &mut tx,
                &branch(MAIN),
                claim(&booking, "SUB-2", "C-MAIN-1"),
                at(120)
            )
            .await
        ),
        USED_BY_ANOTHER
    );
    tx.rollback().await.expect("rollback");
    assert_eq!(shop.stored(MAIN, "BK-C").await, used);
    shop.finish().await;
}

/// *Using a booking* row 1: no booking with that id **in the save's branch** answers
/// `ไม่พบการจองของสาขานี้`.
#[tokio::test]
async fn redeem_refuses_a_booking_of_another_branch() {
    let shop = Shop::open().await;
    let s = &shop.service;
    let main = confirmed("BK-MAIN", MAIN, 0);
    let mut air = confirmed("BK-AIR", AIRPORT, 0);
    air.rate = rate("32.7");
    shop.seed(&main).await;
    shop.seed(&air).await;

    let mut tx = shop.begin().await;
    assert_eq!(
        refusal(
            s.redeem_in(
                &mut tx,
                &branch(AIRPORT),
                claim(&main, "SUB-1", "C-AIR-1"),
                at(60)
            )
            .await
        ),
        NOT_FOUND_HERE
    );
    let mut nobody = claim(&main, "SUB-1", "C-MAIN-1");
    nobody.booking_id = "BK-NONE".to_string();
    nobody.form_booking = "BK-NONE".to_string();
    assert_eq!(
        refusal(s.redeem_in(&mut tx, &branch(MAIN), nobody, at(60)).await),
        NOT_FOUND_HERE
    );
    let step = s
        .redeem_in(
            &mut tx,
            &branch(AIRPORT),
            claim(&air, "SUB-1", "C-AIR-1"),
            at(60),
        )
        .await
        .expect("AIRPORT's own booking");
    assert_eq!(step, RedeemStep::Use {});
    tx.rollback().await.expect("rollback");
    shop.finish().await;
}

/// *Using a booking* row 5: a member's booking is used only for the member's customer, the member
/// read from the save's branch.
#[tokio::test]
async fn redeem_checks_the_member_of_the_booking() {
    let shop = Shop::open().await;
    let s = &shop.service;
    shop.seed_member(&member_row("MB-1", MAIN, "C-MAIN-1", SECRET_A, 0))
        .await;
    let mut booking = pending("BK-M", MAIN, "MB-1", 0);
    booking.status = BookingStatus::Confirmed;
    booking.expires_at = Some(at(1800));
    shop.seed(&booking).await;

    let mut tx = shop.begin().await;
    assert_eq!(
        refusal(
            s.redeem_in(
                &mut tx,
                &branch(MAIN),
                claim(&booking, "SUB-1", "C-MAIN-2"),
                at(60)
            )
            .await
        ),
        MEMBER_MISMATCH
    );
    let step = s
        .redeem_in(
            &mut tx,
            &branch(MAIN),
            claim(&booking, "SUB-1", "C-MAIN-1"),
            at(60),
        )
        .await
        .expect("the member's own customer");
    assert_eq!(step, RedeemStep::Use {});
    tx.rollback().await.expect("rollback");
    shop.finish().await;
}
