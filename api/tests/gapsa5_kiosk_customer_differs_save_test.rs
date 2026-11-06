//! XC-BE-GAPS-A row 5: the desk save tells the kiosk whose passport it saved on (D13, D20).
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 5, 2026-10-27).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Decided, and pinned here (`docs/design/blueprint/kiosk-shared-phone.md`):
//! - D13 (`:229`, B16 `:391`): a save that names a kiosk request whose member is B, saved on the passport's
//!   customer A ≠ B, is saved on A; the request keeps B; after `KIOSK_COMPLETED` comes one `KIOSK_CUSTOMER_DIFFERS`
//!   `{saved_customer: A, request_customer: B}` by the saver, on the request, at the save's time. `KioskSaveStep`
//!   hands `complete_saved_in` the saved exchange's `customer_id`.
//! - D20 (`:236`): a save whose form passport is the desk's skip (`WALKIN` / `XXX`, `web/src/screens/desk/steps/
//!   customer/step.tsx` `onSkip`, `step.test.tsx:402`), recognised by `WalkInPlaceholder::is`, hands `None`: never
//!   a different customer, no line, the member stays.
//! - The line is written on the save's own transaction, where the request already stands `COMPLETED` (as
//!   `api/tests/kiosk_http_common/mod.rs:221-233` checks): the `Counter` below refuses it otherwise.
//! - A repeat of the same save writes no line at all.
//!
//! The scene is `kiosk_save_step_time_test.rs`'s (copied): a real save and a real kiosk service; the other contexts
//! the kiosk asks are answered by `Counter`, whose audit log is kept in memory.
//! These tests need a real PostgreSQL, as `save_transaction_test.rs` does.

#[allow(unused_imports)]
mod save_common;
mod shift_holder_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use save_common::*;
use shift_holder_common::user_of;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{MinorUnit, Rate, Side, UnitSize};
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::commands::new_quote::NewQuote;
use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
use xc_kiosk::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use xc_kiosk::domain::kiosk::value_objects::audit_line::{AuditLine, KioskAudit};
use xc_kiosk::domain::kiosk::value_objects::counter_facts::LineId;
use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
use xchange::application::workflows::kiosk_save_step::KioskSaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};

const FRONT: &str = "employee-7";
/// The request's member, a customer other than the passport's.
const MEMBER: &str = "member-B";

type Asked<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

#[derive(Default)]
struct Counter {
    lines: Mutex<Vec<(String, AuditLine, DateTime<Utc>)>>,
    pool: std::sync::OnceLock<sqlx::PgPool>,
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
                unit_size: UnitSize::ONE,
                minor_unit: MinorUnit::TWO,
                line_id: LineId(1),
            }],
        }
    }

    /// The lines of the save's two kinds, in order: (kind, who, request, what, at).
    fn save_lines(&self) -> Vec<(String, String, String, KioskAudit, DateTime<Utc>)> {
        self.lines
            .lock()
            .expect("lines")
            .iter()
            .filter(|(_, line, _)| {
                matches!(
                    line.what,
                    KioskAudit::Completed { .. } | KioskAudit::CustomerDiffers { .. }
                )
            })
            .map(|(_, line, at)| {
                (
                    line.what.kind().to_owned(),
                    line.actor.clone(),
                    line.subject.clone(),
                    line.what.clone(),
                    *at,
                )
            })
            .collect()
    }

    fn differs(&self) -> Vec<KioskAudit> {
        self.save_lines()
            .into_iter()
            .filter(|l| l.0 == "KIOSK_CUSTOMER_DIFFERS")
            .map(|l| l.3)
            .collect()
    }
}

impl KioskPorts for Counter {
    fn branch_name<'a>(&'a self, _branch: &'a BranchId) -> Asked<'a, Option<String>> {
        Box::pin(async { Ok(Some("Main".to_owned())) })
    }

    fn counter<'a>(
        &'a self,
        _branch: &'a BranchId,
        _now: DateTime<Utc>,
    ) -> Asked<'a, Option<CounterView>> {
        Box::pin(async { Ok(Some(Self::view())) })
    }

    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        _now: DateTime<Utc>,
    ) -> Asked<'a, Option<CounterView>> {
        Box::pin(async move {
            the_line_exists(tx, branch).await?;
            Ok(Some(Self::view()))
        })
    }

    /// Keeps the line; a `KIOSK_CUSTOMER_DIFFERS` line must come on the save's transaction, where the request
    /// already stands `COMPLETED` (`kiosk_http_common/mod.rs:221-233`).
    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> Asked<'a, ()> {
        Box::pin(async move {
            if matches!(line.what, KioskAudit::CustomerDiffers { .. }) {
                let seen: Option<String> = sqlx::query_scalar(
                    "SELECT status FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
                )
                .bind(branch.as_str())
                .bind(&line.subject)
                .fetch_optional(&mut **tx)
                .await?;
                if seen.as_deref() != Some("COMPLETED") {
                    return Err(AppError::Internal(format!(
                        "test: KIOSK_CUSTOMER_DIFFERS for {} while it stands {seen:?}",
                        line.subject
                    )));
                }
            }
            self.lines
                .lock()
                .expect("lines")
                .push((branch.as_str().to_owned(), line.clone(), at));
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn check_stale_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _line_id: LineId,
        _currency: xc_kernel::money::CurrencyCode,
        _side: Side,
        _thb: xc_kernel::money::Money,
        _now: DateTime<Utc>,
    ) -> Asked<'a, bool> {
        Box::pin(async { Ok(true) })
    }

    fn quote_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        currency: xc_kernel::money::CurrencyCode,
        side: Side,
        quantity: xc_kernel::money::ForeignAmount,
        now: DateTime<Utc>,
    ) -> Asked<'a, Option<String>> {
        Box::pin(async move {
            let pool = self
                .pool
                .get()
                .expect("the scene gave the counter the shop's pool");
            kiosk_quote_in(pool, tx, branch, currency, side, quantity, now).await
        })
    }
}

fn teller(id: &str) -> SaveActor {
    SaveActor {
        id: id.to_owned(),
        username: FRONT.to_owned(),
        employee_id: Some(id.to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
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

/// The passport the desk's skip sends (`step.tsx` `onSkip`).
fn walk_in() -> ReserveCustomer {
    ReserveCustomer {
        name: "WALK-IN".to_owned(),
        passport_no: "WALKIN".to_owned(),
        nationality: "XXX".to_owned(),
        birth_date: String::new(),
        passport_expiry: String::new(),
        mode: "LIVE".to_owned(),
    }
}

struct Scene {
    shop: Shop,
    counter: Arc<Counter>,
    kiosk: KioskService,
    flow: SaveTransactionWorkflow,
    /// The passport's customer (`person()`).
    customer: String,
    passport: String,
    front_id: String,
}

impl Scene {
    async fn start() -> Self {
        let shop = Shop::start().await;
        let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
        let shift = shift_holder_common::open_shift_for(
            &shop.pool,
            MAIN,
            FRONT,
            &[("THB", "100000"), ("USD", "1000")],
        )
        .await;
        fund_ledger_drawer(&shop.pool, &shift, &[("THB", "100000"), ("USD", "1000")]).await;
        let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
        let front_id = user_of(&shop.pool, FRONT).await;
        let branch_tx = BranchTx::new(Transactor::new(shop.pool.clone()));
        let counter = Arc::new(Counter::default());
        counter
            .pool
            .set(shop.pool.clone())
            .expect("the counter is new");
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
            front_id,
        }
    }

    /// Request `id` (buy 100 USD, cash), checked by the front, ready to be saved; its member is `member`.
    async fn checked_with_member(&self, id: &str, member: Option<&str>) {
        let b = branch(MAIN);
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
        self.kiosk.quote(&b, quote, at(0)).await.expect("quote");
        self.kiosk.confirm(&b, id, at(5)).await.expect("confirm");
        self.kiosk
            .accept(&b, &desk(&self.front_id), id, at(6))
            .await
            .expect("accept");
        self.kiosk
            .identity_checked(&b, &desk(&self.front_id), id, true, at(7))
            .await
            .expect("identity checked");
        if let Some(member) = member {
            let done = sqlx::query(
                "UPDATE kiosk_requests SET customer_id = $1 WHERE branch_id = $2 AND id = $3",
            )
            .bind(member)
            .bind(MAIN)
            .bind(id)
            .execute(&self.shop.pool)
            .await
            .expect("set the request's member");
            assert_eq!(done.rows_affected(), 1);
        }
    }

    async fn member_of(&self, id: &str) -> Option<String> {
        sqlx::query_scalar(
            "SELECT customer_id FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
        )
        .bind(MAIN)
        .bind(id)
        .fetch_one(&self.shop.pool)
        .await
        .expect("read the request's member")
    }

    /// The customer the saved exchange is on.
    async fn saved_on(&self, document_ref: &str) -> String {
        sqlx::query_scalar("SELECT customer_id FROM transactions WHERE document_ref = $1")
            .bind(document_ref)
            .fetch_one(&self.shop.pool)
            .await
            .expect("read the saved exchange")
    }

    /// The form on the passport's customer (`person()`).
    fn form(&self, submission: &str) -> SaveTransaction {
        form(submission, &self.customer, &self.passport)
    }

    /// The form of the desk's skip, on the placeholder customer `walk_in_id`.
    fn walk_in_form(&self, submission: &str, walk_in_id: &str) -> SaveTransaction {
        let mut cmd = form(submission, walk_in_id, &self.passport);
        cmd.customer = walk_in();
        cmd
    }

    async fn save_at(
        &self,
        cmd: SaveTransaction,
        request: &str,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        tokio::time::timeout(
            MUST_FINISH,
            self.flow.save_as(
                &branch(MAIN),
                cmd,
                &with_kiosk(request),
                &teller(&self.front_id),
                now,
            ),
        )
        .await
        .expect("a save finishes")
    }

    fn differs_line(&self, saved: &str) -> KioskAudit {
        KioskAudit::CustomerDiffers {
            saved_customer: saved.to_owned(),
            request_customer: MEMBER.to_owned(),
        }
    }
}

/// B16: saved on the passport's customer, the request's member is another ⇒ after `KIOSK_COMPLETED`, one
/// `KIOSK_CUSTOMER_DIFFERS` with the saved exchange's customer and the member, by the saver, at the save's time;
/// the exchange is on the passport's customer and the request keeps its member.
#[tokio::test]
async fn gapsa5_a_save_on_another_customer_than_the_member_writes_customer_differs() {
    let s = Scene::start().await;
    s.checked_with_member("q1", Some(MEMBER)).await;

    let now = at(60);
    let saved = s
        .save_at(s.form("gapa-0001-aaaaaaaaaa"), "q1", now)
        .await
        .expect("the holder saves the request");

    let on = s.saved_on(&saved.document_ref).await;
    assert_eq!(on, s.customer, "saved on the passport's customer");
    let completed = KioskAudit::Completed {
        document_ref: saved.document_ref.clone(),
    };
    assert_eq!(
        s.counter.save_lines(),
        [
            (
                "KIOSK_COMPLETED".to_owned(),
                s.front_id.clone(),
                "q1".to_owned(),
                completed,
                millis(now),
            ),
            (
                "KIOSK_CUSTOMER_DIFFERS".to_owned(),
                s.front_id.clone(),
                "q1".to_owned(),
                s.differs_line(&on),
                millis(now),
            ),
        ]
    );
    assert_eq!(s.member_of("q1").await.as_deref(), Some(MEMBER));
    s.shop.finish().await;
}

/// D20: the desk's skip (`WALKIN` / `XXX`) is never a different customer: no line, the member stays. Beside it, a
/// save of the same shape on a real passport gets its line, so the rule is seen to run.
#[tokio::test]
async fn gapsa5_a_save_on_the_desk_skip_writes_no_customer_differs() {
    let s = Scene::start().await;
    let walk_in_id = s.shop.reserve(MAIN, walk_in()).await;
    assert_ne!(walk_in_id, s.customer);
    s.checked_with_member("q1", Some(MEMBER)).await;
    s.checked_with_member("q2", Some(MEMBER)).await;

    let skipped = s
        .save_at(
            s.walk_in_form("gapa-0002-aaaaaaaaaa", &walk_in_id),
            "q1",
            at(60),
        )
        .await
        .expect("the skip saves the request");
    assert_eq!(s.saved_on(&skipped.document_ref).await, walk_in_id);
    assert!(
        s.counter.differs().is_empty(),
        "the skip placeholder is never a different customer: {:?}",
        s.counter.differs()
    );
    assert_eq!(s.member_of("q1").await.as_deref(), Some(MEMBER));

    s.save_at(s.form("gapa-0003-aaaaaaaaaa"), "q2", at(70))
        .await
        .expect("a save on a real passport");
    assert_eq!(s.counter.differs(), [s.differs_line(&s.customer)]);
    let subjects: Vec<String> = s
        .counter
        .save_lines()
        .into_iter()
        .map(|l| format!("{} {}", l.0, l.2))
        .collect();
    assert_eq!(
        subjects,
        [
            "KIOSK_COMPLETED q1",
            "KIOSK_COMPLETED q2",
            "KIOSK_CUSTOMER_DIFFERS q2"
        ]
    );
    s.shop.finish().await;
}

/// A save on the request's own member writes no line; a request with no member (a kiosk request) neither. Beside
/// them, a request whose member is another customer gets its line.
#[tokio::test]
async fn gapsa5_a_save_on_the_member_or_on_no_member_writes_no_customer_differs() {
    let s = Scene::start().await;
    let customer = s.customer.clone();
    s.checked_with_member("q1", Some(&customer)).await;
    s.checked_with_member("q2", None).await;
    s.checked_with_member("q3", Some(MEMBER)).await;

    s.save_at(s.form("gapa-0004-aaaaaaaaaa"), "q1", at(60))
        .await
        .expect("saved on the member");
    s.save_at(s.form("gapa-0005-aaaaaaaaaa"), "q2", at(70))
        .await
        .expect("saved, no member");
    assert!(s.counter.differs().is_empty(), "{:?}", s.counter.differs());
    assert_eq!(s.member_of("q1").await, Some(customer));
    assert_eq!(s.member_of("q2").await, None);

    s.save_at(s.form("gapa-0006-aaaaaaaaaa"), "q3", at(80))
        .await
        .expect("saved on another customer");
    assert_eq!(s.counter.differs(), [s.differs_line(&s.customer)]);
    s.shop.finish().await;
}

/// A repeat of the same save changes nothing and writes no line at all: one `KIOSK_COMPLETED`, one
/// `KIOSK_CUSTOMER_DIFFERS`, as after the first.
#[tokio::test]
async fn gapsa5_a_repeat_save_writes_no_second_customer_differs() {
    let s = Scene::start().await;
    s.checked_with_member("q1", Some(MEMBER)).await;

    let first = s
        .save_at(s.form("gapa-0007-aaaaaaaaaa"), "q1", at(60))
        .await
        .expect("the save");
    let after_first = s.counter.save_lines();
    assert_eq!(after_first.len(), 2, "{after_first:?}");
    let repeat = s
        .save_at(s.form("gapa-0007-aaaaaaaaaa"), "q1", at(120))
        .await
        .expect("the same save again");
    assert_eq!(repeat.document_ref, first.document_ref);
    assert_eq!(s.counter.save_lines(), after_first);
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
