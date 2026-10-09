//! Written before the implementation (ticket T8c-2a). Do not edit, delete or weaken.
//!
//! The kiosk's part in the save of an exchange: `KioskSaveStep`, a `SaveStep`
//! registered with `SaveTransactionWorkflow::with_steps`. A save that names a
//! kiosk request is checked against that request before anything is written,
//! and completes the request in the transaction of the save.
//!
//! The rules are the old server's (`beforeSave` / `afterSave` of its kiosk
//! module); the sentences are those of `xc-kiosk`. The kiosk service itself is
//! real and so is the save; the other contexts the kiosk asks (`KioskPorts`)
//! are answered by `Counter` below.
//!
//! These tests need a real PostgreSQL, as `save_transaction_test.rs` does.

#[allow(unused_imports)]
mod save_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{MinorUnit, Rate, Side, UnitSize};
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::commands::new_quote::NewQuote;
use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
use xc_kiosk::domain::kiosk::commands::save_counter_rate::SaveCounterRate;
use xc_kiosk::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use xc_kiosk::domain::kiosk::value_objects::audit_line::{AuditLine, KioskAudit};
use xc_kiosk::domain::kiosk::value_objects::counter_facts::LineId;
use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
use xchange::application::workflows::kiosk_save_step::KioskSaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};

const NOT_READY: &str = "คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว";
const MUST_BE_LIVE: &str = "คิวต้องบันทึกเป็นรายการจริง";
const DIFFERS: &str = "สกุลเงินและจำนวนต้องตรงกับคิว";
const METHOD_DIFFERS: &str = "ช่องทางต้องตรงกับที่ลูกค้าเลือก";

/// One change to a form.
type Change = fn(&mut SaveTransaction);

const B2: &str = "B2";
/// The employee holding the request: an id, never a username.
const FRONT: &str = "employee-7";
const OTHER: &str = "employee-8";

// ------------------------------------------------- the kiosk's other contexts

type Asked<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

/// What the kiosk asks of the other contexts: an open counter buying USD at
/// 32.1, and an audit log kept in memory that can be told to fail.
#[derive(Default)]
struct Counter {
    lines: Mutex<Vec<(String, AuditLine)>>,
    audit_fails: AtomicBool,
}

impl Counter {
    fn view() -> CounterView {
        CounterView {
            branch_name: "Main".to_owned(),
            open: true,
            rates: vec![CounterRate {
                code: code("USD"),
                buy: Rate::parse("32.1").expect("a rate"),
                sell: Rate::parse("33").expect("a rate"),
                // #756 (RE-4b, rates.md §4.9 R1): the port answers the published line.
                unit_size: UnitSize::ONE,
                minor_unit: MinorUnit::TWO,
                line_id: LineId(1),
            }],
        }
    }

    /// The `KIOSK_COMPLETED` lines written so far: (branch, who, document).
    fn completed(&self) -> Vec<(String, String, String)> {
        self.lines
            .lock()
            .expect("lines")
            .iter()
            .filter_map(|(branch, line)| match &line.what {
                KioskAudit::Completed { document_ref } => {
                    Some((branch.clone(), line.actor.clone(), document_ref.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

impl KioskPorts for Counter {
    fn branch_name<'a>(&'a self, _branch: &'a BranchId) -> Asked<'a, Option<String>> {
        Box::pin(async { Ok(Some("Main".to_owned())) })
    }

    fn counter<'a>(&'a self, _branch: &'a BranchId) -> Asked<'a, Option<CounterView>> {
        Box::pin(async { Ok(Some(Self::view())) })
    }

    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> Asked<'a, Option<CounterView>> {
        Box::pin(async move {
            the_line_exists(tx, branch).await?;
            Ok(Some(Self::view()))
        })
    }

    fn save_rate<'a>(
        &'a self,
        _branch: &'a BranchId,
        _rate: &'a SaveCounterRate,
        _actor: &'a str,
        _now: DateTime<Utc>,
    ) -> Asked<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn record_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        _at: DateTime<Utc>,
    ) -> Asked<'a, ()> {
        Box::pin(async move {
            if self.audit_fails.load(Ordering::SeqCst) {
                return Err(AppError::Internal("test: the audit log is down".to_owned()));
            }
            self.lines
                .lock()
                .expect("lines")
                .push((branch.as_str().to_owned(), line.clone()));
            Ok(())
        })
    }
}

// ------------------------------------------------------------------ the scene

/// A staff member: the id is the employee's id, the username something else.
fn staff(id: &str, role: Role) -> SaveActor {
    SaveActor {
        id: id.to_owned(),
        username: format!("user-{id}"),
        employee_id: Some(id.to_owned()),
        role,
    }
}

fn desk(id: &str) -> KioskActor {
    KioskActor::Staff {
        username: id.to_owned(),
        role: KioskRole::Front,
    }
}

fn with_kiosk(request: &str) -> SaveExtras {
    SaveExtras {
        kiosk_request_id: Some(request.to_owned()),
        ..SaveExtras::default()
    }
}

struct Scene {
    shop: Shop,
    counter: Arc<Counter>,
    kiosk: KioskService,
    flow: SaveTransactionWorkflow,
    customer: String,
    /// A passport uploaded by `FRONT`.
    passport: String,
}

impl Scene {
    /// MAIN with a rate, an open shift, a customer, and a workflow whose only
    /// step is the kiosk's.
    async fn start() -> Self {
        let shop = Shop::start().await;
        let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
        let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
        let branch_tx = BranchTx::new(Transactor::new(shop.pool.clone()));
        let counter = Arc::new(Counter::default());
        let kiosk = KioskService::new(branch_tx.clone(), counter.clone());
        let steps: Vec<Arc<dyn SaveStep>> = vec![Arc::new(KioskSaveStep::new(kiosk.clone()))];
        let flow = SaveTransactionWorkflow::new(branch_tx, shop.shifts.clone(), shop.files.clone())
            .with_steps(steps);
        Self {
            shop,
            counter,
            kiosk,
            flow,
            customer,
            passport,
        }
    }

    /// The customer's request at the kiosk: buy 100 USD, paid in cash.
    async fn quoted(&self, branch_id: &str, id: &str) {
        let quote = NewQuote {
            input: QuoteInput {
                side: Some(Side::Buy),
                currency: Some(code("USD")),
                amount: Some(money("100")),
                payment_method: Some("Cash".to_owned()),
            },
            new_id: id.to_owned(),
            client_ip: "203.0.113.7".to_owned(),
        };
        let made = self
            .kiosk
            .quote(&branch(branch_id), quote, at(0))
            .await
            .expect("quote");
        assert_eq!(made.request.quote.foreign, money("100"));
    }

    async fn waiting(&self, branch_id: &str, id: &str) {
        self.quoted(branch_id, id).await;
        self.kiosk
            .confirm(&branch(branch_id), id, at(5))
            .await
            .expect("confirm");
    }

    async fn accepted(&self, branch_id: &str, id: &str, by: &str) {
        self.waiting(branch_id, id).await;
        self.kiosk
            .accept(&branch(branch_id), &desk(by), id, at(6))
            .await
            .expect("accept");
    }

    /// A request whose passport check `by` confirmed: ready to be saved.
    async fn id_checked(&self, branch_id: &str, id: &str, by: &str) {
        self.accepted(branch_id, id, by).await;
        self.kiosk
            .identity_checked(&branch(branch_id), &desk(by), id, true, at(7))
            .await
            .expect("identity checked");
    }

    /// `status|submission|document` of every request, in order.
    async fn requests(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT branch_id || '/' || id || ' ' || status || '|' || \
             coalesce(submission_id, '-') || '|' || coalesce(document_ref, '-') \
             FROM kiosk_requests ORDER BY branch_id, id",
        )
        .fetch_all(&self.shop.pool)
        .await
        .expect("read the requests")
    }

    fn form(&self, submission: &str) -> SaveTransaction {
        form(submission, &self.customer, &self.passport)
    }

    async fn save(
        &self,
        cmd: SaveTransaction,
        extras: &SaveExtras,
        who: &SaveActor,
    ) -> AppResult<SavedTransaction> {
        tokio::time::timeout(
            MUST_FINISH,
            self.flow.save_as(&branch(MAIN), cmd, extras, who, at(60)),
        )
        .await
        .expect("a save finishes")
    }
}

// ------------------------------------------------------------------ the tests

#[tokio::test]
async fn a_save_that_names_no_request_is_none_of_the_kiosks_business() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;
    let front = staff(FRONT, Role::Front);

    // no request named, and a request named with the empty text
    let saved = s
        .save(
            s.form("none-0001-aaaaaaaaaa"),
            &SaveExtras::default(),
            &front,
        )
        .await
        .expect("a save with no kiosk request");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    let saved = s
        .save(s.form("none-0002-aaaaaaaaaa"), &with_kiosk(""), &front)
        .await
        .expect("the empty text names no request");
    assert_eq!(saved.document_ref, "XC-20261006-000002");
    // … not even when the save differs from every request there is
    let mut sell = s.form("none-0003-aaaaaaaaaa");
    sell.side = "SELL".to_owned();
    sell.rate = "33".to_owned();
    s.save(sell, &SaveExtras::default(), &front)
        .await
        .expect("a sale with no kiosk request");

    assert_eq!(s.requests().await, ["MAIN/q1 ID_CHECKED|-|-"]);
    assert!(s.counter.completed().is_empty());
    s.shop.finish().await;
}

#[tokio::test]
async fn the_employee_holding_a_checked_request_saves_it_and_the_request_is_completed() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;
    s.id_checked(MAIN, "q2", FRONT).await;

    let saved = s
        .save(
            s.form("done-0001-aaaaaaaaaa"),
            &with_kiosk("q1"),
            &staff(FRONT, Role::Front),
        )
        .await
        .expect("the holder saves the request");

    assert_eq!(saved.document_ref, "XC-20261006-000001");
    assert_eq!(s.shop.count("transactions").await, 1);
    // the request named is completed and linked; the other one is not touched
    assert_eq!(
        s.requests().await,
        [
            "MAIN/q1 COMPLETED|done-0001-aaaaaaaaaa|XC-20261006-000001",
            "MAIN/q2 ID_CHECKED|-|-",
        ]
    );
    // the audit line names the saver by id, not by username
    assert_eq!(
        s.counter.completed(),
        [(
            MAIN.to_owned(),
            FRONT.to_owned(),
            "XC-20261006-000001".to_owned()
        )]
    );
    s.shop.finish().await;
}

#[tokio::test]
async fn the_role_of_the_saver_plays_no_part() {
    // the old server asked who holds the request, not what they are
    let s = Scene::start().await;
    for (n, role) in [Role::Owner, Role::Manager, Role::Rates, Role::Documents]
        .into_iter()
        .enumerate()
    {
        let id = format!("r{n}");
        s.id_checked(MAIN, &id, FRONT).await;
        s.save(
            s.form(&format!("role-000{n}-aaaaaaaaaa")),
            &with_kiosk(&id),
            &staff(FRONT, role),
        )
        .await
        .unwrap_or_else(|e| panic!("{role:?} holding the request saves it: {e:?}"));
    }
    assert_eq!(s.counter.completed().len(), 4);
    s.shop.finish().await;
}

#[tokio::test]
async fn a_request_that_is_not_ready_or_not_the_savers_refuses_the_save() {
    let s = Scene::start().await;
    s.shop.seed_branch(B2, "Branch two").await;
    s.quoted(MAIN, "quoted").await;
    s.waiting(MAIN, "waiting").await;
    s.accepted(MAIN, "accepted", FRONT).await;
    s.id_checked(MAIN, "others", OTHER).await;
    s.id_checked(B2, "elsewhere", FRONT).await;
    // held by somebody whose id is this saver's *username*
    s.id_checked(MAIN, "by-name", &format!("user-{FRONT}"))
        .await;
    let front = staff(FRONT, Role::Front);
    let before = (s.shop.everything().await, s.requests().await);

    for request in [
        "no-such-request",
        "quoted",
        "waiting",
        "accepted",
        "others",
        "elsewhere",
        "by-name",
        " others",
        "%",
    ] {
        let got = s
            .save(s.form("deny-0001-aaaaaaaaaa"), &with_kiosk(request), &front)
            .await;
        assert_eq!(refusal(got), NOT_READY, "request {request:?}");
    }
    assert_eq!((s.shop.everything().await, s.requests().await), before);
    // the owner does not hold it either
    let owner = SaveActor {
        id: OWNER.to_owned(),
        username: FRONT.to_owned(),
        employee_id: None,
        role: Role::Owner,
    };
    let owners = s.shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    let before = (s.shop.everything().await, s.requests().await);
    let got = s
        .save(
            form("deny-0001-aaaaaaaaaa", &s.customer, &owners),
            &with_kiosk("others"),
            &owner,
        )
        .await;
    assert_eq!(refusal(got), NOT_READY);
    assert_eq!((s.shop.everything().await, s.requests().await), before);
    assert!(s.counter.completed().is_empty());
    s.shop.finish().await;
}

#[tokio::test]
async fn the_save_must_be_live_and_be_what_the_customer_asked_for() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;
    s.id_checked(MAIN, "q2", OTHER).await;
    let front = staff(FRONT, Role::Front);
    let slip = s.shop.upload(MAIN, FRONT, "application/pdf", PDF).await;
    let before = (s.shop.everything().await, s.requests().await);
    let change = |f: Change| {
        let mut cmd = s.form("diff-0001-aaaaaaaaaa");
        f(&mut cmd);
        if cmd.method != "Cash" {
            cmd.payment_ref = "REF-1".to_owned();
            cmd.slip = Some(slip.clone());
        }
        cmd
    };

    let cases: [(&str, Change, &str); 9] = [
        (
            "demo",
            |c| c.customer.mode = "DEMO".to_owned(),
            MUST_BE_LIVE,
        ),
        ("currency", |c| c.currency = "EUR".to_owned(), DIFFERS),
        ("no currency", |c| c.currency = "ZZZ".to_owned(), DIFFERS),
        ("side", |c| c.side = "SELL".to_owned(), DIFFERS),
        ("more", |c| c.amount = "100.01".to_owned(), DIFFERS),
        ("less", |c| c.amount = "99.99".to_owned(), DIFFERS),
        (
            "method",
            |c| c.method = "Bank Transfer".to_owned(),
            METHOD_DIFFERS,
        ),
        // which sentence when several things are wrong: live, then the
        // amounts, then the method
        (
            "demo and amount",
            |c| {
                c.customer.mode = "DEMO".to_owned();
                c.amount = "5".to_owned();
            },
            MUST_BE_LIVE,
        ),
        (
            "amount and method",
            |c| {
                c.amount = "5".to_owned();
                c.method = "Bank Transfer".to_owned();
            },
            DIFFERS,
        ),
    ];
    for (what, f, sentence) in cases {
        let got = s.save(change(f), &with_kiosk("q1"), &front).await;
        assert_eq!(refusal(got), sentence, "{what}");
    }
    // whose request it is comes before all of them
    let got = s
        .save(
            change(|c| c.customer.mode = "DEMO".to_owned()),
            &with_kiosk("q2"),
            &front,
        )
        .await;
    assert_eq!(refusal(got), NOT_READY);
    // 100.00 is 100
    assert_eq!((s.shop.everything().await, s.requests().await), before);
    s.save(
        change(|c| c.amount = "100.00".to_owned()),
        &with_kiosk("q1"),
        &front,
    )
    .await
    .expect("the same amount written with decimals");
    s.shop.finish().await;
}

#[tokio::test]
async fn sending_the_same_save_again_is_answered_and_completes_nothing_twice() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;
    let front = staff(FRONT, Role::Front);
    let first = s
        .save(s.form("again-0001-aaaaaaaaa"), &with_kiosk("q1"), &front)
        .await
        .expect("the first sending");
    let after_first = (s.shop.everything().await, s.requests().await);

    let again = s
        .save(s.form("again-0001-aaaaaaaaa"), &with_kiosk("q1"), &front)
        .await
        .expect("the same sending again");
    assert_eq!(again.document_ref, first.document_ref);
    assert_eq!((s.shop.everything().await, s.requests().await), after_first);
    assert_eq!(s.counter.completed().len(), 1);

    // a completed request takes no other exchange, from its holder or anybody
    let got = s
        .save(s.form("again-0002-aaaaaaaaa"), &with_kiosk("q1"), &front)
        .await;
    assert_eq!(refusal(got), NOT_READY);
    // … and the first exchange is not answered again to somebody else
    assert_eq!((s.shop.everything().await, s.requests().await), after_first);
    let others = s.shop.upload(MAIN, OTHER, "image/jpeg", JPEG).await;
    let got = s
        .save(
            form("again-0001-aaaaaaaaa", &s.customer, &others),
            &with_kiosk("q1"),
            &staff(OTHER, Role::Front),
        )
        .await;
    assert_eq!(refusal(got), NOT_READY);
    assert_eq!(s.shop.count("transactions").await, 1);
    assert_eq!(s.requests().await, after_first.1);
    s.shop.finish().await;
}

#[tokio::test]
async fn a_save_the_shop_refuses_leaves_the_request_ready() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;
    let front = staff(FRONT, Role::Front);
    let before = (s.shop.everything().await, s.requests().await);

    // the kiosk agrees, the counter rate has moved since
    s.shop.set_rate(MAIN, "USD", "32.2", "33").await;
    let got = s
        .save(s.form("shop-0001-aaaaaaaaaa"), &with_kiosk("q1"), &front)
        .await;
    assert_eq!(refusal(got), RATE_MISMATCH);
    assert_eq!((s.shop.everything().await, s.requests().await), before);
    assert!(s.counter.completed().is_empty());

    // the same request is saved once the rate is back
    s.shop.set_rate(MAIN, "USD", "32.1", "33").await;
    s.save(s.form("shop-0001-aaaaaaaaaa"), &with_kiosk("q1"), &front)
        .await
        .expect("the request is still ready");
    assert_eq!(
        s.requests().await,
        ["MAIN/q1 COMPLETED|shop-0001-aaaaaaaaaa|XC-20261006-000001"]
    );
    s.shop.finish().await;
}

#[tokio::test]
async fn a_request_that_cannot_be_completed_undoes_the_save() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;
    let front = staff(FRONT, Role::Front);
    let before = (s.shop.everything().await, s.requests().await);

    s.counter.audit_fails.store(true, Ordering::SeqCst);
    let got = s
        .save(s.form("undo-0001-aaaaaaaaaa"), &with_kiosk("q1"), &front)
        .await;
    assert!(
        matches!(got, Err(AppError::Internal(_))),
        "the failure is passed on as it is: {got:?}"
    );
    // no exchange, no document number used, the request as it was
    assert_eq!((s.shop.everything().await, s.requests().await), before);

    s.counter.audit_fails.store(false, Ordering::SeqCst);
    let saved = s
        .save(s.form("undo-0001-aaaaaaaaaa"), &with_kiosk("q1"), &front)
        .await
        .expect("the same save once the audit log is back");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    assert_eq!(s.counter.completed().len(), 1);
    s.shop.finish().await;
}

/// #756 (RE-4b, migration 5): a stored quote references its line, so the counter's one line is
/// a row of `published_rate_lines` (with its approved request and publication), written in the
/// caller's transaction the first time the counter is read.
async fn the_line_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> AppResult<()> {
    let line = LineId(1).to_string();
    let request = format!("{line}-{}", branch.as_str());
    sqlx::query(
        "WITH r AS ( \
           INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
               object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
               status, idempotency_key) \
           SELECT gen_random_uuid(), organization_id, 'RATE_PUBLICATION', id, $1::uuid, 'test', \
               '1', '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
           FROM branches WHERE id = $2 \
             AND NOT EXISTS (SELECT 1 FROM published_rate_lines WHERE id = $1::uuid) \
           RETURNING id, organization_id, branch_id), \
         p AS ( \
           INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
               approved_request_id, draft_id, published_by) \
           SELECT $1::uuid, r.organization_id, r.branch_id, \
               COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = r.branch_id), 0) + 1, \
               now(), r.id, NULL, 'test' FROM r \
           RETURNING id, organization_id) \
         INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT p.id, p.organization_id, p.id, 'USD', 'ALL', 1, 32.1, 33, \
             '{\"buy_mode\": \"MANUAL\", \"sell_mode\": \"MANUAL\"}'::jsonb FROM p",
    )
    .bind(&line)
    .bind(branch.as_str())
    .bind(&request)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    Ok(())
}
