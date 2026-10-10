//! Ticket #183: tests that the KIOSK_COMPLETED audit line carries the save's time.

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

const FRONT: &str = "employee-7";

type Asked<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

#[derive(Default)]
struct Counter {
    lines: Mutex<Vec<(String, AuditLine, DateTime<Utc>)>>,
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

    /// The `KIOSK_COMPLETED` lines written so far: (branch, who, document, at).
    fn completed(&self) -> Vec<(String, String, String, DateTime<Utc>)> {
        self.lines
            .lock()
            .expect("lines")
            .iter()
            .filter_map(|(branch, line, at)| match &line.what {
                KioskAudit::Completed { document_ref } => Some((
                    branch.clone(),
                    line.actor.clone(),
                    document_ref.clone(),
                    *at,
                )),
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
        at: DateTime<Utc>,
    ) -> Asked<'a, ()> {
        Box::pin(async move {
            if self.audit_fails.load(Ordering::SeqCst) {
                return Err(AppError::Internal("test: the audit log is down".to_owned()));
            }
            self.lines
                .lock()
                .expect("lines")
                .push((branch.as_str().to_owned(), line.clone(), at));
            Ok(())
        })
    }

    /// #769: `KioskPorts::check_stale_in` has no default body, so this fake implements it. It
    /// admits every quote, the behaviour the fake had through the default.
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
}

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
    passport: String,
}

impl Scene {
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

    async fn id_checked(&self, branch_id: &str, id: &str, by: &str) {
        self.accepted(branch_id, id, by).await;
        self.kiosk
            .identity_checked(&branch(branch_id), &desk(by), id, true, at(7))
            .await
            .expect("identity checked");
    }

    fn form(&self, submission: &str) -> SaveTransaction {
        form(submission, &self.customer, &self.passport)
    }

    async fn save_at(
        &self,
        cmd: SaveTransaction,
        extras: &SaveExtras,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        tokio::time::timeout(
            MUST_FINISH,
            self.flow.save_as(&branch(MAIN), cmd, extras, who, now),
        )
        .await
        .expect("a save finishes")
    }
}

#[tokio::test]
async fn kiosk_completed_audit_line_carries_the_saves_time() {
    let s = Scene::start().await;
    s.id_checked(MAIN, "q1", FRONT).await;

    let save_now = at(60);
    assert_ne!(
        save_now.timestamp_subsec_nanos() % 1_000_000,
        0,
        "save_now must have sub-millisecond digits"
    );

    let saved = s
        .save_at(
            s.form("done-0001-aaaaaaaaaa"),
            &with_kiosk("q1"),
            &staff(FRONT, Role::Front),
            save_now,
        )
        .await
        .expect("the holder saves the request");

    assert_eq!(saved.document_ref, "XC-20261006-000001");
    assert_eq!(s.shop.count("transactions").await, 1);

    let completed = s.counter.completed();
    assert_eq!(completed.len(), 1);
    let (branch_recorded, who_recorded, doc_recorded, at_recorded) = &completed[0];
    assert_eq!(branch_recorded, MAIN);
    assert_eq!(who_recorded, FRONT);
    assert_eq!(doc_recorded, "XC-20261006-000001");
    assert_eq!(*at_recorded, millis(save_now));
    assert_ne!(*at_recorded, save_now);

    let repeat_now = at(120);
    let repeat = s
        .save_at(
            s.form("done-0001-aaaaaaaaaa"),
            &with_kiosk("q1"),
            &staff(FRONT, Role::Front),
            repeat_now,
        )
        .await
        .expect("repeat of same submission succeeds");

    assert_eq!(repeat.document_ref, "XC-20261006-000001");
    let completed_after = s.counter.completed();
    assert_eq!(completed_after.len(), 1);
    assert_eq!(completed_after[0].3, millis(save_now));

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
