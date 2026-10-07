//! `DeviceService::queue` where the branch has said it has no drawer
//! (`hasDrawer` = `"0"`, DRV-4 #287): a request id that was already queued is
//! answered as the existing idempotency answers it, whatever the switch says
//! now — the job as stored, or `REQUEST_CHANGED` for other content. Only a new
//! request id is refused with `สาขานี้ไม่มีลิ้นชัก` (lane controller, #287 Q4).
//!
//! Written before the refactor that folds `queue`'s two copies of the replay
//! rule into one. These tests are green on `main`: they hold kept behaviour,
//! so the refactor cannot change it. Do not edit, delete or weaken.
//!
//! Needs `TEST_DATABASE_URL`, as the other device tests; none skips.

mod common;
mod devices_common;
mod routes_common;

use devices_common::*;
use xc_auth::domain::policy::value_objects::actor::Actor;

/// The first model of the committed configuration (#274).
const NEOCAL: &str = "NEOCAL_NTP_8220BT";

/// `queue`'s answer to a new drawer request where the branch has no drawer.
const NO_DRAWER: &str = "สาขานี้ไม่มีลิ้นชัก";

fn drawer_request(n: u32, reason: &str) -> QueueJob {
    QueueJob {
        kind: "DRAWER_REQUEST".to_owned(),
        reference: String::new(),
        request_id: request_id(n),
        reason: reason.to_owned(),
    }
}

/// The owner says MAIN has no drawer.
async fn no_drawer_at_main(db: &Devices, seconds: i64) {
    db.service
        .set_drawer(
            &branch(MAIN),
            NEOCAL,
            false,
            &Actor::owner("owner"),
            at(seconds),
        )
        .await
        .expect("the owner says MAIN has no drawer");
}

#[tokio::test]
async fn a_request_queued_before_the_switch_went_to_0_answers_its_job_again() {
    let db = Devices::start().await;
    db.ids.give_one(&job_id(1));
    let original = db
        .service
        .queue(&branch(MAIN), drawer_request(1, "change"), ACTOR, at(1))
        .await
        .expect("MAIN has said nothing yet: the drawer request is queued");
    no_drawer_at_main(&db, 2).await;
    let before = db.stored().await;

    let again = db
        .service
        .queue(&branch(MAIN), drawer_request(1, "change"), ACTOR, at(3))
        .await
        .expect("the same request again is answered, not refused");

    assert_eq!(again, original, "the same request is the same job");
    assert!(
        db.stored().await.same_as(&before),
        "a replay stores no job and no line"
    );

    // a request id never seen is still refused
    db.ids.give_one(&job_id(2));
    let new = db
        .service
        .queue(&branch(MAIN), drawer_request(2, "change"), ACTOR, at(4))
        .await;
    assert_eq!(refusal(new), NO_DRAWER);

    db.finish().await;
}

#[tokio::test]
async fn a_replay_answers_the_job_as_it_is_now_after_the_switch_went_to_0() {
    let db = Devices::start().await;
    db.ids.give_one(&job_id(1));
    db.service
        .queue(&branch(MAIN), drawer_request(1, "change"), ACTOR, at(1))
        .await
        .expect("MAIN has said nothing yet: the drawer request is queued");
    let carried_out = db
        .service
        .process(&branch(MAIN), &job_id(1), ACTOR, at(2))
        .await
        .expect("the simulated driver carries the job out");
    assert_eq!(carried_out.status, JobStatus::CommandSimulated);
    no_drawer_at_main(&db, 3).await;

    let again = db
        .service
        .queue(&branch(MAIN), drawer_request(1, "change"), ACTOR, at(4))
        .await
        .expect("the same request again is answered, not refused");

    assert_eq!(
        again, carried_out,
        "the stored job, with the status it has now"
    );

    db.finish().await;
}

#[tokio::test]
async fn a_changed_request_after_the_switch_went_to_0_is_refused_as_changed() {
    let db = Devices::start().await;
    db.ids.give_one(&job_id(1));
    db.service
        .queue(&branch(MAIN), drawer_request(1, "change"), ACTOR, at(1))
        .await
        .expect("MAIN has said nothing yet: the drawer request is queued");
    no_drawer_at_main(&db, 2).await;
    let before = db.stored().await;

    let changed = db
        .service
        .queue(
            &branch(MAIN),
            drawer_request(1, "another reason"),
            ACTOR,
            at(3),
        )
        .await;

    assert_eq!(refusal(changed), REQUEST_CHANGED);
    assert!(
        db.stored().await.same_as(&before),
        "a refusal stores no job and no line"
    );

    db.finish().await;
}
