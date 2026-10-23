//! KWQ-C5 — an unscanned web request past its window (design `docs/design/blueprint/kiosk-shared-phone.md`
//! §4.2 D2, D8; §4.4 B4, B12): a `QUOTED` web request older than `WEB_REQUEST_LIFE_MS` is gone for
//! the member. It does not block a new request (`alreadyQueued`), and it cannot be given a fresh QR.
//! The window is `KioskMachine::is_purgeable_for(.., web = true, ..)`, the same one
//! `find_by_token` already applies.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks) after the PR 4481
//! review (issuecomment-6328870087). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;
mod kwq_common;

use std::sync::Arc;

use kiosk_service_common::*;
use kwq_common::RecordingQueue;
use pos_error::AppError;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kiosk::application::kiosk::web_request::WebRequestService;

const MINUTE: i64 = 60_000;
/// Just past the 60-minute web window.
const PAST_WINDOW: i64 = 61 * MINUTE;

struct World {
    db: TestDb,
    kiosk: KioskService,
    web: WebRequestService,
}

async fn world() -> World {
    let db = TestDb::start().await;
    let kiosk = db.service(FakePorts::standard());
    let queue = Arc::new(RecordingQueue::default());
    let web = WebRequestService::new(kiosk.clone(), queue as Arc<dyn QueueTickets>);
    World { db, kiosk, web }
}

impl World {
    async fn create(
        &self,
        customer: &str,
        id: &str,
        token: &str,
        ms: i64,
    ) -> Result<String, AppError> {
        let mut tx = self.db.begin().await;
        let ip = format!("198.51.100.{}", id.len() + token.len());
        let out = self
            .web
            .create_in(
                &mut tx,
                &branch(MAIN),
                customer,
                from_ip(buy100(id), &ip),
                token,
                at(ms),
            )
            .await;
        match out {
            Ok(created) => {
                tx.commit().await.expect("commit");
                Ok(created.request_id)
            }
            Err(e) => {
                tx.rollback().await.expect("rollback");
                Err(e)
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_past_its_window_does_not_block_a_new_one() {
    let w = world().await;
    w.create("cust-1", "web-1", "tok-1", 0)
        .await
        .expect("first");

    // inside the window: still the open request (B4)
    match w.create("cust-1", "web-2", "tok-2", 59 * MINUTE).await {
        Err(AppError::Conflict(msg)) => assert_eq!(msg, "alreadyQueued:web-1"),
        other => panic!("inside the window: expected alreadyQueued:web-1, got {other:?}"),
    }

    // past the window: the old one no longer counts, the new one is made
    let id = w
        .create("cust-1", "web-3", "tok-3", PAST_WINDOW)
        .await
        .unwrap_or_else(|e| panic!("past the window the member can ask again: {e:?}"));
    assert_eq!(id, "web-3");
    let main = branch(MAIN);
    assert_eq!(
        w.kiosk
            .find_by_token(&main, "tok-3", at(PAST_WINDOW + MINUTE))
            .await
            .expect("new token"),
        "web-3"
    );
    assert!(
        matches!(
            w.kiosk
                .find_by_token(&main, "tok-1", at(PAST_WINDOW + MINUTE))
                .await,
            Err(AppError::NotFound { .. })
        ),
        "the expired token stays dead"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_past_its_window_gets_no_fresh_qr() {
    let w = world().await;
    let main = branch(MAIN);
    w.create("cust-1", "web-1", "tok-old", 0)
        .await
        .expect("created");

    let mut tx = w.db.begin().await;
    let r = w
        .web
        .reissue_qr_in(
            &mut tx,
            &main,
            "cust-1",
            "web-1",
            "tok-new",
            at(PAST_WINDOW),
        )
        .await;
    tx.rollback().await.expect("rollback");
    assert!(
        matches!(r, Err(AppError::NotFound { .. })),
        "an expired request answers tokenNotFound, never an expiresAt in the past: {r:?}"
    );
    assert!(
        matches!(
            w.kiosk
                .find_by_token(&main, "tok-new", at(PAST_WINDOW))
                .await,
            Err(AppError::NotFound { .. })
        ),
        "no live token was made"
    );
}
