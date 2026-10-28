//! PV-16, PV-17 and PV-18 (pricing-tiers.md §15.8, §15.8a; `H:` §8): `rateDisplaySave` (`POST /api/pricing/display`),
//! which settings a branch reads (its row, else MAIN's, else V.9's defaults), and the public projection's `display`
//! block — `{tiers, featured, highlight, order, more}`.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - PV-16: featured with 11 codes, or `tiers: []` ⇒ `validationFailed`; a caller who is not EXECUTIVE ⇒ refused;
//!   nothing written.
//! - PV-17: no row at B, MAIN has one ⇒ B's projection and board read MAIN's (`display.from = "MAIN"`); `clear: true`
//!   at B deletes B's row and returns it to MAIN's; no row anywhere ⇒ `H:` §8's defaults (`from = "default"`).
//! - PV-18: `allMarket` and Super Rich lists KHR, which the branch has no price for ⇒ KHR in `display.more`, no line,
//!   no price; `allMarket` off ⇒ `more` is empty. `order` is featured first (their set order), then the currencies
//!   Super Rich lists, then by code.
//!
//! reading: Super Rich's "market order" (§15.8) is not stored anywhere today (`rate_reference_lines` has no position,
//! and `SuperRichFeed` reads the list into a code-keyed map), so PV-18 pins only what does not depend on it: one
//! priced, non-featured currency SR lists (GBP) comes before priced ones SR does not list (AUD, CNY, by code), even
//! though AUD and CNY sort before GBP.

mod v9_follow_common;

use serde_json::{Value, json};
use v9_follow_common::*;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const BR_B: &str = "BR-B";

/// `H:` §8 / V9:display `DEFAULT_DISPLAY`: featured is a code default, not a statistic.
const DEFAULT_FEATURED: [&str; 10] = [
    "MYR", "CNY", "INR", "RUB", "KRW", "JPY", "GBP", "USD", "TWD", "EUR",
];

async fn board_display(room: &Room, branch: &str) -> Value {
    let a = view(room, &owner(), "pricing_board", branch, None).await;
    a.ok("pricing_board")["data"]["display"].clone()
}

async fn branch_b(room: &Room) {
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    room.publish_manual(
        BR_B,
        &[
            ("USD", "1", "35.10", "35.50"),
            ("EUR", "1", "38.10", "38.70"),
        ],
    )
    .await;
}

// ------------------------------------------------------------ PV-16

/// PV-16: `rateDisplaySave` with 11 featured codes is `validationFailed` (`H:` §8: at most 10); nothing written.
#[tokio::test]
async fn rate_display_save_with_eleven_featured_codes_is_validation_failed() {
    let room = Room::start().await;
    let eleven = [
        "MYR", "CNY", "INR", "RUB", "KRW", "JPY", "GBP", "USD", "TWD", "EUR", "AUD",
    ];
    let a = command(
        &room,
        &owner(),
        DISPLAY,
        MAIN,
        json!({ "tiers": [1, 2, 3, 4], "featured": eleven, "highlight": true, "allMarket": true }),
    )
    .await;
    a.refused(400, "validationFailed", "11 featured");
    assert_eq!(room.count("rate_display_settings").await, 0);
    room.finish().await;
}

/// PV-16: `rateDisplaySave` with `tiers: []` is `validationFailed` (`H:` §8: at least one of 1–4); nothing written.
#[tokio::test]
async fn rate_display_save_with_no_tiers_is_validation_failed() {
    let room = Room::start().await;
    let a = command(
        &room,
        &owner(),
        DISPLAY,
        MAIN,
        json!({ "tiers": [], "featured": ["USD"], "highlight": true, "allMarket": true }),
    )
    .await;
    a.refused(400, "validationFailed", "no tiers");
    assert_eq!(room.count("rate_display_settings").await, 0);
    room.finish().await;
}

/// PV-16: `rateDisplaySave` is an EXECUTIVE frame change (V.9's OWNER): a RATE_MANAGER is refused
/// `roleNotAllowedHere`; nothing written.
#[tokio::test]
async fn rate_display_save_by_a_caller_who_is_not_executive_is_refused() {
    let room = Room::start().await;
    let a = command(
        &room,
        &rate_manager(),
        DISPLAY,
        MAIN,
        json!({ "tiers": [1], "featured": ["USD"], "highlight": true, "allMarket": true }),
    )
    .await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(a.code(), json!(V3Error::from(err).code), "{}", a.text);
    assert_eq!(room.count("rate_display_settings").await, 0);
    room.finish().await;
}

// ------------------------------------------------------------ PV-17

/// PV-17: B has no display row and MAIN has one ⇒ B's projection `display` is MAIN's and B's `pricing_board`
/// `display` is MAIN's with `from: "MAIN"`; MAIN's own board says `from: "branch"`.
#[tokio::test]
async fn a_branch_with_no_display_row_reads_mains() {
    let room = Room::start().await;
    room.policy(MAIN).await;
    branch_b(&room).await;
    room.display_row(MAIN, &[1, 2], &["EUR", "USD"], false, false)
        .await;

    let display = projection(&room, BR_B).await["display"].clone();
    assert_eq!(display["tiers"], json!([1, 2]), "{display}");
    assert_eq!(display["featured"], json!(["EUR", "USD"]), "{display}");
    assert_eq!(display["highlight"], json!(false), "{display}");
    assert_eq!(display["more"], json!([]), "allMarket off: {display}");

    assert_eq!(
        board_display(&room, BR_B).await,
        json!({ "tiers": [1, 2], "featured": ["EUR", "USD"], "highlight": false, "allMarket": false, "from": "MAIN" })
    );
    assert_eq!(board_display(&room, MAIN).await["from"], json!("branch"));
    room.finish().await;
}

/// PV-17: `rateDisplaySave` at B writes B's own row (B reads it, `from: "branch"`); `{clear: true}` at B deletes it
/// and B reads MAIN's again (`from: "MAIN"`).
#[tokio::test]
async fn clear_at_a_branch_returns_it_to_mains_display() {
    let room = Room::start().await;
    room.policy(MAIN).await;
    branch_b(&room).await;
    room.display_row(MAIN, &[1, 2, 3, 4], &["EUR"], true, true)
        .await;

    let a = command(
        &room,
        &owner(),
        DISPLAY,
        BR_B,
        json!({ "tiers": [1], "featured": ["GBP"], "highlight": false, "allMarket": false }),
    )
    .await;
    assert_eq!(a.ok("save at B"), json!({ "saved": true }), "§15.8a");
    assert_eq!(
        board_display(&room, BR_B).await,
        json!({ "tiers": [1], "featured": ["GBP"], "highlight": false, "allMarket": false, "from": "branch" })
    );
    assert_eq!(
        projection(&room, BR_B).await["display"]["featured"],
        json!(["GBP"])
    );

    command(&room, &owner(), DISPLAY, BR_B, json!({ "clear": true }))
        .await
        .ok("clear at B");
    assert_eq!(
        room.count("rate_display_settings WHERE branch_id = 'BR-B'")
            .await,
        0,
        "B's row is deleted"
    );
    assert_eq!(
        board_display(&room, BR_B).await,
        json!({ "tiers": [1, 2, 3, 4], "featured": ["EUR"], "highlight": true, "allMarket": true, "from": "MAIN" })
    );
    assert_eq!(
        projection(&room, BR_B).await["display"]["featured"],
        json!(["EUR"])
    );
    room.finish().await;
}

/// PV-17: no display row anywhere ⇒ `H:` §8's defaults: tiers `[1,2,3,4]`, featured
/// `MYR, CNY, INR, RUB, KRW, JPY, GBP, USD, TWD, EUR`, highlight on, `allMarket` on, `from: "default"`.
#[tokio::test]
async fn with_no_display_row_anywhere_the_v9_defaults_apply() {
    let room = Room::start().await;
    room.policy(MAIN).await;
    branch_b(&room).await;

    assert_eq!(
        board_display(&room, BR_B).await,
        json!({ "tiers": [1, 2, 3, 4], "featured": DEFAULT_FEATURED, "highlight": true, "allMarket": true, "from": "default" })
    );
    let display = projection(&room, BR_B).await["display"].clone();
    assert_eq!(display["tiers"], json!([1, 2, 3, 4]), "{display}");
    assert_eq!(display["featured"], json!(DEFAULT_FEATURED), "{display}");
    assert_eq!(display["highlight"], json!(true), "{display}");
    room.finish().await;
}

// ------------------------------------------------------------ PV-18

/// B prices USD, EUR, GBP, CNY and AUD; its row features USD then EUR; a fresh Super Rich batch lists GBP, KHR and
/// USD. KHR has no price at B.
async fn pv18(room: &Room, all_market: bool) -> Value {
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    room.publish_manual(
        BR_B,
        &[
            ("USD", "1", "35.10", "35.50"),
            ("EUR", "1", "38.10", "38.70"),
            ("GBP", "1", "44.10", "44.90"),
            ("CNY", "1", "4.80", "5.00"),
            ("AUD", "1", "22.10", "22.90"),
        ],
    )
    .await;
    room.display_row(BR_B, &[1, 2, 3, 4], &["USD", "EUR"], true, all_market)
        .await;
    room.sr(
        whole_second_now() - minutes(2),
        &[
            ("GBP", "44.20", "44.80"),
            ("KHR", "0.0080", "0.0090"),
            ("USD", "35.20", "35.40"),
        ],
    )
    .await;
    projection(room, BR_B).await
}

/// PV-18: `allMarket` on ⇒ KHR, which SR lists and B has no price for, is in `display.more` ("contact the counter"),
/// with no line and no price; `order` is the featured USD, EUR, then GBP (SR lists it), then AUD, CNY (by code).
#[tokio::test]
async fn all_market_puts_a_super_rich_currency_with_no_price_in_more_and_orders_featured_then_sr_then_code()
 {
    let room = Room::start().await;
    let data = pv18(&room, true).await;
    assert_eq!(
        data["display"]["more"],
        json!(["KHR"]),
        "{}",
        data["display"]
    );
    assert!(
        lines_of(&data, "KHR").is_empty(),
        "KHR has no line and no price: {data}"
    );
    assert_eq!(
        data["display"]["order"],
        json!(["USD", "EUR", "GBP", "AUD", "CNY"]),
        "featured (set order) → SR → code"
    );
    room.finish().await;
}

/// PV-18: `allMarket` off ⇒ `more` is empty, whatever SR lists; the order is unchanged.
#[tokio::test]
async fn without_all_market_more_is_empty() {
    let room = Room::start().await;
    let data = pv18(&room, false).await;
    assert_eq!(data["display"]["more"], json!([]), "{}", data["display"]);
    assert_eq!(
        data["display"]["order"],
        json!(["USD", "EUR", "GBP", "AUD", "CNY"])
    );
    room.finish().await;
}
