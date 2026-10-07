//! Written before the implementation (ticket #283). Do not edit, delete or weaken.
//!
//! Two rules of v2's `saveTransaction` that the recorded replays cannot see,
//! held through `POST /api/staff/rpc`:
//!
//! - **The order of the checks.** v2 asks for the cash count in its proxy
//!   (`hookRpc`), before the engine; in the engine the kiosk's `beforeSave`
//!   comes before custody (`docs/design/xchange-v3.md` §5.4 and the P2-S row;
//!   #153). A save that two of them would refuse is answered with the earlier
//!   one's sentence. No recorded save is refused by two of them, so the
//!   replays stay green whatever order `SaveAction::new` registers the steps
//!   in (PR 279, review follow-up a).
//! - **Only the JSON `true` confirms the count.** v2 refuses a cash save whose
//!   `countConfirmed` is anything but `true` (`api/tests/fixtures/devices/
//!   v2_device_cases.json` → `countConfirmed`: `false`, absent, `"true"` and
//!   `1` refused, `api/tools/v2-fixtures/gen_devices.mjs:370-382`). The
//!   replays send only `true` or nothing (PR 279, review follow-up b).
//!
//! Every case is a variation of `rpc/ops_actions.json` #99 (FR2, whom custody
//! refuses) or #100 (FR1, whom custody lets save), sent after #1–#98 are
//! replayed as `rpc_save_test.rs` replays them. Every save is live (a `DEMO`
//! save is not count-checked, a decided difference of §5.4) and a first save
//! of its submission (a repeat is not count-checked either). After the
//! variations #99 and #100 are replayed and held to what v2 answered, so a
//! refused variation is also shown to have left nothing behind.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use compat_common::Fixture;
use rpc_common::{Answer, Desk, INVALID_REQUEST, MAIN, Replay, Rule, call_of};
use serde_json::{Map, Value, json};
use snapshot_common::time;

const OPS_FILE: &str = "rpc/ops_actions.json";

/// `ops_actions.json`: the steps `rpc_ops_test.rs` answers by a decided
/// difference, the owner's door it does not send, and where custody was
/// switched on (as `rpc_save_test.rs` replays them).
const UNKNOWN_NAME_BY_STATE: [usize; 3] = [2, 23, 40];
const OWNERS_DOOR: std::ops::RangeInclusive<usize> = 9..=17;
const SWITCHED_ON_AT: usize = 15;
/// `ops_actions.json` #99: FR2, who does not hold the drawer, saves cash;
/// custody refuses.
const CUSTODY_REFUSES: usize = 99;
/// `ops_actions.json` #100: FR1 saves cash under the draft of #95; accepted.
const CUSTODY_LETS_SAVE: usize = 100;

/// v2's proxy: a cash save whose count was not confirmed
/// (`v2_device_cases.json` → `countFirst`).
const COUNT_FIRST: &str = "ยืนยันการนับเงินก่อนบันทึกรายการเงินสด";
/// The kiosk's refusal of a save naming a request that is not ready, not the
/// saver's, or not there (v2 `server.mjs:8064`).
const KIOSK_NOT_READY: &str = "คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว";
/// Custody's refusal of #99, as v2 recorded it.
const NOT_THE_DRAWER: &str = "ต้องเป็น Front ผู้รับผิดชอบลิ้นชักที่ลงเวลาเข้าแล้ว";

/// A kiosk request nobody made.
const NOBODYS_REQUEST: &str = "00000000-0000-4000-8000-0000000d0f01";

/// A submission no step of the file uses: every variation is a first save.
fn fresh_submission(n: u8) -> Value {
    json!(format!("00000000-0000-4000-8000-0000000d00{n:02x}"))
}

/// The desk as it was just before #99: #1–#98 replayed and held to v2.
async fn at_the_saves(fixture: &Fixture) -> Replay<'_> {
    assert_eq!(
        fixture.step(CUSTODY_REFUSES).error(),
        Some(NOT_THE_DRAWER),
        "v2 refused #99 for custody"
    );
    assert_eq!(fixture.step(CUSTODY_LETS_SAVE).status, Some(200));
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
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
    for n in 1..CUSTODY_REFUSES {
        if n == SWITCHED_ON_AT {
            replay.desk.custody_on(MAIN).await;
        }
        replay.step(fixture.step(n)).await;
    }
    replay
}

/// The save of step `n`, sent by whoever sent it at the time it was sent,
/// with its payload changed by `edit`.
async fn save_like(
    replay: &Replay<'_>,
    n: usize,
    edit: impl FnOnce(&mut Map<String, Value>),
) -> Answer {
    let step = replay.fixture.step(n);
    assert_eq!(step.action.as_deref(), Some("saveTransaction"), "#{n}");
    replay.desk.clock.set(time(&step.at));
    let mut payload = replay
        .ids
        .to_actual(step.payload.as_ref().expect("a payload"));
    let fields = payload.as_object_mut().expect("an object payload");
    assert_eq!(fields["mode"], json!("LIVE"), "#{n}");
    assert_eq!(fields["method"], json!("Cash"), "#{n}");
    edit(fields);
    replay
        .app
        .send(call_of(
            replay.fixture,
            step,
            &replay.cookies,
            Some(&payload),
        ))
        .await
}

/// #99 and #100 replayed and held to what v2 answered: custody's refusal, then
/// the first exchange of the day (`XC-20261006-000001`) completing the draft.
async fn the_day_goes_on_as_recorded(mut replay: Replay<'_>) {
    let fixture = replay.fixture;
    replay.step(fixture.step(CUSTODY_REFUSES)).await;
    replay.step(fixture.step(CUSTODY_LETS_SAVE)).await;
    assert_eq!(
        replay.answers[&CUSTODY_LETS_SAVE].json()["documentRef"],
        json!("XC-20261006-000001")
    );
    replay.finish().await;
}

// ── the order of the checks ──────────────────────────────────────────────────

/// FR1 may save (#100), but the count is not confirmed and the save names a
/// kiosk request nobody made. v2's proxy refuses for the count before the
/// engine — and the kiosk's `beforeSave` in it — is asked.
#[actix_web::test]
async fn the_count_is_asked_before_a_kiosk_request_is_judged() {
    let fixture = Fixture::load(OPS_FILE);
    let replay = at_the_saves(&fixture).await;

    let answer = save_like(&replay, CUSTODY_LETS_SAVE, |p| {
        p.remove("countConfirmed");
        p.insert("kioskRequestId".into(), json!(NOBODYS_REQUEST));
        p.insert("submissionId".into(), fresh_submission(1));
    })
    .await;
    answer.assert_refused(
        400,
        COUNT_FIRST,
        "#100 uncounted and naming a request nobody made: the count first",
    );

    the_day_goes_on_as_recorded(replay).await;
}

/// FR2 does not hold the drawer (#99), and the count is not confirmed. v2's
/// proxy refuses for the count before the engine — and custody in it — is
/// asked.
#[actix_web::test]
async fn the_count_is_asked_before_custody() {
    let fixture = Fixture::load(OPS_FILE);
    let replay = at_the_saves(&fixture).await;

    let answer = save_like(&replay, CUSTODY_REFUSES, |p| {
        p.remove("countConfirmed");
    })
    .await;
    answer.assert_refused(
        400,
        COUNT_FIRST,
        "#99 uncounted, by a Front without the drawer: the count first",
    );

    the_day_goes_on_as_recorded(replay).await;
}

/// FR2 does not hold the drawer (#99), the count is confirmed, and the save
/// names a kiosk request nobody made. In v2's engine the kiosk's `beforeSave`
/// comes before custody (§5.4, #153), so the kiosk's sentence is the answer.
#[actix_web::test]
async fn a_kiosk_request_is_judged_before_custody() {
    let fixture = Fixture::load(OPS_FILE);
    let replay = at_the_saves(&fixture).await;

    let answer = save_like(&replay, CUSTODY_REFUSES, |p| {
        assert_eq!(p["countConfirmed"], json!(true));
        p.insert("kioskRequestId".into(), json!(NOBODYS_REQUEST));
    })
    .await;
    answer.assert_refused(
        400,
        KIOSK_NOT_READY,
        "#99 naming a request nobody made, by a Front without the drawer: the kiosk first",
    );

    the_day_goes_on_as_recorded(replay).await;
}

// ── what confirms the count ──────────────────────────────────────────────────

/// #100 with `countConfirmed` set to anything but the JSON `true` is refused
/// for the count — v2 recorded `false`, `"true"` and `1`; the others are the
/// same rule (`countConfirmed === true`). The recorded #100, with `true`, is
/// then accepted as v2 accepted it.
#[actix_web::test]
async fn only_the_json_true_confirms_the_count() {
    let fixture = Fixture::load(OPS_FILE);
    let replay = at_the_saves(&fixture).await;

    let not_a_confirmation = [
        json!("true"),
        json!(1),
        json!("1"),
        json!({}),
        json!([]),
        json!(false),
        Value::Null,
    ];
    for (i, value) in not_a_confirmation.into_iter().enumerate() {
        let label = format!("#100 with countConfirmed = {value}");
        let answer = save_like(&replay, CUSTODY_LETS_SAVE, |p| {
            p.insert("countConfirmed".into(), value);
            p.insert("submissionId".into(), fresh_submission(0x10 + i as u8));
        })
        .await;
        answer.assert_refused(400, COUNT_FIRST, &label);
    }

    the_day_goes_on_as_recorded(replay).await;
}
