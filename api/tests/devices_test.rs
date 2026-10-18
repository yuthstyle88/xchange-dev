//! Specification tests for T13: `DeviceService` — print and drawer jobs, the
//! printer switch, what an employee saw of the drawer, and the lines written
//! about all of it.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Three kinds of test:
//! - the nine scenarios recorded from v2's own device code
//!   (`fixtures/devices/v2_device_cases.json`) are replayed call by call, and
//!   after every call the three tables must hold exactly what v2 held;
//! - what v3 adds on purpose: time cut to milliseconds, request ids kept per
//!   branch, everything written inside the branch transaction;
//! - the tables themselves.
//!
//! They need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`, a role that may create databases). Without it they FAIL;
//! none skips.

mod common;
mod devices_common;
mod routes_common;

use std::str::FromStr;

use devices_common::*;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::{Value, json};

// ───────────────────────────── the recording, replayed ──────────────────────

fn text(args: &Value, key: &str) -> String {
    args[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} in {args}"))
        .to_owned()
}

fn optional(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::to_owned)
}

fn decimal(args: &Value, key: &str) -> Decimal {
    Decimal::from_str(&text(args, key)).expect("a decimal")
}

/// What one recorded call answered, in the recording's words.
enum Answer {
    Nothing,
    Job(Value),
    Overview(Value),
}

/// The `transactions` row the save writes before its device step (amended for #759, RE-4e:
/// rates.md §4.9 R9 and the ticket author's ruling, the device service reads the stored row's
/// `unit_size` by document number, and a missing row is an error). The rate is per 1, as the
/// saved exchange carries it, so `unit_size` is 1; the principal is the baht HALF_UP to 2
/// places. A customer and an open shift of the branch come with it. A row already there (the
/// same document saved again) is kept.
async fn the_saved_row(
    pool: &sqlx::PgPool,
    branch_id: &str,
    document_ref: &str,
    submission_id: &str,
    currency: &str,
    amount: &str,
    rate: &str,
) {
    sqlx::query(
        // CI-4 (#651): a customer belongs to the organisation
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'dev-customer-' || $1, organization_id, 'dev-customer-' || $1, 'Somchai', 'LIVE', now() \
         FROM branches WHERE id = $1 ON CONFLICT DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's customer");
    // The shift's teller (`shifts.teller_id` is NOT NULL since #753, one OPEN shift per teller since M3).
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, 'dev-teller-' || $1, 'PENDING', 'test', 'dev-teller-' || $1 FROM organizations \
         WHERE code = 'XC' ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's teller");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ('dev-shift-' || $1, $1, 'OPEN', now(), 'owner', 'dev-open-' || $1, \
           (SELECT id FROM users WHERE legacy_employee_id = 'dev-teller-' || $1)) \
         ON CONFLICT DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's shift");
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
            customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
            recorded_by, completed_at) \
         VALUES ($1, $2, 'LIVE', 'dev-shift-' || $2, $3, 'hash', 'dev-customer-' || $2, 'Somchai', 'THA', \
            'owner', 'WALK-IN', '', '2026-10-06', now(), now(), 'owner', now()) \
         ON CONFLICT (organization_id, document_ref) DO NOTHING",
    )
    .bind(document_ref)
    .bind(branch_id)
    .bind(submission_id)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("the saved row {document_ref}: {e}"));
    sqlx::query(
        "INSERT INTO transaction_lines ( \
            id, organization_id, transaction_id, shop_side, currency, \
            quantity, unit_size, rate_snapshot, thb_amount, fee_thb, \
            settle_thb, line_no, created_at, updated_at \
         ) \
         SELECT \
            gen_random_uuid(), t.organization_id, t.id, 'BUY', $3, \
            $4::numeric, 1, $5::numeric, round($4::numeric * $5::numeric, 2), 0, \
            round($4::numeric * $5::numeric, 2), 1, now(), now() \
         FROM transactions t \
         WHERE t.document_ref = $1 AND t.branch_id = $2 \
         ON CONFLICT (transaction_id, line_no) DO NOTHING",
    )
    .bind(document_ref)
    .bind(branch_id)
    .bind(currency)
    .bind(amount)
    .bind(rate)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("the saved row line {document_ref}: {e}"));
}

async fn db_has_branch(db: &Devices, branch_id: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM branches WHERE id = $1)")
        .bind(branch_id)
        .fetch_one(&db.pool)
        .await
        .expect("look the branch up")
}

async fn call(db: &Devices, step: &Step) -> AppResult<Answer> {
    let b = branch(&step.branch);
    let now = instant(&step.now);
    let a = &step.args;
    let who = step.actor.as_str();
    match step.call.as_str() {
        "queue" => {
            let cmd = QueueJob {
                kind: text(a, "kind"),
                reference: text(a, "ref"),
                request_id: text(a, "requestId"),
                reason: text(a, "reason"),
            };
            let job = db.service.queue(&b, cmd, who, now).await?;
            Ok(Answer::Job(job_json(&job)))
        }
        "process" => {
            let job = db.service.process(&b, &text(a, "id"), who, now).await?;
            Ok(Answer::Job(job_json(&job)))
        }
        "retry" => {
            db.service.retry(&b, &text(a, "id"), who, now).await?;
            Ok(Answer::Nothing)
        }
        "printerOnline" => {
            let online = a["online"].as_bool().expect("online");
            db.service.set_printer_online(&b, online, who, now).await?;
            Ok(Answer::Nothing)
        }
        "drawerObservation" => {
            let reference = optional(a, "ref").unwrap_or_default();
            db.service
                .drawer_observation(&b, &reference, &text(a, "state"), who, now)
                .await?;
            Ok(Answer::Nothing)
        }
        "overview" => {
            let o = db.service.overview(&b).await?;
            Ok(Answer::Overview(json!({
                "jobs": o.jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>(),
                "events": o.events.iter().map(|e| json!({
                    "kind": e.kind, "ref": e.reference, "at": micros(e.at),
                })).collect::<Vec<_>>(),
                "printerOnline": o.printer_online,
            })))
        }
        "scanImported" => {
            db.service
                .scan_imported(&b, &text(a, "uploadKey"), &text(a, "mime"), who, now)
                .await?;
            Ok(Answer::Nothing)
        }
        "afterSave" => {
            // #759 (RE-4e, R9): the save wrote its row before this step.
            if db_has_branch(db, &step.branch).await {
                the_saved_row(
                    &db.pool,
                    &step.branch,
                    &text(a, "documentRef"),
                    &text(a, "submissionId"),
                    &text(a, "currency"),
                    &text(a, "amount"),
                    &text(a, "rate"),
                )
                .await;
            }
            let saved = SavedExchange {
                cash: a["cash"].as_bool().expect("cash"),
                submission_id: text(a, "submissionId"),
                document_ref: text(a, "documentRef"),
                currency: text(a, "currency"),
                amount: decimal(a, "amount"),
                rate: decimal(a, "rate"),
                passport_key: optional(a, "passport"),
                slip_key: optional(a, "slip"),
            };
            let service = db.service.clone();
            let who = who.to_owned();
            let inner = b.clone();
            db.branch_tx()
                .run(&b, move |tx| {
                    Box::pin(
                        async move { service.after_save_in(tx, &inner, &saved, &who, now).await },
                    )
                })
                .await?;
            Ok(Answer::Nothing)
        }
        other => panic!("the recording has a call the test does not know: {other}"),
    }
}

/// Replays one recorded scenario and compares, after every call, the answer and
/// the whole content of the three tables with what v2 answered and held.
async fn replay(name: &str) {
    let recording = recording();
    let scenario = recording
        .scenarios
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no scenario {name} in the recording"));
    assert!(!scenario.steps.is_empty(), "{name} has no steps");

    let db = Devices::start().await;
    db.exchanges.clear();
    for (branch_id, exchanges) in &scenario.exchanges {
        for e in exchanges {
            let state = if e.status == "COMPLETED" {
                ExchangeState::Completed
            } else {
                ExchangeState::NotCompleted
            };
            db.exchanges.set(branch_id, &e.id, state);
        }
    }

    let mut expected = Stored::default();
    let mut replayed = 0;
    for (i, step) in scenario.steps.iter().enumerate() {
        let label = format!(
            "{name} step {i} ({} by {} in {}{})",
            step.call,
            step.actor,
            step.branch,
            step.note
                .as_deref()
                .map(|n| format!(": {n}"))
                .unwrap_or_default()
        );
        if step.v3.is_some() {
            // the one call v3 answers differently on purpose; it has a test of its own
            assert!(!step.changed, "{label}: a skipped step must store nothing");
            continue;
        }
        db.ids.give(&step.ids);
        let answer = call(&db, step).await;

        match (&step.error, answer) {
            (Some(sentence), answer) => {
                let said = refusal(answer.map(|_| ()));
                assert_eq!(&said, sentence, "{label}");
            }
            (None, Err(e)) => panic!("{label}: v2 accepted this, got {e:?}"),
            (None, Ok(Answer::Nothing)) => {
                assert!(step.result.is_none(), "{label}: v2 answered with a value");
            }
            (None, Ok(Answer::Job(job))) => {
                let want = recorded_job(step.result.as_ref().expect("a recorded job"));
                assert!(same_json(&job, &want), "{label}\n got  {job}\n want {want}");
            }
            (None, Ok(Answer::Overview(mut got))) => {
                let mut want = step.result.clone().expect("a recorded overview");
                for e in want["events"].as_array_mut().expect("events") {
                    let at = e["at"].as_str().expect("at").to_owned();
                    e["at"] = Value::String(six_decimals(&at));
                }
                // v2 orders jobs by the instant they were made; none of the
                // recorded overviews has two jobs of one instant
                assert!(
                    same_json(&got, &want),
                    "{label}\n got  {}\n want {}",
                    got.take(),
                    want.take()
                );
            }
        }

        for job in &step.stored.jobs {
            let job = recorded_job(job);
            expected
                .jobs
                .insert(job["id"].as_str().expect("id").to_owned(), job);
        }
        for event in &step.stored.events {
            expected.events.push(recorded_event(event));
        }
        for s in &step.stored.settings {
            expected.settings.insert(
                (
                    s["branch_id"].as_str().expect("branch_id").to_owned(),
                    s["key"].as_str().expect("key").to_owned(),
                ),
                s["value"].as_str().expect("value").to_owned(),
            );
        }
        assert_eq!(
            db.ids.asked_for_too_many(),
            0,
            "{label}: an id was drawn where v2 drew none"
        );
        let stored = db.stored().await;
        assert!(
            stored.same_as(&expected),
            "{label}: what is stored differs from v2\n got  {stored:?}\n want {expected:?}"
        );
        replayed += 1;
    }
    assert!(replayed > 0, "{name}: nothing was replayed");
    db.finish().await;
}

/// The recording is the one that was checked in: nine scenarios, none empty.
/// A replay that silently reads less would pass for the wrong reason.
#[test]
fn the_recording_is_whole() {
    let r = recording();
    assert_eq!(r.counts.scenarios, 9);
    assert_eq!(r.scenarios.len(), 9);
    let steps: usize = r.scenarios.iter().map(|s| s.steps.len()).sum();
    assert_eq!(steps, 329);
    assert_eq!(r.counts.steps, 329);
    let refused = r
        .scenarios
        .iter()
        .flat_map(|s| &s.steps)
        .filter(|s| s.error.is_some())
        .count();
    assert_eq!(refused, 66);
    assert_eq!(r.counts.refused, 66);
    let stored = r
        .scenarios
        .iter()
        .flat_map(|s| &s.steps)
        .filter(|s| s.changed)
        .count();
    assert_eq!(stored, 250);
    assert_eq!(r.counts.stored, 250);
    assert_eq!(r.counts.v3_differs, 1);
    // every sentence the tests name is the one v2 said
    let said = |key: &str| r.sentences[key].as_str();
    assert_eq!(said("badBranch"), BAD_BRANCH);
    assert_eq!(said("noSuchBranch"), NO_SUCH_BRANCH);
    assert_eq!(said("badKind"), BAD_KIND);
    assert_eq!(said("badRequestId"), BAD_REQUEST_ID);
    assert_eq!(said("printNeedsRef"), PRINT_NEEDS_REF);
    assert_eq!(said("notCompletedHere"), NOT_COMPLETED_HERE);
    assert_eq!(said("reasonTooLong"), REASON_TOO_LONG);
    assert_eq!(said("drawerNeedsReason"), DRAWER_NEEDS_REASON);
    assert_eq!(said("requestChanged"), REQUEST_CHANGED);
    assert_eq!(said("noSuchJob"), NO_SUCH_JOB);
    assert_eq!(said("retryOnlyFailed"), RETRY_ONLY_FAILED);
    assert_eq!(said("notHere"), NOT_HERE);
    assert_eq!(said("badState"), BAD_STATE);
    assert_eq!(r.sentences.len(), 13);
    assert_eq!(r.count_first, COUNT_FIRST);
}

#[tokio::test]
async fn queue_refuses_what_v2_refuses_in_v2s_order() {
    replay("queue_checks").await;
}

#[tokio::test]
async fn the_same_request_again_is_the_same_job_and_a_changed_one_is_refused() {
    replay("queue_again").await;
}

#[tokio::test]
async fn a_job_is_processed_once_and_only_a_failed_one_is_retried() {
    replay("process_and_retry").await;
}

#[tokio::test]
async fn every_branch_has_its_own_printer_switch() {
    replay("printer_switch").await;
}

#[tokio::test]
async fn a_drawer_observation_names_an_exchange_of_the_branch_or_none() {
    replay("drawer_observation").await;
}

#[tokio::test]
async fn a_saved_exchange_leaves_its_lines_and_a_print_job_once() {
    replay("after_save").await;
}

#[tokio::test]
async fn an_uploaded_scan_leaves_a_line() {
    replay("scan_imported").await;
}

#[tokio::test]
async fn the_branch_is_judged_before_the_request() {
    replay("branches").await;
}

#[tokio::test]
async fn the_overview_is_the_newest_hundred_jobs_and_two_hundred_lines() {
    replay("overview").await;
}

// ───────────────────────────── pure rules ───────────────────────────────────

/// v2 refuses a cash exchange that was not counted, before it is saved.
#[test]
fn cash_is_counted_before_it_is_saved() {
    let r = recording();
    assert_eq!(r.count_confirmed.len(), 20);
    assert_eq!(r.count_confirmed.iter().filter(|c| c.refused).count(), 4);
    for case in &r.count_confirmed {
        let got = DeviceService::require_count_confirmed(case.cash, case.confirmed);
        if case.refused {
            assert_eq!(refusal(got), COUNT_FIRST, "{case:?}");
        } else {
            assert!(got.is_ok(), "{case:?}: {got:?}");
        }
    }
}

/// The id source the server runs with: v2 draws a random UUID per job.
#[test]
fn the_real_id_source_hands_out_random_uuids() {
    let source = UuidJobIds;
    let ids: Vec<String> = (0..64).map(|_| source.next()).collect();
    for id in &ids {
        let b = id.as_bytes();
        assert_eq!(b.len(), 36, "{id}");
        for (i, c) in b.iter().enumerate() {
            match i {
                8 | 13 | 18 | 23 => assert_eq!(*c, b'-', "{id}"),
                14 => assert_eq!(*c, b'4', "{id}: version 4"),
                _ => assert!(
                    c.is_ascii_digit() || (b'a'..=b'f').contains(c),
                    "{id}: lower-case hex"
                ),
            }
        }
    }
    let mut distinct = ids.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 64);
}

// ───────────────────────────── what v3 adds on purpose ──────────────────────

/// Time comes in as an argument and is cut to whole milliseconds — in the
/// answer and in every column.
#[tokio::test]
async fn time_is_cut_to_milliseconds_everywhere() {
    let db = Devices::start().await;
    let main = branch(MAIN);
    db.printer(MAIN, false).await;
    db.ids.give_one(&job_id(1));
    let cmd = QueueJob {
        kind: "PRINT_RECEIPT".to_owned(),
        reference: REF1.to_owned(),
        request_id: request_id(1),
        reason: String::new(),
    };
    assert_ne!(
        micros(at(0)),
        cut(at(0)),
        "the test instant has microseconds"
    );

    let job = db.service.queue(&main, cmd, ACTOR, at(0)).await.unwrap();
    assert_eq!(micros(job.created_at), cut(at(0)));
    assert_eq!(micros(job.updated_at), cut(at(0)));

    let job = db
        .service
        .process(&main, &job.id, ACTOR, at(5))
        .await
        .unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(micros(job.created_at), cut(at(0)));
    assert_eq!(micros(job.updated_at), cut(at(5)));

    db.service
        .retry(&main, &job.id, ACTOR, at(7))
        .await
        .unwrap();
    db.service
        .set_printer_online(&main, true, ACTOR, at(8))
        .await
        .unwrap();
    db.service
        .drawer_observation(&main, "", "OPEN", ACTOR, at(9))
        .await
        .unwrap();
    db.service
        .scan_imported(&main, "k", "image/png", ACTOR, at(10))
        .await
        .unwrap();

    let stored = db.jobs().await;
    assert_eq!(stored[&job.id]["created"], cut(at(0)));
    assert_eq!(stored[&job.id]["updated"], cut(at(7)));
    let at_of: Vec<String> = db
        .events()
        .await
        .iter()
        .map(|e| e["at"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        at_of,
        [
            cut(at(0)),
            cut(at(5)),
            cut(at(7)),
            cut(at(8)),
            cut(at(9)),
            cut(at(10))
        ]
    );

    let overview = db.service.overview(&main).await.unwrap();
    assert_eq!(micros(overview.jobs[0].updated_at), cut(at(7)));
    assert_eq!(micros(overview.events[0].at), cut(at(10)));
    db.finish().await;
}

/// v2 keeps request ids for the whole system: the same id in a second branch
/// is "the earlier request, changed". v3 keeps them per branch, like every
/// other key (the one recorded step marked `v3: differs`).
#[tokio::test]
async fn a_request_id_belongs_to_one_branch() {
    let db = Devices::start().await;
    let cmd = |reference: &str| QueueJob {
        kind: "PRINT_RECEIPT".to_owned(),
        reference: reference.to_owned(),
        request_id: request_id(7),
        reason: "reprint".to_owned(),
    };
    db.ids.give_one(&job_id(1));
    let first = db
        .service
        .queue(&branch(MAIN), cmd(REF1), "owner", at(0))
        .await
        .unwrap();
    db.ids.give_one(&job_id(2));
    let second = db
        .service
        .queue(&branch(B9), cmd(REF9), "owner", at(1))
        .await
        .expect("the same request id in another branch is a new job");
    assert_eq!(first.id, job_id(1));
    assert_eq!(second.id, job_id(2));
    assert_eq!(second.branch_id, B9);

    // each branch answers its own repeat with its own job
    db.ids.give_one(&job_id(3));
    let again = db
        .service
        .queue(&branch(B9), cmd(REF9), "owner", at(2))
        .await
        .unwrap();
    assert_eq!(again.id, job_id(2));
    db.ids.give_one(&job_id(4));
    let again = db
        .service
        .queue(&branch(MAIN), cmd(REF1), "owner", at(3))
        .await
        .unwrap();
    assert_eq!(again.id, job_id(1));
    assert_eq!(db.jobs().await.len(), 2);
    assert_eq!(db.events().await.len(), 2);
    db.finish().await;
}

/// Two jobs made at the same instant: the overview lists the greater id first,
/// so the order is the same on every call (v2 leaves it to the database).
#[tokio::test]
async fn jobs_of_one_instant_are_listed_by_id() {
    let db = Devices::start().await;
    for n in [2, 3, 1] {
        db.queued(MAIN, "DRAWER_REQUEST", "", "change", n).await;
    }
    let overview = db.service.overview(&branch(MAIN)).await.unwrap();
    let ids: Vec<&str> = overview.jobs.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, [job_id(3), job_id(2), job_id(1)]);
    // the lines are newest first by the order they were written
    let reasons: Vec<&str> = overview
        .events
        .iter()
        .map(|e| e.detail["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["change", "change", "change"]);
    let jobs: Vec<&str> = overview
        .events
        .iter()
        .map(|e| e.detail["job"].as_str().unwrap())
        .collect();
    assert_eq!(jobs, [job_id(1), job_id(3), job_id(2)]);
    assert!(overview.events[0].seq > overview.events[1].seq);
    db.finish().await;
}

/// The line about an uploaded scan is written for a branch that exists — the
/// upload route has checked it, the service checks again.
#[tokio::test]
async fn a_scan_line_needs_a_branch_that_exists() {
    let db = Devices::start().await;
    let said = db
        .service
        .scan_imported(&branch("no such"), "k", "image/png", ACTOR, at(0))
        .await;
    assert_eq!(refusal(said), BAD_BRANCH);
    let said = db
        .service
        .scan_imported(&branch("B404"), "k", "image/png", ACTOR, at(0))
        .await;
    assert_eq!(refusal(said), NO_SUCH_BRANCH);
    assert!(db.events().await.is_empty());
    db.finish().await;
}

// ───────────────────────────── inside the branch transaction ────────────────

/// One of the six commands that write, run against MAIN.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Write {
    Queue,
    Process,
    Retry,
    Printer,
    Drawer,
    Scan,
}

const WRITES: [Write; 6] = [
    Write::Queue,
    Write::Process,
    Write::Retry,
    Write::Printer,
    Write::Drawer,
    Write::Scan,
];

/// Job 1 is queued (for `Process`), job 2 has failed (for `Retry`).
async fn two_jobs(db: &Devices) {
    db.queued_print(1).await;
    db.queued_print(2).await;
    db.force_status(&job_id(2), "FAILED").await;
}

async fn write(service: DeviceService, ids: std::sync::Arc<FakeIds>, w: Write) -> AppResult<()> {
    let main = branch(MAIN);
    match w {
        Write::Queue => {
            ids.give_one(&job_id(50));
            let cmd = QueueJob {
                kind: "DRAWER_REQUEST".to_owned(),
                reference: String::new(),
                request_id: request_id(50),
                reason: "change".to_owned(),
            };
            service.queue(&main, cmd, ACTOR, at(20)).await.map(|_| ())
        }
        Write::Process => service
            .process(&main, &job_id(1), ACTOR, at(20))
            .await
            .map(|_| ()),
        Write::Retry => service.retry(&main, &job_id(2), ACTOR, at(20)).await,
        Write::Printer => {
            service
                .set_printer_online(&main, false, ACTOR, at(20))
                .await
        }
        Write::Drawer => {
            service
                .drawer_observation(&main, REF1, "OPEN", ACTOR, at(20))
                .await
        }
        Write::Scan => {
            service
                .scan_imported(&main, "k", "image/png", ACTOR, at(20))
                .await
        }
    }
}

/// A command whose transaction cannot commit leaves nothing: every statement
/// of it ran on the branch transaction, none on the pool.
#[tokio::test]
async fn a_command_that_cannot_commit_stores_nothing() {
    let db = Devices::start().await;
    two_jobs(&db).await;
    let before = db.stored().await;
    db.break_the_commit(MAIN).await;
    for w in WRITES {
        let result = write(db.service.clone(), db.ids.clone(), w).await;
        match result {
            Err(AppError::Domain(sentence)) => {
                panic!("{w:?}: a refusal, not a failure: {sentence}")
            }
            Err(_) => {}
            Ok(()) => panic!("{w:?}: succeeded although the transaction cannot commit"),
        }
        let after = db.stored().await;
        assert!(
            after.same_as(&before),
            "{w:?}: something was stored outside the transaction\n got  {after:?}\n want {before:?}"
        );
    }
    db.finish().await;
}

/// Inside its transaction a command uses that transaction's connection and no
/// other: on a pool of one connection every command still completes. A read
/// taken from the pool in the middle of the transaction would find none.
#[tokio::test]
async fn a_command_needs_no_connection_beside_its_transaction() {
    let db = Devices::start().await;
    two_jobs(&db).await;
    let (service, pool) = db.on_one_connection().await;
    for w in WRITES {
        within(
            MUST_FINISH,
            "a command on a pool of one connection",
            write(service.clone(), db.ids.clone(), w),
        )
        .await
        .unwrap_or_else(|e| panic!("{w:?}: {e:?}"));
    }
    pool.close().await;
    db.finish().await;
}

/// While another transaction holds the branch, each command waits and nothing
/// of it is visible; when the branch is free it completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_command_that_writes_waits_for_the_branch() {
    let db = Devices::start().await;
    two_jobs(&db).await;
    for w in WRITES {
        let before = db.stored().await;
        let holder = lock_branch(&db.pool, MAIN).await;
        let task = tokio::spawn(write(db.service.clone(), db.ids.clone(), w));
        until_somebody_waits(&db.pool).await;
        assert!(!task.is_finished(), "{w:?} must wait for the branch");
        let during = db.stored().await;
        assert!(
            during.same_as(&before),
            "{w:?}: wrote before it had the branch\n got  {during:?}\n want {before:?}"
        );
        holder.rollback().await.expect("release the branch");
        within(MUST_FINISH, "the command completes", task)
            .await
            .expect("the task did not panic")
            .unwrap_or_else(|e| panic!("{w:?}: {e:?}"));
        let after = db.stored().await;
        assert!(!after.same_as(&before), "{w:?} stored nothing");
    }
    db.finish().await;
}

/// The lock is the branch's: a held MAIN does not stop B9.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_branch_does_not_stop_another_branch() {
    let db = Devices::start().await;
    let holder = lock_branch(&db.pool, MAIN).await;
    within(MUST_FINISH, "a job in B9 while MAIN is held", async {
        db.queued(B9, "PRINT_RECEIPT", REF9, "", 1).await;
        db.service
            .process(&branch(B9), &job_id(1), ACTOR, at(1))
            .await
            .unwrap();
        db.service
            .set_printer_online(&branch(B9), false, ACTOR, at(2))
            .await
            .unwrap();
    })
    .await;
    holder.rollback().await.unwrap();
    db.finish().await;
}

/// Reading never takes the branch: the overview answers while it is held.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_overview_does_not_wait_for_the_branch() {
    let db = Devices::start().await;
    db.queued_print(1).await;
    let version = db.version(MAIN).await;
    let holder = lock_branch(&db.pool, MAIN).await;
    let overview = within(
        MUST_FINISH,
        "the overview while MAIN is held",
        db.service.overview(&branch(MAIN)),
    )
    .await
    .unwrap();
    assert_eq!(overview.jobs.len(), 1);
    assert!(overview.printer_online);
    holder.rollback().await.unwrap();
    assert_eq!(db.version(MAIN).await, version, "reading writes nothing");
    db.finish().await;
}

/// `process` decides with what it reads after it has the branch: a printer
/// switched off by the transaction before it fails the job.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn process_reads_the_printer_switch_after_it_has_the_branch() {
    let db = Devices::start().await;
    db.queued_print(1).await;
    let mut holder = lock_branch(&db.pool, MAIN).await;
    let service = db.service.clone();
    let task = tokio::spawn(async move {
        service
            .process(&branch(MAIN), &job_id(1), ACTOR, at(5))
            .await
    });
    until_somebody_waits(&db.pool).await;
    sqlx::query(
        "INSERT INTO device_settings (branch_id, key, value) VALUES ('MAIN', 'printerOnline', '0')",
    )
    .execute(&mut *holder)
    .await
    .unwrap();
    holder.commit().await.unwrap();
    let job = within(MUST_FINISH, "process", task).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    db.finish().await;
}

/// … and a job that the transaction before already processed is returned as it
/// is, with no second line.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn process_reads_the_job_after_it_has_the_branch() {
    let db = Devices::start().await;
    db.queued_print(1).await;
    let lines = db.events().await.len();
    let mut holder = lock_branch(&db.pool, MAIN).await;
    let service = db.service.clone();
    let task = tokio::spawn(async move {
        service
            .process(&branch(MAIN), &job_id(1), ACTOR, at(5))
            .await
    });
    until_somebody_waits(&db.pool).await;
    sqlx::query("UPDATE device_jobs SET status = 'COMMAND_SIMULATED' WHERE id = $1")
        .bind(job_id(1))
        .execute(&mut *holder)
        .await
        .unwrap();
    holder.commit().await.unwrap();
    let job = within(MUST_FINISH, "process", task).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::CommandSimulated);
    assert_eq!(micros(job.updated_at), cut(at(0)), "not touched");
    assert_eq!(db.events().await.len(), lines, "no second line");
    db.finish().await;
}

/// `retry` too: a job the transaction before queued again is no longer failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retry_reads_the_job_after_it_has_the_branch() {
    let db = Devices::start().await;
    db.queued_print(1).await;
    db.force_status(&job_id(1), "FAILED").await;
    let before = db.events().await.len();
    let mut holder = lock_branch(&db.pool, MAIN).await;
    let service = db.service.clone();
    let task =
        tokio::spawn(async move { service.retry(&branch(MAIN), &job_id(1), ACTOR, at(5)).await });
    until_somebody_waits(&db.pool).await;
    sqlx::query("UPDATE device_jobs SET status = 'QUEUED' WHERE id = $1")
        .bind(job_id(1))
        .execute(&mut *holder)
        .await
        .unwrap();
    holder.commit().await.unwrap();
    let said = within(MUST_FINISH, "retry", task).await.unwrap();
    assert_eq!(refusal(said), RETRY_ONLY_FAILED);
    assert_eq!(db.events().await.len(), before);
    db.finish().await;
}

/// `queue` looks for the earlier request after it has the branch: a request
/// the transaction before stored (with other content) is "changed", not a
/// database error and not a second job.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_looks_for_the_earlier_request_after_it_has_the_branch() {
    let db = Devices::start().await;
    let mut holder = lock_branch(&db.pool, MAIN).await;
    let service = db.service.clone();
    db.ids.give_one(&job_id(2));
    let task = tokio::spawn(async move {
        let cmd = QueueJob {
            kind: "DRAWER_REQUEST".to_owned(),
            reference: String::new(),
            request_id: request_id(9),
            reason: "change".to_owned(),
        };
        service.queue(&branch(MAIN), cmd, ACTOR, at(5)).await
    });
    until_somebody_waits(&db.pool).await;
    sqlx::query(
        "INSERT INTO device_jobs (id, request_id, request_hash, branch_id, ref, kind, status, actor, created_at, updated_at) \
         VALUES ($1, $2, 'the hash of something else', 'MAIN', '', 'DRAWER_REQUEST', 'QUEUED', 'somebody', now(), now())",
    )
    .bind(job_id(1))
    .bind(request_id(9))
    .execute(&mut *holder)
    .await
    .unwrap();
    holder.commit().await.unwrap();
    let said = within(MUST_FINISH, "queue", task).await.unwrap();
    assert_eq!(refusal(said), REQUEST_CHANGED);
    assert_eq!(db.jobs().await.len(), 1);
    assert!(db.events().await.is_empty());
    db.finish().await;
}

/// Sixteen callers send the same request at once: one job, one line, sixteen
/// times the same answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_same_request_from_many_callers_at_once_is_one_job() {
    let db = Devices::start().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let service = db.service.clone();
        tasks.push(tokio::spawn(async move {
            let cmd = QueueJob {
                kind: "PRINT_RECEIPT".to_owned(),
                reference: REF1.to_owned(),
                request_id: request_id(1),
                reason: String::new(),
            };
            service.queue(&branch(MAIN), cmd, ACTOR, at(0)).await
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(
            within(MUST_FINISH, "queue", task)
                .await
                .unwrap()
                .expect("every caller gets the job")
                .id,
        );
    }
    ids.dedup();
    assert_eq!(ids.len(), 1, "one job for all: {ids:?}");
    assert_eq!(db.jobs().await.len(), 1);
    assert_eq!(db.events().await.len(), 1);
    db.finish().await;
}

/// A command that stores bumps the branch's version by one; a refusal — before
/// the transaction or inside it — bumps nothing.
#[tokio::test]
async fn a_stored_command_moves_the_branch_version_and_a_refusal_does_not() {
    let db = Devices::start().await;
    two_jobs(&db).await;
    let main = branch(MAIN);
    for w in WRITES {
        let before = db.version(MAIN).await;
        write(db.service.clone(), db.ids.clone(), w)
            .await
            .unwrap_or_else(|e| panic!("{w:?}: {e:?}"));
        assert_eq!(db.version(MAIN).await, before + 1, "{w:?}");
    }
    let before = db.version(MAIN).await;
    let other = db.version(B9).await;
    let said = db
        .service
        .process(&main, "no such job", ACTOR, at(30))
        .await;
    assert_eq!(refusal(said), NO_SUCH_JOB);
    let said = db.service.retry(&main, &job_id(1), ACTOR, at(30)).await;
    assert_eq!(refusal(said), RETRY_ONLY_FAILED);
    let changed = QueueJob {
        kind: "DRAWER_REQUEST".to_owned(),
        reference: String::new(),
        request_id: request_id(50),
        reason: "something else".to_owned(),
    };
    let said = db.service.queue(&main, changed, ACTOR, at(30)).await;
    assert_eq!(refusal(said), REQUEST_CHANGED);
    let said = db
        .service
        .drawer_observation(&main, "", "AJAR", ACTOR, at(30))
        .await;
    assert_eq!(refusal(said), BAD_STATE);
    assert_eq!(db.version(MAIN).await, before);
    assert_eq!(
        db.version(B9).await,
        other,
        "MAIN's commands never touch B9"
    );
    db.finish().await;
}

/// `after_save_in` writes on the transaction it is handed — the saving of the
/// exchange — and takes no transaction of its own: rolled back, nothing stays.
#[tokio::test]
async fn the_lines_of_a_save_live_and_die_with_the_callers_transaction() {
    let db = Devices::start().await;
    let saved = SavedExchange {
        cash: true,
        submission_id: "sub-1".to_owned(),
        document_ref: REF1.to_owned(),
        currency: "USD".to_owned(),
        amount: Decimal::from_str("100.50").unwrap(),
        rate: Decimal::from_str("32.1234").unwrap(),
        passport_key: None,
        slip_key: None,
    };
    let main = branch(MAIN);
    // #759 (RE-4e, R9): the save wrote its row before its device step.
    the_saved_row(&db.pool, MAIN, REF1, "sub-1", "USD", "100.50", "32.1234").await;

    // rolled back by the caller
    db.ids.give_one(&job_id(1));
    let mut tx = db.pool.begin().await.unwrap();
    db.service
        .after_save_in(&mut tx, &main, &saved, ACTOR, at(0))
        .await
        .unwrap();
    let inside: i64 = sqlx::query_scalar("SELECT count(*) FROM device_events")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        inside, 3,
        "the caller sees the lines on its own transaction"
    );
    assert!(db.events().await.is_empty(), "nobody else does yet");
    tx.rollback().await.unwrap();
    assert!(db.events().await.is_empty());
    assert!(db.jobs().await.is_empty());

    // a failure after it, inside BranchTx: the same
    db.ids.give_one(&job_id(2));
    let service = db.service.clone();
    let (b, s) = (main.clone(), saved.clone());
    let failed: AppResult<()> = db
        .branch_tx()
        .run(&main, move |tx| {
            Box::pin(async move {
                service.after_save_in(tx, &b, &s, ACTOR, at(1)).await?;
                Err(AppError::Domain("the save failed after all".to_owned()))
            })
        })
        .await;
    assert_eq!(refusal(failed), "the save failed after all");
    assert!(db.events().await.is_empty());
    assert!(db.jobs().await.is_empty());

    // committed by the caller
    db.ids.give_one(&job_id(3));
    let service = db.service.clone();
    let (b, s) = (main.clone(), saved.clone());
    db.branch_tx()
        .run(&main, move |tx| {
            Box::pin(async move { service.after_save_in(tx, &b, &s, ACTOR, at(2)).await })
        })
        .await
        .unwrap();
    let events = db.events().await;
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["kind"], "COUNT_CONFIRMED");
    // 100.50 × 32.1234 = 3228.4017, HALF_UP to 2 places: 3228.40 (amended for #759,
    // rates.md §4.9 R2: the count line's baht is ExchangeArithmetic::thb); v2's doubles are not asked
    assert!(same_json(
        &events[0]["detail"],
        &json!({"mode": "EMPLOYEE_CONFIRMATION", "currency": "USD", "foreignAmount": 100.5, "thb": 3228.40})
    ));
    assert_eq!(events[0]["at"], cut(at(2)));
    let jobs = db.jobs().await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[&job_id(3)]["request_id"], format!("auto:MAIN:{REF1}"));
    assert_eq!(jobs[&job_id(3)]["created"], cut(at(2)));
    db.finish().await;
}

// ───────────────────────────── the tables ───────────────────────────────────

async fn refused(pool: &sqlx::PgPool, sql: &str) -> bool {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .is_err()
}

/// What the service never writes, the tables refuse.
#[tokio::test]
async fn the_tables_refuse_what_the_service_never_writes() {
    let db = Devices::start().await;
    let job = |id: &str, request: &str, branch_id: &str, kind: &str, status: &str| {
        format!(
            "INSERT INTO device_jobs (id, request_id, request_hash, branch_id, ref, kind, status, actor, created_at, updated_at) \
             VALUES ('{id}', '{request}', 'h', '{branch_id}', '', '{kind}', '{status}', 'a', now(), now())"
        )
    };
    assert!(
        !refused(
            &db.pool,
            &job("j1", "r1", "MAIN", "PRINT_RECEIPT", "QUEUED")
        )
        .await
    );
    assert!(
        refused(
            &db.pool,
            &job("j1", "r2", "MAIN", "PRINT_RECEIPT", "QUEUED")
        )
        .await,
        "a second job with the same id"
    );
    assert!(
        refused(
            &db.pool,
            &job("j2", "r1", "MAIN", "PRINT_RECEIPT", "QUEUED")
        )
        .await,
        "the same request id twice in one branch"
    );
    assert!(
        !refused(&db.pool, &job("j2", "r1", "B9", "DRAWER_REQUEST", "FAILED")).await,
        "the same request id in another branch"
    );
    assert!(
        refused(
            &db.pool,
            &job("j3", "r3", "B404", "PRINT_RECEIPT", "QUEUED")
        )
        .await,
        "a branch that does not exist"
    );
    assert!(
        refused(&db.pool, &job("j3", "r3", "MAIN", "SCAN", "QUEUED")).await,
        "a kind that is not one"
    );
    assert!(
        refused(&db.pool, &job("j3", "r3", "MAIN", "PRINT_RECEIPT", "DONE")).await,
        "a status that is not one"
    );
    for status in ["SIMULATED_PRINTED", "COMMAND_SIMULATED"] {
        let id = format!("j-{status}");
        assert!(!refused(&db.pool, &job(&id, &id, "MAIN", "PRINT_RECEIPT", status)).await);
    }

    let event = |dedupe: &str, branch_id: &str, detail: &str| {
        format!(
            "INSERT INTO device_events (dedupe, branch_id, actor, kind, ref, detail, at) \
             VALUES ({dedupe}, '{branch_id}', 'a', 'K', '', {detail}, now())"
        )
    };
    assert!(!refused(&db.pool, &event("NULL", "MAIN", "'{}'")).await);
    assert!(
        !refused(&db.pool, &event("NULL", "MAIN", "'{}'")).await,
        "lines without a key never collide"
    );
    assert!(!refused(&db.pool, &event("'k'", "MAIN", "'{}'")).await);
    assert!(
        refused(&db.pool, &event("'k'", "MAIN", "'{}'")).await,
        "the same key twice in one branch"
    );
    assert!(
        !refused(&db.pool, &event("'k'", "B9", "'{}'")).await,
        "the same key in another branch"
    );
    assert!(
        refused(&db.pool, &event("NULL", "B404", "'{}'")).await,
        "a branch that does not exist"
    );
    assert!(
        refused(&db.pool, &event("NULL", "MAIN", "NULL")).await,
        "a line without detail"
    );
    assert!(
        refused(&db.pool, &event("NULL", "MAIN", "'not json'")).await,
        "detail that is not JSON"
    );

    let setting = |branch_id: &str| {
        format!(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ('{branch_id}', 'printerOnline', '1')"
        )
    };
    assert!(!refused(&db.pool, &setting("MAIN")).await);
    assert!(
        refused(&db.pool, &setting("MAIN")).await,
        "one switch per branch"
    );
    assert!(!refused(&db.pool, &setting("B9")).await);
    assert!(refused(&db.pool, &setting("B404")).await);
    db.finish().await;
}
