//! #6273: the owner saves a branch's kiosk screen values — `POST /api/v3/commands/kiosk_screen_save` — and the kiosk
//! `rates` answer (`screen`) serves them (owner 2026-11-05: "ว่า อันไหน คอนฟิก ได้ อย่า ฝังใหน code"; principle
//! #5993, `docs/design/blueprint/user-set-settings.md`).
//!
//! Written before the implementation by claude/37e371ca on the lane controller's order (claude/022a7453). Do not
//! edit, delete or weaken these tests; add tests in a file of your own. One test per row of the ticket's behaviour
//! table, named after the row.
//!
//! Shapes these tests assume (the ticket's skeleton, `dto/v3/kiosk_screen.rs`):
//! - Save body: `{ branch, quickAmounts, tiles, idleSeconds, reviewSeconds, refreshSeconds, popularDays,
//!   defaultPaymentMethod, expectedVersion }` + `Idempotency-Key`. A field sent `null` is not set: it answers its
//!   default. `expectedVersion` is `null` while the branch has no row. The answer is `KioskScreenData`, with the new
//!   `version`.
//! - A refusal is `{ error: { code, field? } }`.
//! - The defaults and the refresh bound are `application::kiosk_screen`'s; nothing here restates them.
//!
//! Callers are real sessions (`kiosk_http_common::Counter`): `owner` is the Owner account (ADMIN + EXECUTIVE); the
//! other keys are staff of `MAIN` with one v2 role each (`test_callers::V2_STAFF`).
//!
//! Needs `TEST_DATABASE_URL` naming a database ending `_test`.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, as_who, person};
use rpc_common::{Answer, Call};
use serde_json::{Value, json};
use xchange::application::kiosk_screen;

const MAIN: &str = "MAIN";
const SAVE: &str = "/api/v3/commands/kiosk_screen_save";

/// Every built-in staff role but EXECUTIVE, as v2 role names (`test_callers::V2_STAFF`).
const NOT_OWNERS: &[&str] = &["FRONT", "MANAGER", "RATES", "FINANCE", "SYSTEM", "MONITOR"];

async fn counter() -> Counter {
    let staff: Vec<_> = NOT_OWNERS
        .iter()
        .enumerate()
        .map(|(i, role)| person(role, role, 0x61 + i as u8))
        .collect();
    Counter::open(&staff).await
}

/// A save as `who`, with a fresh idempotency key.
async fn save(counter: &Counter, who: &str, body: Value) -> Answer {
    let key = format!("kiosk-screen-{}", uuid::Uuid::new_v4());
    let call = Call::post(&counter.origin, SAVE, body.to_string()).header("idempotency-key", &key);
    counter.app.send(as_who(call, &counter.cookies, who)).await
}

/// Every field not set, for `MAIN`, expecting no row.
fn unset() -> Value {
    json!({
        "branch": MAIN,
        "quickAmounts": null,
        "tiles": null,
        "idleSeconds": null,
        "reviewSeconds": null,
        "refreshSeconds": null,
        "popularDays": null,
        "defaultPaymentMethod": null,
        "expectedVersion": null,
    })
}

fn with(mut body: Value, fields: Value) -> Value {
    for (k, v) in fields.as_object().expect("fields are an object") {
        body[k] = v.clone();
    }
    body
}

/// The `screen` of the kiosk's `rates` answer, as the kiosk reads it.
async fn screen(counter: &Counter) -> Value {
    let answer = rates(counter).await;
    answer["screen"].clone()
}

async fn rates(counter: &Counter) -> Value {
    let answer = counter.ask(None, "rates", json!({})).await;
    assert_eq!(answer.status, 200, "rates: {}", answer.text);
    answer.json()
}

fn code(a: &Answer) -> Value {
    a.json()["error"]["code"].clone()
}

fn field(a: &Answer) -> Value {
    a.json()["error"]["field"].clone()
}

/// The branch's `kioskScreen` row: its version, or `None` without one.
async fn stored_version(counter: &Counter) -> Option<i64> {
    sqlx::query_scalar("SELECT version FROM branch_settings WHERE branch_id = $1 AND name = $2")
        .bind(MAIN)
        .bind(kiosk_screen::SETTING)
        .fetch_optional(&counter.desk.db.pool)
        .await
        .expect("read the kioskScreen row")
}

/// A payment method `rates` lists as enabled for at least one side, other than the not-set default.
async fn another_enabled_method(counter: &Counter) -> String {
    let answer = rates(counter).await;
    answer["paymentMethods"]
        .as_array()
        .unwrap_or_else(|| panic!("no paymentMethods in {answer}"))
        .iter()
        .find(|m| {
            m["id"] != json!(kiosk_screen::DEFAULT_PAYMENT_METHOD)
                && m["enabled"] == json!(true)
                && (m["buy"] == json!(true) || m["sell"] == json!(true))
        })
        .and_then(|m| m["id"].as_str())
        .unwrap_or_else(|| panic!("an enabled method besides the default in {answer}"))
        .to_owned()
}

/// K1: the owner saves the chips [100, 200, 500]; the kiosk `rates` answer serves them, marked as the owner's.
/// Restated on the ticket author's ruling (#6273, controller claude/022a7453): `source` is `"owner"`, the string
/// PR 6270 already ships, not the ticket's first wording `"set"`.
#[actix_web::test]
async fn k1_owner_saves_chips() {
    let counter = counter().await;
    let saved = save(
        &counter,
        "owner",
        with(unset(), json!({ "quickAmounts": [100, 200, 500] })),
    )
    .await;
    assert_eq!(saved.status, 200, "save: {}", saved.text);

    let screen = screen(&counter).await;
    assert_eq!(screen["quickAmounts"], json!([100, 200, 500]), "{screen}");
    assert_eq!(screen["source"], json!("owner"), "{screen}");
    counter.finish().await;
}

/// K2: every staff role but EXECUTIVE is refused, and nothing is written. Restated on the ticket author's ruling
/// (#6273, controller claude/022a7453): the refusal is `400 roleNotAllowedHere`, as every settings save refuses, not
/// the ticket's first wording 403.
#[actix_web::test]
async fn k2_a_non_owner_saves() {
    let counter = counter().await;
    for &who in NOT_OWNERS {
        let refused = save(
            &counter,
            who,
            with(unset(), json!({ "quickAmounts": [100, 200, 500] })),
        )
        .await;
        assert_eq!(refused.status, 400, "{who}: {}", refused.text);
        assert_eq!(
            code(&refused),
            json!("roleNotAllowedHere"),
            "{who}: {}",
            refused.text
        );
    }
    assert_eq!(stored_version(&counter).await, None, "nothing written");
    assert_eq!(
        screen(&counter).await["quickAmounts"],
        json!(kiosk_screen::DEFAULT_QUICK_AMOUNTS)
    );
    counter.finish().await;
}

/// K3: chips [0], or 9 chips, are refused 400 naming the field, and nothing is written.
#[actix_web::test]
async fn k3_chips_zero_or_nine_chips() {
    let counter = counter().await;
    for (what, chips) in [
        ("[0]", json!([0])),
        ("9 chips", json!([1, 2, 3, 4, 5, 6, 7, 8, 9])),
    ] {
        let refused = save(
            &counter,
            "owner",
            with(unset(), json!({ "quickAmounts": chips })),
        )
        .await;
        assert_eq!(refused.status, 400, "{what}: {}", refused.text);
        assert_eq!(
            field(&refused),
            json!("quickAmounts"),
            "{what}: {}",
            refused.text
        );
    }
    assert_eq!(stored_version(&counter).await, None, "nothing written");
    counter.finish().await;
}

/// K4: a default payment method the branch enables for neither side is refused 400, and nothing is written.
#[actix_web::test]
async fn k4_default_payment_method_disabled_for_both_sides() {
    let counter = counter().await;
    let method = another_enabled_method(&counter).await;

    // The owner switches it off on both sides through the payment methods screen.
    let list = Call::action(
        &counter.origin,
        "/api/payment-methods",
        "list",
        json!({ "branchId": MAIN }),
    );
    let listed = counter
        .app
        .send(as_who(list, &counter.cookies, "owner"))
        .await;
    assert_eq!(listed.status, 200, "list: {}", listed.text);
    let listed = listed.json();
    let mut rows = listed["methods"].clone();
    let row = rows
        .as_array_mut()
        .expect("methods is an array")
        .iter_mut()
        .find(|r| r["id"] == json!(method))
        .unwrap_or_else(|| panic!("{method} in {listed}"));
    row["enabled"] = json!(false);
    row["buy"] = json!(false);
    row["sell"] = json!(false);
    let switch_off = Call::action(
        &counter.origin,
        "/api/payment-methods",
        "save",
        json!({ "branchId": MAIN, "version": listed["version"], "methods": rows }),
    );
    let switched = counter
        .app
        .send(as_who(switch_off, &counter.cookies, "owner"))
        .await;
    assert_eq!(
        switched.status, 200,
        "switch {method} off: {}",
        switched.text
    );

    let refused = save(
        &counter,
        "owner",
        with(unset(), json!({ "defaultPaymentMethod": method })),
    )
    .await;
    assert_eq!(refused.status, 400, "{method}: {}", refused.text);
    assert_eq!(stored_version(&counter).await, None, "nothing written");
    counter.finish().await;
}

/// K5: resetting one field (sending it `null`) makes it answer its default; the other fields keep their set values.
#[actix_web::test]
async fn k5_reset_one_field() {
    let counter = counter().await;
    let method = another_enabled_method(&counter).await;
    let all_set = json!({
        "quickAmounts": [100, 200, 500],
        "tiles": kiosk_screen::DEFAULT_TILES + 2,
        "idleSeconds": kiosk_screen::DEFAULT_IDLE_SECONDS + 60,
        "reviewSeconds": kiosk_screen::DEFAULT_REVIEW_SECONDS + 2,
        "refreshSeconds": kiosk_screen::MAX_REFRESH_SECONDS,
        "popularDays": kiosk_screen::DEFAULT_POPULAR_DAYS + 30,
        "defaultPaymentMethod": method,
    });
    let first = save(&counter, "owner", with(unset(), all_set.clone())).await;
    assert_eq!(first.status, 200, "first save: {}", first.text);
    let version = first.json()["version"].clone();
    assert!(
        version.is_i64(),
        "the answer carries the new version: {}",
        first.text
    );

    let reset = save(
        &counter,
        "owner",
        with(
            with(unset(), all_set.clone()),
            json!({ "tiles": null, "expectedVersion": version }),
        ),
    )
    .await;
    assert_eq!(reset.status, 200, "reset tiles: {}", reset.text);

    let screen = screen(&counter).await;
    assert_eq!(
        screen["tiles"],
        json!(kiosk_screen::DEFAULT_TILES),
        "{screen}"
    );
    assert_eq!(screen["defaults"], json!(["tiles"]), "{screen}");
    for kept in [
        "quickAmounts",
        "idleSeconds",
        "reviewSeconds",
        "refreshSeconds",
        "popularDays",
        "defaultPaymentMethod",
    ] {
        assert_eq!(
            screen[kept], all_set[kept],
            "{kept} keeps its set value: {screen}"
        );
    }
    counter.finish().await;
}

/// K6: a save naming a version other than the row's is refused 409 `staleVersion`, and nothing is written.
#[actix_web::test]
async fn k6_stale_version() {
    let counter = counter().await;
    let first = save(
        &counter,
        "owner",
        with(unset(), json!({ "quickAmounts": [100, 200, 500] })),
    )
    .await;
    assert_eq!(first.status, 200, "first save: {}", first.text);
    let version = stored_version(&counter).await;
    assert!(version.is_some(), "the first save wrote a row");

    for (what, expected) in [
        ("null with a row", json!(null)),
        ("a later version", json!(99)),
    ] {
        let stale = save(
            &counter,
            "owner",
            with(
                unset(),
                json!({ "quickAmounts": [700], "expectedVersion": expected }),
            ),
        )
        .await;
        assert_eq!(stale.status, 409, "{what}: {}", stale.text);
        assert_eq!(
            code(&stale),
            json!("staleVersion"),
            "{what}: {}",
            stale.text
        );
    }
    assert_eq!(stored_version(&counter).await, version, "nothing written");
    assert_eq!(
        screen(&counter).await["quickAmounts"],
        json!([100, 200, 500])
    );
    counter.finish().await;
}
