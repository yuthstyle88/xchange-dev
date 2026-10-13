//! Written before the implementation (ticket P2-S). Do not edit, delete or weaken.
//!
//! The save family — `saveTransaction` on `POST /api/rpc` and
//! `POST /api/staff/rpc` — for what the recorded replays cannot see. The
//! replays (`rpc_test.rs` for `rpc/engine_actions.json`, `rpc_ops_test.rs` for
//! `rpc/ops_actions.json`, `rpc_documents_test.rs` for
//! `snapshot/transactions.json`) hold every recorded save step once
//! `rpc_common/gap_save.rs` says `WIRED`; they were measured to stay green
//! when:
//!
//! - the payload's `staffDraftId` is not passed to the save: nothing reads the
//!   draft after ops #100. Held here by reading it (v2: `ops_actions.json`
//!   #135, the draft `COMPLETED` with the save's number);
//! - `folderUrl` and `sheetUrl` name `MAIN` whatever the branch: every
//!   recorded save is in `MAIN`. Held here by a save in another branch (v2:
//!   `server.mjs:6853`, both built from the branch of the request);
//! - the payload's `kioskRequestId` is not passed, or the kiosk's step is not
//!   registered: no recorded save names a kiosk request. Held here by a save
//!   naming a request nobody made (v2: `server.mjs:8064`).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use compat_common::Fixture;
use rpc_common::{App, Call, Cookies, Desk, INVALID_REQUEST, MAIN, RPC, Replay, Rule};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const OPS_FILE: &str = "rpc/ops_actions.json";
const ENGINE_FILE: &str = "rpc/engine_actions.json";

/// `ops_actions.json`: the steps `rpc_ops_test.rs` answers by a decided
/// difference, the owner's door it does not send, and where custody was
/// switched on.
const UNKNOWN_NAME_BY_STATE: [usize; 3] = [2, 23, 40];
const OWNERS_DOOR: std::ops::RangeInclusive<usize> = 9..=17;
const SWITCHED_ON_AT: usize = 15;
/// `ops_actions.json`: FR1 begins the draft (#95) and saves under it (#100).
const DRAFT_BEGUN_AT: usize = 95;
const SAVED_UNDER_THE_DRAFT_AT: usize = 100;

/// The kiosk's refusal of a save naming a request that is not ready, not the
/// saver's, or not there (v2 `server.mjs:8064`).
const KIOSK_NOT_READY: &str = "คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว";

/// The first bytes of a PNG: enough for the upload route.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
];

async fn owner_snapshot(app: &App, cookies: &Cookies, branch: &str) -> Value {
    let answer = app
        .send(
            Call::action(ORIGIN, RPC, "adminSnapshot", json!({ "branchId": branch }))
                .cookie(&cookies["owner"]),
        )
        .await;
    assert_eq!(
        answer.status, 200,
        "the owner's snapshot: {:.300}",
        answer.text
    );
    answer.json()
}

// ── the draft a save names ───────────────────────────────────────────────────

/// ops #100: FR1 saves under the draft of #95. After it the owner sees that
/// draft completed, carrying the number of the exchange — as v2's #135 shows.
#[actix_web::test]
async fn a_save_completes_the_draft_it_names() {
    let fixture = Fixture::load(OPS_FILE);
    let begun = fixture.step(DRAFT_BEGUN_AT);
    assert_eq!(begun.action.as_deref(), Some("opsBeginTransaction"));
    assert_eq!(begun.status, Some(200));
    let saved = fixture.step(SAVED_UNDER_THE_DRAFT_AT);
    assert_eq!(saved.action.as_deref(), Some("saveTransaction"));
    let recorded_draft = begun.json()["id"]
        .as_str()
        .expect("the id of the draft")
        .to_string();
    assert_eq!(
        saved.payload.as_ref().expect("a payload")["staffDraftId"],
        json!(recorded_draft),
        "#100 names the draft of #95"
    );

    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    for n in UNKNOWN_NAME_BY_STATE {
        replay = replay.rule(
            n,
            Rule::Differs {
                status: 400,
                sentence: INVALID_REQUEST,
            },
        );
    }
    for n in OWNERS_DOOR {
        replay = replay.rule(n, Rule::Excluded("the owner's door: another ticket's"));
    }
    for n in 1..=SAVED_UNDER_THE_DRAFT_AT {
        if n == SWITCHED_ON_AT {
            replay.desk.custody_on(MAIN).await;
        }
        replay.step(fixture.step(n)).await;
    }

    let document_ref = replay.answers[&SAVED_UNDER_THE_DRAFT_AT].json()["documentRef"].clone();
    assert_eq!(document_ref, json!("XC-20261006-000001"));
    let draft_id = replay
        .ids
        .actual(&recorded_draft)
        .expect("the draft v3 made for #95")
        .to_string();
    let snapshot = owner_snapshot(&replay.app, &replay.cookies, MAIN).await;
    let drafts = snapshot["operations"]["drafts"]
        .as_array()
        .expect("the drafts of the branch");
    let draft = drafts
        .iter()
        .find(|d| d["id"] == json!(draft_id))
        .unwrap_or_else(|| panic!("the draft {draft_id} among {drafts:?}"));
    assert_eq!(draft["status"], json!("COMPLETED"), "{draft}");
    assert_eq!(draft["documentRef"], document_ref, "{draft}");
    replay.finish().await;
}

// ── a branch that is not MAIN ────────────────────────────────────────────────

/// The owner in a branch of their own making: the branch, the customer and the
/// passport of a DEMO exchange there (a DEMO save needs no rate and no shift,
/// `engine_actions.json` #125). Answers the branch's id, and (OR-4b #767, C2) the key of the
/// branch's TELLER, who reserves the customer and saves: the owner's user no longer does.
async fn a_second_branch(replay: &mut Replay<'_>) -> (String, String, String, String) {
    let owner = replay.cookies["owner"].clone();
    let owner = &owner;
    let made = replay
        .app
        .send(
            Call::action(
                ORIGIN,
                RPC,
                "adminCreateBranch",
                json!({ "requestId": "00000000-0000-4000-8000-0000000c0001", "name": "Airport" }),
            )
            .cookie(owner),
        )
        .await;
    assert_eq!(made.status, 200, "the branch: {:.300}", made.text);
    let branch = made.json()["branchId"]
        .as_str()
        .expect("the branch's id")
        .to_string();
    assert_ne!(branch, MAIN);
    let teller = replay.employ_teller(&branch).await;

    let reserved = replay
        .app
        .send(
            Call::action(
                ORIGIN,
                RPC,
                "reserveCustomer",
                json!({
                    "branchId": branch, "name": "Customer 1", "passportNo": "P1000001",
                    "nationality": "DEU", "birthDate": "1990-01-02",
                    "passportExpiry": "2031-06-11", "mode": "DEMO",
                }),
            )
            // OR-4b (C2): ExchangeTransact is TELLER's
            .cookie(&replay.cookies[&teller]),
        )
        .await;
    assert_eq!(reserved.status, 200, "the customer: {:.300}", reserved.text);
    let customer = reserved.json()["customerId"]
        .as_str()
        .expect("the customer's number")
        .to_string();

    let uploaded = replay
        .app
        .send(
            // OR-4b (C2): the teller who saves uploads the passport (a save attaches its saver's)
            Call::post(ORIGIN, "/api/staff/upload", PNG)
                .header("content-type", "image/png")
                .cookie(&replay.cookies[&teller]),
        )
        .await;
    assert_eq!(uploaded.status, 200, "the passport: {:.300}", uploaded.text);
    let passport = uploaded.json()["uploadKey"]
        .as_str()
        .expect("the passport's key")
        .to_string();
    (branch, customer, passport, teller)
}

/// A DEMO exchange of `engine_actions.json` #125, in `branch`.
fn demo_save(branch: &str, customer: &str, passport: &str, submission: &str) -> Value {
    json!({
        "branchId": branch,
        "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
        "birthDate": "1990-01-02", "passportExpiry": "2031-06-11", "mode": "DEMO",
        "submissionId": submission, "customerId": customer,
        "type": "BUY", "currency": "USD", "amount": 100, "rate": 1, "fee": 0, "bankFee": 0,
        "method": "Cash", "ref": "", "account": "Cash Drawer", "staff": "desk",
        "booking": "WALK-IN", "notes": "", "valueDate": "2026-10-06",
        "paidAt": "2026-10-06T07:04:00.000Z",
        "paymentConfirmed": true, "identityConfirmed": true, "countConfirmed": true,
        "passport": { "uploadKey": passport },
    })
}

/// v2 builds both links from the branch of the request (`server.mjs:6853`):
/// the documents of the exchange, and the transactions sheet of that branch.
#[actix_web::test]
async fn the_links_of_a_save_name_its_own_branch() {
    let fixture = Fixture::load(ENGINE_FILE);
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    let (branch, customer, passport, teller) = a_second_branch(&mut replay).await;

    // Re-expected for TX-6 by the test-writer, decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): a DEMO save is quoted like a LIVE one, so
    // in a branch that publishes nothing it is refused; the branch then publishes the dollar at the
    // rate the save names (1), and the save's links are what this test holds.
    replay
        .app
        .send(
            Call::action(
                ORIGIN,
                RPC,
                "saveTransaction",
                demo_save(
                    &branch,
                    &customer,
                    &passport,
                    "00000000-0000-4000-8000-0000000c0005",
                ),
            )
            .cookie(&replay.cookies[&teller]),
        )
        .await
        .assert_refused(
            400,
            "rateUnavailable: no published line",
            "a DEMO save with nothing published (decision A2)",
        );
    snapshot_common::publish_rate_per_one(&replay.desk.db.pool, &branch, "USD", "1", "1").await;

    let answer = replay
        .app
        .send(
            Call::action(
                ORIGIN,
                RPC,
                "saveTransaction",
                demo_save(
                    &branch,
                    &customer,
                    &passport,
                    "00000000-0000-4000-8000-0000000c0002",
                ),
            )
            // OR-4b (C2): the branch's TELLER saves
            .cookie(&replay.cookies[&teller]),
        )
        .await;
    assert_eq!(answer.status, 200, "the save: {:.300}", answer.text);
    let saved = answer.json();
    let document_ref = saved["documentRef"].as_str().expect("a number");
    assert_eq!(
        saved["folderUrl"],
        json!(format!("/documents?branch={branch}&ref={document_ref}")),
        "{saved}"
    );
    assert_eq!(
        saved["sheetUrl"],
        json!(format!("/?branch={branch}#transactions")),
        "{saved}"
    );
    replay.finish().await;
}

// ── a kiosk request ──────────────────────────────────────────────────────────

/// A save that names a kiosk request is the kiosk's to judge before anything
/// is saved: one nobody made is refused with the kiosk's sentence, and no
/// exchange exists afterwards.
#[actix_web::test]
async fn a_save_naming_a_kiosk_request_nobody_made_is_refused() {
    let fixture = Fixture::load(ENGINE_FILE);
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    let (branch, customer, passport, teller) = a_second_branch(&mut replay).await;

    let mut payload = demo_save(
        &branch,
        &customer,
        &passport,
        "00000000-0000-4000-8000-0000000c0003",
    );
    payload["kioskRequestId"] = json!("00000000-0000-4000-8000-0000000c0004");
    let answer = replay
        .app
        .send(
            // OR-4b (C2): the branch's TELLER saves
            Call::action(ORIGIN, RPC, "saveTransaction", payload).cookie(&replay.cookies[&teller]),
        )
        .await;
    answer.assert_refused(400, KIOSK_NOT_READY, "a request nobody made");

    let snapshot = owner_snapshot(&replay.app, &replay.cookies, &branch).await;
    assert_eq!(
        snapshot["transactions"],
        json!([]),
        "nothing was saved in the branch"
    );
    replay.finish().await;
}
