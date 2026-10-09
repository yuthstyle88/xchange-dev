//! OR-6 (`docs/design/blueprint/org-roles-branches.md` §6): `CustodyService` asks its ports for
//! the branch's service window at the command's own time and hands that answer to the machine,
//! both for `ops*` commands and for the save gate (`check_save_in`).
//!
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! The ports here answer a window of 15:00–23:00 in Asia/Tokyo on 2026-10-06; the desk's own
//! clock is 12:xx in Bangkok (14:xx in Tokyo), inside the old 11:00–22:50 rule and outside this
//! window. A service that still reads the Bangkok clock, or that does not use what the ports
//! answered, gets these wrong.
//!
//! This file does not compile before OR-6: `ServiceWindow` and
//! `CustodyPorts::service_window` are what the ticket adds.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use custody_common::{PriorSave, SaveAttempt, actor};
use custody_service_common::{Desk, FakeOutside};
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::json;
use shift_service_common::{TestDb, code, money, refusal};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::value_objects::custody_error::CustodyError;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

/// `hh:mm` on 2026-10-06 in Asia/Tokyo (UTC+9, no daylight saving).
fn tokyo(hhmm: &str) -> DateTime<Utc> {
    let (h, m) = hhmm.split_once(':').expect("hh:mm");
    FixedOffset::east_opt(9 * 3600)
        .expect("UTC+9")
        .with_ymd_and_hms(2026, 10, 6, h.parse().unwrap(), m.parse().unwrap(), 0)
        .single()
        .expect("a time in Tokyo")
        .with_timezone(&Utc)
}

fn ct(t: DateTime<Utc>) -> CustodyTime {
    CustodyTime::from_epoch_millis(t.timestamp_millis())
}

/// The test's ports: the usual fake for everything, and one fixed window, recording the times
/// it was asked about.
struct Windowed {
    inner: Arc<FakeOutside>,
    window: ServiceWindow,
    asked: Mutex<Vec<(String, CustodyTime)>>,
}

impl CustodyPorts for Windowed {
    fn outside_facts<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        self.inner.outside_facts(tx, branch, open_shift_id)
    }

    fn customer_known<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        self.inner.customer_known(tx, branch, customer_id)
    }

    fn evidence_owned<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        owner_id: &'a str,
        keys: &'a [String],
    ) -> BoxFuture<'a, AppResult<bool>> {
        self.inner.evidence_owned(tx, branch, owner_id, keys)
    }

    fn employee_role<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CustodyRole>>> {
        self.inner.employee_role(tx, branch, employee_id)
    }

    fn service_window<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<ServiceWindow>> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap()
                .push((branch.as_str().to_owned(), now));
            Ok(self.window)
        })
    }
}

/// The desk with its drawer committed (phase `Ready`), then served by ports that answer a
/// Tokyo 15:00–23:00 window.
async fn ready_in_tokyo(db: &TestDb) -> (Desk, Arc<Windowed>) {
    let (mut d, id) = Desk::opening_proposed(db).await;
    d.ok("BE1", "opsCommit", json!({ "id": id })).await;
    let ports = Arc::new(Windowed {
        inner: d.outside.clone(),
        window: ServiceWindow::Open {
            open: ct(tokyo("15:00")),
            close: ct(tokyo("23:00")),
        },
        asked: Mutex::new(Vec::new()),
    });
    d.custody = CustodyService::new(
        Arc::new(PgCustodyRepository::new(db.pool.clone())),
        db.service(),
        ports.clone(),
        db.branch_tx(),
    );
    (d, ports)
}

async fn command_at(
    d: &Desk,
    who: &str,
    action: &str,
    at: DateTime<Utc>,
) -> AppResult<serde_json::Value> {
    let payload = match action {
        "opsStartService" => json!({
            "equipmentReady": true, "ratesVerified": true,
            "rateUpdated": "2026-10-06T03:00:00.000Z",
        }),
        "opsBeginTransaction" => json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        other => panic!("no payload for {other}"),
    };
    d.custody
        .command(&d.branch, action, &payload, &actor(who), at)
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_command_is_judged_by_the_window_the_ports_answer_for_its_time() {
    let db = TestDb::start().await;
    let (d, ports) = ready_in_tokyo(&db).await;

    // 14:59 in Tokyo is 12:59 in Bangkok: inside the old rule, outside this branch's window
    assert_eq!(
        refusal(command_at(&d, "BE1", "opsStartService", tokyo("14:59")).await),
        CustodyError::ServiceNotReady.to_string()
    );
    let asked = ports.asked.lock().unwrap().clone();
    assert_eq!(
        asked.last(),
        Some(&(d.branch.as_str().to_owned(), ct(tokyo("14:59")))),
        "the ports are asked about this branch at the command's own time"
    );

    command_at(&d, "BE1", "opsStartService", tokyo("15:00"))
        .await
        .expect("15:00 in Tokyo opens this branch's service");

    assert_eq!(
        refusal(command_at(&d, "F1", "opsBeginTransaction", tokyo("22:50")).await),
        CustodyError::OutsideServiceHours.to_string()
    );
    command_at(&d, "F1", "opsBeginTransaction", tokyo("22:49"))
        .await
        .expect("22:49 in Tokyo is before close − 10 min");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_save_gate_is_judged_by_the_window_the_ports_answer_for_its_time() {
    let db = TestDb::start().await;
    let (d, ports) = ready_in_tokyo(&db).await;
    command_at(&d, "BE1", "opsStartService", tokyo("15:00"))
        .await
        .expect("the service opens");

    let gate = |at: DateTime<Utc>| {
        let (service, branch) = (d.custody.clone(), d.branch.clone());
        let tx = db.branch_tx();
        async move {
            let b = branch.clone();
            tx.run(&branch, move |tx| {
                Box::pin(async move {
                    let save = SaveAttempt {
                        customer_id: "CX-00000001".into(),
                        draft_id: None,
                        side: Side::Buy,
                        currency: code("USD"),
                        amount: money("100"),
                        prior: PriorSave::None,
                    };
                    service.check_save_in(tx, &b, &save, &actor("F1"), at).await
                })
            })
            .await
        }
    };

    gate(tokyo("22:49"))
        .await
        .expect("a new save before 22:50 Tokyo");
    // 22:51 in Tokyo is 20:51 in Bangkok: no draft is needed by the old rule
    assert_eq!(
        refusal(gate(tokyo("22:51")).await),
        CustodyError::SaveClosed.to_string()
    );
    // 14:30 in Tokyo is 12:30 in Bangkok: inside the old rule, before this branch opens
    assert_eq!(
        refusal(gate(tokyo("14:30")).await),
        CustodyError::SaveClosed.to_string()
    );
    let asked = ports.asked.lock().unwrap().clone();
    assert_eq!(
        asked.last(),
        Some(&(d.branch.as_str().to_owned(), ct(tokyo("14:30")))),
        "the save gate asks about this branch at the save's own time"
    );
    db.drop_schema().await;
}
