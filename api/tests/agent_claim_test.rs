//! Row AG-2 (`docs/design/xchange-v3.md` §5.4c *Claim → send → record*, *Who writes
//! `device_jobs.status`*, *Security*): claim and report through
//! `POST /api/device-agent/{claim,report}`. Written before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with the
//! test's clock (`RpcClock`, as `/api/devices` reads it). The agent is written straight into
//! `device_agents` (row AG-1's table) with the SHA-256 of a known token, so nothing here
//! depends on row AG-4; drawer jobs are queued by the owner through `POST /api/devices`.
//! This file compiles on `main` and fails there on results.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const CLAIM: &str = "/api/device-agent/claim";
const REPORT: &str = "/api/device-agent/report";
const MAIN: &str = "MAIN";
const B9: &str = "B9";

const AGENT: &str = "a0000000-0000-4000-8000-000000000001";
const AGENT_NAME: &str = "Mac เคาน์เตอร์ 1";
const TOKEN: &str = "xcda_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// `(last_seen_at, agent_version, agent_build, drawer_transport, drawer_target)`.
type Seen = (
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// `ESC @` + `ESC p 0 25 250` (NEOCAL, the first model of `devices.printerModels`), base64.
const NEOCAL_KICK: &str = "G0AbcAAZ+g==";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn t0() -> DateTime<Utc> {
    time("2026-10-08T03:00:00.000Z")
}

fn at(millis: i64) -> DateTime<Utc> {
    t0() + Duration::milliseconds(millis)
}

struct Shop {
    desk: Desk,
    app: App,
    owner: Cookies,
}

impl Shop {
    /// MAIN with a paired, active agent; B9 with no agent.
    async fn open() -> Shop {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, \
             paired_at, last_seen_at, agent_version, agent_build, agent_os, agent_host, \
             drawer_transport, drawer_target) VALUES ($1::uuid, $2, $3, $4, 'owner', $5, $4, $4, \
             '1.0.0', 'abc1234', 'macos-14.6-arm64', 'counter-1.local', 'CUPS', 'POS80_')",
        )
        .bind(AGENT)
        .bind(MAIN)
        .bind(AGENT_NAME)
        .bind(t0() - Duration::hours(1))
        .bind(sha256_hex(TOKEN))
        .execute(&desk.db.pool)
        .await
        .expect("an agent of MAIN (row AG-1's table)");
        let app = rpc_app!(desk);
        let people: Vec<_> = one_of_each_role().into_iter().take(1).collect();
        let owner = sign_in(&app, ORIGIN, &people).await;
        Shop { desk, app, owner }
    }

    fn clock(&self, t: DateTime<Utc>) {
        self.desk.clock.set(t);
    }

    /// The owner queues a drawer job of `branch` at `t`; its id.
    async fn queue_drawer(&self, branch: &str, n: u32, t: DateTime<Utc>) -> String {
        self.clock(t);
        let answer = self
            .app
            .send(
                Call::action(
                    ORIGIN,
                    "/api/devices",
                    "queue",
                    json!({ "branchId": branch, "kind": "DRAWER_REQUEST", "ref": "",
                            "requestId": format!("dddddddd-0000-4000-8000-{n:012x}"),
                            "reason": "ทอนเงินลูกค้า" }),
                )
                .cookie(&self.owner["OWNER"]),
            )
            .await;
        assert_eq!(answer.status, 200, "queue: {}", answer.text);
        answer.json()["id"]
            .as_str()
            .expect("the job's id")
            .to_owned()
    }

    async fn queue_receipt_job(&self, id: &str, t: DateTime<Utc>) {
        sqlx::query(
            "INSERT INTO device_jobs (id, request_id, request_hash, branch_id, ref, kind, status, \
             actor, created_at, updated_at) VALUES ($1, $1, 'h', 'MAIN', 'XC-1', 'PRINT_RECEIPT', \
             'QUEUED', 'owner', $2, $2)",
        )
        .bind(id)
        .bind(t)
        .execute(&self.desk.db.pool)
        .await
        .expect("a receipt job");
    }

    fn agent_call(path: &str, body: Value) -> Call {
        Call::post(ORIGIN, path, body.to_string())
            .without("origin")
            .without("x-xchange-request")
            .header("authorization", &format!("Bearer {TOKEN}"))
            .header("x-real-ip", "10.1.1.1")
    }

    async fn claim_at(&self, t: DateTime<Utc>, wait_ms: u64) -> Answer {
        self.clock(t);
        self.app
            .send(Shop::agent_call(CLAIM, claim_body(wait_ms)))
            .await
    }

    async fn report_at(&self, t: DateTime<Utc>, body: Value) -> Answer {
        self.clock(t);
        self.app.send(Shop::agent_call(REPORT, body)).await
    }

    /// `(status, lease_id, leased_by, lease_until, outcome_reason)` of a job.
    async fn job(
        &self,
        id: &str,
    ) -> (
        String,
        Option<String>,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<String>,
    ) {
        sqlx::query_as(
            "SELECT status, lease_id::text, leased_by::text, lease_until, outcome_reason \
             FROM device_jobs WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("read job {id}: {e}"))
    }

    /// `(kind, actor, ref, detail)` of every line of MAIN written by the agent, in order.
    async fn agent_lines(&self) -> Vec<(String, String, String, Value)> {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT kind, actor, ref, detail::text FROM device_events \
             WHERE actor = $1 ORDER BY seq",
        )
        .bind(format!("agent:{AGENT}"))
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read device_events");
        rows.into_iter()
            .map(|(k, a, r, d)| (k, a, r, serde_json::from_str(&d).expect("JSON detail")))
            .collect()
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn claim_body(wait_ms: u64) -> Value {
    json!({ "waitMs": wait_ms, "version": "1.0.1", "build": "def5678", "os": "macos-14.6-arm64",
            "host": "counter-1.local", "drawer": { "transport": "CUPS", "target": "POS80_" } })
}

fn report_body(job: &str, lease: &str, outcome: &str) -> Value {
    json!({ "jobId": job, "leaseId": lease, "outcome": outcome, "reason": "",
            "detail": { "cupsJob": "POS80_-12" } })
}

/// The lease of a 200 claim answer, after checking the answer's exact shape.
fn leased(answer: &Answer, job: &str) -> String {
    assert_eq!(answer.status, 200, "a job is handed out: {}", answer.text);
    let body = answer.json();
    let keys: Vec<&str> = body
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        ["bytes", "jobId", "leaseId", "sendWithinMs"],
        "four fields and nothing else: {body}"
    );
    assert_eq!(body["jobId"], job);
    body["leaseId"].as_str().expect("a lease id").to_owned()
}

// ── claim ────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn nothing_to_do_answers_204_at_once_when_wait_ms_is_0() {
    let shop = Shop::open().await;
    let started = Instant::now();
    let answer = shop.claim_at(at(0), 0).await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    assert_eq!(answer.text, "", "a 204 has no body");
    assert!(started.elapsed() < StdDuration::from_millis(1500));
    shop.finish().await;
}

/// The oldest drawer job is leased for 10 s with the branch's kick bytes, and the agent's row
/// takes what the claim says.
#[actix_web::test]
async fn a_drawer_job_is_leased_with_the_branch_s_kick() {
    let shop = Shop::open().await;
    let first = shop.queue_drawer(MAIN, 1, at(0)).await;
    let _second = shop.queue_drawer(MAIN, 2, at(500)).await;
    let answer = shop.claim_at(at(1_000), 0).await;
    let lease = leased(&answer, &first);
    let body = answer.json();
    assert_eq!(body["bytes"], NEOCAL_KICK, "the model's pulse, base64");
    assert_eq!(body["sendWithinMs"], 9_000, "created_at + 10 s − now");

    let (status, lease_id, by, until, _) = shop.job(&first).await;
    assert_eq!(
        (status.as_str(), lease_id.as_deref(), by.as_deref(), until),
        (
            "LEASED",
            Some(lease.as_str()),
            Some(AGENT),
            Some(at(11_000))
        )
    );
    let lines = shop.agent_lines().await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, "JOB_LEASED");
    assert_eq!(lines[0].3["job"], first);

    let seen: Seen = sqlx::query_as(
        "SELECT last_seen_at, agent_version, agent_build, drawer_transport, drawer_target \
             FROM device_agents WHERE id = $1::uuid",
    )
    .bind(AGENT)
    .fetch_one(&shop.desk.db.pool)
    .await
    .expect("read the agent");
    assert_eq!(
        seen,
        (
            Some(at(1_000)),
            Some("1.0.1".to_owned()),
            Some("def5678".to_owned()),
            Some("CUPS".to_owned()),
            Some("POS80_".to_owned())
        )
    );
    shop.finish().await;
}

/// A drawer job created more than 10 s before the claim is `EXPIRED` and never handed out.
#[actix_web::test]
async fn a_job_older_than_ten_seconds_expires_and_is_never_sent() {
    let shop = Shop::open().await;
    let old = shop.queue_drawer(MAIN, 1, at(0)).await;
    let answer = shop.claim_at(at(10_001), 0).await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    assert_eq!(shop.job(&old).await.0, "EXPIRED");
    let lines = shop.agent_lines().await;
    assert_eq!(
        lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["EXPIRED"]
    );
    shop.finish().await;
}

/// Receipts and another branch's drawer jobs are never handed to this agent.
#[actix_web::test]
async fn receipts_and_another_branch_s_jobs_are_never_claimed() {
    let shop = Shop::open().await;
    shop.queue_receipt_job("receipt-1", at(0)).await;
    let other = shop.queue_drawer(B9, 1, at(0)).await;
    let answer = shop.claim_at(at(100), 0).await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    assert_eq!(shop.job("receipt-1").await.0, "QUEUED");
    assert_eq!(shop.job(&other).await.0, "QUEUED");
    shop.finish().await;
}

/// A lease whose 10 s ran out becomes `FAILED` `LEASE_LAPSED` at the next claim, which then
/// hands out the next job.
#[actix_web::test]
async fn a_lapsed_lease_fails_and_the_next_job_is_leased() {
    let shop = Shop::open().await;
    let first = shop.queue_drawer(MAIN, 1, at(0)).await;
    leased(&shop.claim_at(at(1_000), 0).await, &first);
    let second = shop.queue_drawer(MAIN, 2, at(11_500)).await;
    let answer = shop.claim_at(at(12_000), 0).await;
    leased(&answer, &second);
    let (status, _, _, _, reason) = shop.job(&first).await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("FAILED", Some("LEASE_LAPSED"))
    );
    let kinds: Vec<String> = shop.agent_lines().await.into_iter().map(|l| l.0).collect();
    assert_eq!(kinds, ["JOB_LEASED", "FAILED", "JOB_LEASED"]);
    shop.finish().await;
}

/// Follow-up 1 of the review of PR 324 (§5.4c *The bytes*): a job that reaches the claim in
/// a branch whose model has no pulse is failed `NO_PULSE`; nothing is handed out.
#[actix_web::test]
async fn a_branch_without_a_pulse_fails_the_job_no_pulse() {
    let shop = Shop::open().await;
    sqlx::query(
        "INSERT INTO device_settings (branch_id, key, value) VALUES ('MAIN', 'printerModel', 'NO_SUCH_MODEL')",
    )
    .execute(&shop.desk.db.pool)
    .await
    .expect("a model the registry does not know");
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let answer = shop.claim_at(at(100), 0).await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    let (status, _, _, _, reason) = shop.job(&job).await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("FAILED", Some("NO_PULSE"))
    );
    assert_eq!(
        shop.agent_lines()
            .await
            .last()
            .map(|l| l.0.clone())
            .as_deref(),
        Some("FAILED")
    );
    shop.finish().await;
}

/// The long poll: with nothing to do the claim waits, probing, and answers as soon as a job
/// arrives; with nothing arriving it answers 204 after `waitMs`. While it waits it holds no
/// branch lock: the owner's queue goes through at once.
#[actix_web::test]
async fn the_long_poll_answers_when_a_job_arrives_and_holds_no_lock() {
    let shop = Shop::open().await;
    shop.clock(at(0));
    let started = Instant::now();
    let (claim, queued) = tokio::join!(
        shop.app.send(Shop::agent_call(CLAIM, claim_body(5_000))),
        async {
            tokio::time::sleep(StdDuration::from_millis(600)).await;
            let before = Instant::now();
            let id = shop.queue_drawer(MAIN, 1, at(0)).await;
            (id, before.elapsed())
        }
    );
    let (job, queue_took) = queued;
    assert!(
        queue_took < StdDuration::from_millis(1_000),
        "the queue did not wait for the claim: {queue_took:?}"
    );
    leased(&claim, &job);
    let took = started.elapsed();
    assert!(
        took >= StdDuration::from_millis(500) && took < StdDuration::from_millis(3_000),
        "answered after the job arrived, well before waitMs: {took:?}"
    );

    let started = Instant::now();
    let answer = shop.claim_at(at(1_000), 1_200).await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    let took = started.elapsed();
    assert!(
        took >= StdDuration::from_millis(1_000) && took < StdDuration::from_millis(3_000),
        "waited about waitMs: {took:?}"
    );
    shop.finish().await;
}

// ── report ───────────────────────────────────────────────────────────────────

/// A report moves the job from `LEASED` to its outcome and writes the audit line of §5.4c
/// *Security*.
#[actix_web::test]
async fn a_report_records_drawer_opened_with_the_audit_line() {
    let shop = Shop::open().await;
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let lease = leased(&shop.claim_at(at(1_000), 0).await, &job);
    let answer = shop
        .report_at(at(1_400), report_body(&job, &lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(shop.job(&job).await.0, "DRAWER_OPENED");

    let lines = shop.agent_lines().await;
    let opened: Vec<_> = lines.iter().filter(|l| l.0 == "DRAWER_OPENED").collect();
    assert_eq!(opened.len(), 1, "{lines:?}");
    let (_, actor, _, detail) = opened[0];
    assert_eq!(actor, &format!("agent:{AGENT}"));
    let owner_id: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_one(&shop.desk.db.pool)
    .await
    .expect("read owner users.id");
    for (key, expected) in [
        ("job", json!(job)),
        ("reason", json!("ทอนเงินลูกค้า")),
        // OR-7.m G2: rule 2 (#3700)
        ("queuedBy", json!(owner_id)),
        ("agent", json!(AGENT)),
        ("agentName", json!(AGENT_NAME)),
        ("agentVersion", json!("1.0.1")),
        ("drawerTarget", json!("POS80_")),
        ("leaseId", json!(lease)),
        ("cupsJob", json!("POS80_-12")),
        ("physicalAction", json!(true)),
        ("physicalOpenConfirmed", json!(false)),
    ] {
        assert_eq!(detail[key], expected, "detail.{key}: {detail}");
    }
    for key in ["queuedAt", "leasedAt", "reportedAt"] {
        assert!(detail[key].is_string(), "detail.{key} is a time: {detail}");
    }
    shop.finish().await;
}

#[actix_web::test]
async fn the_same_report_again_answers_200_and_writes_nothing() {
    let shop = Shop::open().await;
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let lease = leased(&shop.claim_at(at(1_000), 0).await, &job);
    let body = report_body(&job, &lease, "DRAWER_OPENED");
    assert_eq!(shop.report_at(at(1_400), body.clone()).await.status, 200);
    let lines = shop.agent_lines().await.len();
    let again = shop.report_at(at(1_900), body).await;
    assert_eq!(again.status, 200, "{}", again.text);
    assert_eq!(shop.agent_lines().await.len(), lines, "nothing written");
    shop.finish().await;
}

/// Another lease, another outcome after one was recorded, a job never leased, or a job of
/// another branch: refused; a report never puts a job back to `QUEUED`.
#[actix_web::test]
async fn a_report_that_does_not_match_its_lease_is_refused() {
    let shop = Shop::open().await;
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let waiting = shop.queue_drawer(MAIN, 2, at(500)).await;
    let other = shop.queue_drawer(B9, 3, at(0)).await;
    let lease = leased(&shop.claim_at(at(1_000), 0).await, &job);

    let wrong_lease = "1ea5e000-0000-4000-8000-0000000000ff";
    let answer = shop
        .report_at(at(1_200), report_body(&job, wrong_lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer.status, 409, "another lease: {}", answer.text);
    let answer = shop
        .report_at(at(1_200), report_body(&waiting, &lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer.status, 409, "a job not leased: {}", answer.text);
    let answer = shop
        .report_at(at(1_200), report_body(&other, &lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer.status, 404, "another branch's job: {}", answer.text);
    let answer = shop
        .report_at(at(1_200), report_body(&job, &lease, "QUEUED"))
        .await;
    assert_eq!(
        answer.status, 400,
        "QUEUED is not an outcome: {}",
        answer.text
    );

    assert_eq!(
        shop.report_at(at(1_300), report_body(&job, &lease, "DRAWER_OPENED"))
            .await
            .status,
        200
    );
    let answer = shop
        .report_at(at(1_400), report_body(&job, &lease, "FAILED"))
        .await;
    assert_eq!(answer.status, 409, "another outcome after one was recorded");
    assert_eq!(shop.job(&job).await.0, "DRAWER_OPENED");
    assert_eq!(shop.job(&waiting).await.0, "QUEUED");
    shop.finish().await;
}

/// A late report replaces the lapse: it tells what really happened at the drawer.
#[actix_web::test]
async fn a_late_report_replaces_the_lapse() {
    let shop = Shop::open().await;
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let lease = leased(&shop.claim_at(at(1_000), 0).await, &job);
    assert_eq!(shop.claim_at(at(12_000), 0).await.status, 204);
    assert_eq!(shop.job(&job).await.4.as_deref(), Some("LEASE_LAPSED"));
    let answer = shop
        .report_at(at(12_500), report_body(&job, &lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(shop.job(&job).await.0, "DRAWER_OPENED");
    shop.finish().await;
}

/// A late FAILED report after a lapse (the AGENT_RESTARTED path) replaces LEASE_LAPSED,
/// writes one audit line with the agent's actor, and a repeat answers 200 and writes nothing.
#[actix_web::test]
async fn a_late_failed_report_replaces_the_lapse_and_repeat_is_idempotent() {
    let shop = Shop::open().await;
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let lease = leased(&shop.claim_at(at(1_000), 0).await, &job);
    assert_eq!(shop.claim_at(at(12_000), 0).await.status, 204);
    assert_eq!(shop.job(&job).await.0, "FAILED");
    assert_eq!(shop.job(&job).await.4.as_deref(), Some("LEASE_LAPSED"));

    let report = json!({
        "jobId": job,
        "leaseId": lease,
        "outcome": "FAILED",
        "reason": "AGENT_RESTARTED",
        "detail": { "cupsJob": "POS80_-12" },
    });
    let answer = shop.report_at(at(12_500), report.clone()).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json()["recorded"], true);

    let j = shop.job(&job).await;
    assert_eq!(j.0, "FAILED");
    assert_eq!(j.4.as_deref(), Some("AGENT_RESTARTED"));

    let lines = shop.agent_lines().await;
    let failed_reports: Vec<_> = lines
        .iter()
        .filter(|l| {
            l.0 == "FAILED"
                && l.3.get("outcomeReason").and_then(|r| r.as_str()) == Some("AGENT_RESTARTED")
        })
        .collect();
    assert_eq!(failed_reports.len(), 1, "one FAILED line from the report");
    assert_eq!(failed_reports[0].1, format!("agent:{AGENT}"));
    assert_eq!(failed_reports[0].3["job"], job);

    let count_before = lines.len();
    let second_answer = shop.report_at(at(13_000), report).await;
    assert_eq!(second_answer.status, 200, "{}", second_answer.text);
    assert_eq!(second_answer.json()["recorded"], true);

    let lines_after = shop.agent_lines().await;
    assert_eq!(
        lines_after.len(),
        count_before,
        "a second identical report writes nothing"
    );
    shop.finish().await;
}

/// A repeat report naming another lease after DRAWER_OPENED answers 409 LEASE_MISMATCH.
#[actix_web::test]
async fn a_repeat_report_naming_another_lease_after_drawer_opened_answers_409() {
    let shop = Shop::open().await;
    let job = shop.queue_drawer(MAIN, 1, at(0)).await;
    let lease = leased(&shop.claim_at(at(1_000), 0).await, &job);

    let answer1 = shop
        .report_at(at(2_000), report_body(&job, &lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer1.status, 200, "{}", answer1.text);
    assert_eq!(answer1.json()["recorded"], true);
    assert_eq!(shop.job(&job).await.0, "DRAWER_OPENED");

    let another_lease = "00000000-0000-4000-8000-000000000099";
    let answer2 = shop
        .report_at(at(3_000), report_body(&job, another_lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer2.status, 409, "{}", answer2.text);
    assert_eq!(answer2.json()["error"], "LEASE_MISMATCH");

    let answer3 = shop
        .report_at(at(4_000), report_body(&job, &lease, "DRAWER_OPENED"))
        .await;
    assert_eq!(answer3.status, 200, "{}", answer3.text);
    assert_eq!(answer3.json()["recorded"], true);

    shop.finish().await;
}
