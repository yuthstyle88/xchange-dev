//! CA-10 #626 (design `docs/design/blueprint/cash-shift-transfer.md` §6, rulings Q4 and Q7):
//! the rules of a compat `saveTransaction` that `rpc_save_order_test.rs` and `rpc_save_test.rs`
//! held for v2's Front, restated for the Blueprint's teller.
//! Written before the implementation by the ticket author (claude/c4d72c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Those two files replayed `rpc/ops_actions.json` #1–#98 to give v2's Front the drawer through
//! the custody request model, then varied the save of #99/#100. Design §6 retires that fixture with
//! the request model, and Q4 retires the v2-actor save path: a save is a teller's, on an OPEN shift
//! of their own. The rules they held that still stand are held here, on the same route
//! (`POST /api/staff/rpc`), by the day's FRONT (`FR1`, a TELLER), whose shift `rpc_common`'s
//! replay of `rpc/engine_actions.json` opened by a count (#40, `Replay::retired_behind`):
//! - **the order of the checks** (`docs/design/xchange-v3.md` §5.4, #153): the cash count is asked
//!   first, then a kiosk request the save names, then custody (here: the teller's own OPEN shift);
//! - **only the JSON `true` confirms the count** (`countConfirmed === true`).
//!
//! What the two files held that is pinned elsewhere: a save completing the draft it names
//! (`draft_completion_test.rs`, `branch_day_save_test.rs`), the staff links of an answer
//! (`rpc_action_seam_test.rs`), the count's sentence itself (`tx5_complete_test.rs`,
//! `device_save_step_test.rs`).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
#[allow(clippy::duplicate_mod)]
mod shift_setup_common;
mod snapshot_common;

use chrono::Utc;
use compat_common::Fixture;
use rpc_common::{Answer, Call, Desk, Replay, STAFF_RPC};
use serde_json::{Map, Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const FILE: &str = "rpc/engine_actions.json";
/// The day's FRONT, a TELLER (OR-4b B1), whose shift the replay opened by a count at #40.
const TELLER: &str = "FR1";
/// `engine_actions.json` #120: a LIVE BUY of USD 100 at 32.2 by cash for `CX-00000001`.
const LIVE_BUY: usize = 120;

/// `ยืนยันการนับเงินก่อนบันทึกรายการเงินสด` (`devices_common::COUNT_FIRST`).
const COUNT_FIRST: &str = "ยืนยันการนับเงินก่อนบันทึกรายการเงินสด";
/// The kiosk's refusal of a request it cannot save (`rpc_save_order_test.rs` measured it).
const KIOSK_NOT_READY: &str = "คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว";
/// Custody's refusal of a teller with no OPEN shift of their own (`ShiftError::NoOpenShift`, measured).
const NO_OWN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";

/// A kiosk request nobody made.
const NOBODYS_REQUEST: &str = "00000000-0000-4000-8000-0000000d0f01";

/// A submission no step of the day uses: every variation is a first save.
fn fresh_submission(n: u8) -> Value {
    json!(format!("00000000-0000-4000-8000-0000000d00{n:02x}"))
}

/// The day replayed through the routes up to just before #120: the teller's shift is open.
async fn at_the_save(fixture: &Fixture) -> Replay<'_> {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
    for n in 1..LIVE_BUY {
        replay.step(fixture.step(n)).await;
    }
    replay
}

/// The teller's shift closed by a CLOSE count of what it holds: custody now refuses the teller.
async fn without_an_own_shift(replay: &Replay<'_>) {
    use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
    let pool = &replay.desk.db.pool;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(pool)
            .await
            .expect("the open shift");
    let held =
        xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances::new(pool.clone())
            .shift_balances(&shift_id)
            .await
            .expect("the open shift's balances");
    let counted: Map<String, Value> = held
        .into_iter()
        .map(|b| {
            (
                b.currency.as_str().to_owned(),
                json!(xc_kernel::money::fixed2(b.expected.minor())),
            )
        })
        .collect();
    shift_setup_common::close_shift_by_count(pool, "MAIN", Value::Object(counted), Utc::now())
        .await;
}

/// #120's save, by the teller on the staff route, with its payload changed by `edit`.
async fn save_like(replay: &Replay<'_>, edit: impl FnOnce(&mut Map<String, Value>)) -> Answer {
    let step = replay.fixture.step(LIVE_BUY);
    assert_eq!(step.action.as_deref(), Some("saveTransaction"));
    replay.desk.clock.set(time(&step.at));
    let mut payload = replay
        .ids
        .to_actual(step.payload.as_ref().expect("a payload"));
    let fields = payload.as_object_mut().expect("an object payload");
    assert_eq!(fields["mode"], json!("LIVE"));
    assert_eq!(fields["method"], json!("Cash"));
    fields.insert("countConfirmed".into(), json!(true));
    edit(fields);
    // OR-4b (C2): a save attaches its saver's own files
    let payload = replay.uploads_as(TELLER, &payload).await;
    replay
        .app
        .send(
            Call::action(ORIGIN, STAFF_RPC, "saveTransaction", payload)
                .cookie(&replay.cookies[TELLER]),
        )
        .await
}

// ── the order of the checks ──────────────────────────────────────────────────

/// The teller may save, but the count is not confirmed and the save names a kiosk request nobody
/// made: the count is asked first.
#[actix_web::test]
async fn the_count_is_asked_before_a_kiosk_request_is_judged() {
    let fixture = Fixture::load(FILE);
    let replay = at_the_save(&fixture).await;
    save_like(&replay, |p| {
        p.remove("countConfirmed");
        p.insert("kioskRequestId".into(), json!(NOBODYS_REQUEST));
        p.insert("submissionId".into(), fresh_submission(1));
    })
    .await
    .assert_refused(400, COUNT_FIRST, "uncounted, naming a request nobody made");
    replay.finish().await;
}

/// Custody refuses the teller (no OPEN shift of their own), and the count is not confirmed: the
/// count is asked first.
#[actix_web::test]
async fn the_count_is_asked_before_custody() {
    let fixture = Fixture::load(FILE);
    let replay = at_the_save(&fixture).await;
    without_an_own_shift(&replay).await;
    save_like(&replay, |p| {
        p.remove("countConfirmed");
        p.insert("submissionId".into(), fresh_submission(2));
    })
    .await
    .assert_refused(400, COUNT_FIRST, "uncounted, by a teller with no shift");
    replay.finish().await;
}

/// Custody refuses the teller, the count is confirmed, and the save names a kiosk request nobody
/// made: the kiosk comes before custody (§5.4, #153).
#[actix_web::test]
async fn a_kiosk_request_is_judged_before_custody() {
    let fixture = Fixture::load(FILE);
    let replay = at_the_save(&fixture).await;
    without_an_own_shift(&replay).await;
    save_like(&replay, |p| {
        p.insert("kioskRequestId".into(), json!(NOBODYS_REQUEST));
        p.insert("submissionId".into(), fresh_submission(3));
    })
    .await
    .assert_refused(
        400,
        KIOSK_NOT_READY,
        "naming a request nobody made, by a teller with no shift",
    );
    // and without the kiosk request, custody's own refusal: the order above is not vacuous
    save_like(&replay, |p| {
        p.insert("submissionId".into(), fresh_submission(4));
    })
    .await
    .assert_refused(400, NO_OWN_SHIFT, "counted, by a teller with no shift");
    replay.finish().await;
}

/// A teller who may save, naming a kiosk request nobody made, is refused by the kiosk
/// (`rpc_save_test.rs`, v2 `server.mjs:8064`).
#[actix_web::test]
async fn a_save_naming_a_kiosk_request_nobody_made_is_refused() {
    let fixture = Fixture::load(FILE);
    let replay = at_the_save(&fixture).await;
    save_like(&replay, |p| {
        p.insert("kioskRequestId".into(), json!(NOBODYS_REQUEST));
        p.insert("submissionId".into(), fresh_submission(5));
    })
    .await
    .assert_refused(400, KIOSK_NOT_READY, "naming a request nobody made");
    replay.finish().await;
}

// ── what confirms the count ──────────────────────────────────────────────────

/// `countConfirmed` set to anything but the JSON `true` is refused for the count; with `true`
/// the same save is accepted, so each refusal above is the count's.
#[actix_web::test]
async fn only_the_json_true_confirms_the_count() {
    let fixture = Fixture::load(FILE);
    let replay = at_the_save(&fixture).await;
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
        let label = format!("countConfirmed = {value}");
        save_like(&replay, |p| {
            p.insert("countConfirmed".into(), value);
            p.insert("submissionId".into(), fresh_submission(0x10 + i as u8));
        })
        .await
        .assert_refused(400, COUNT_FIRST, &label);
    }
    let accepted = save_like(&replay, |p| {
        p.insert("submissionId".into(), fresh_submission(0x20));
    })
    .await;
    assert_eq!(
        accepted.status, 200,
        "countConfirmed = true: {:.300}",
        accepted.text
    );
    replay.finish().await;
}
