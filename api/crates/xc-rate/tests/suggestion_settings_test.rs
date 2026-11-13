//! XC-PRICING PX-10 (pricing-tiers.md §5.2, §12.1): `SuggestionSettings` and `PricingSuggestionSettingsStore`,
//! after V.9 `src/pricing.mjs:51` (`{...DEFAULT_CONFIG, ...config['*'], ...config[code]}`) and `:204-206`
//! (`configure`'s checks). Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0).
//! Do not edit, delete or weaken; add tests in a file of your own.

mod rate_engine_common;

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use rate_engine_common::TestDb;
use rust_decimal::Decimal;
use xc_rate::domain::pricing::suggestion_settings::{BetterFrom, SuggestionSettings};
use xc_rate::infrastructure::pricing::suggestion_settings_store::PricingSuggestionSettingsStore;

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn four(v: [&str; 4]) -> [Decimal; 4] {
    v.map(d)
}

#[test]
fn currency_then_branch_then_v9s_default() {
    let s = SuggestionSettings {
        branch_default: Some(four(["0.05", "0.1", "0.1", "0.2"])),
        by_currency: HashMap::from([("USD".to_string(), four(["0", "0.1", "0.2", "0.3"]))]),
    };
    assert_eq!(
        s.better_for("USD"),
        (four(["0", "0.1", "0.2", "0.3"]), BetterFrom::Currency)
    );
    assert_eq!(
        s.better_for("EUR"),
        (four(["0.05", "0.1", "0.1", "0.2"]), BetterFrom::Branch)
    );
    assert_eq!(
        SuggestionSettings::default().better_for("EUR"),
        (four(["0.1", "0.14", "0.2", "0.3"]), BetterFrom::Default),
        "V.9 engine.mjs:13"
    );
}

#[test]
fn v9s_configure_checks() {
    assert_eq!(
        SuggestionSettings::validate(&four(["0", "0.1", "0.1", "0.2"])).unwrap(),
        four(["0", "0.1", "0.1", "0.2"])
    );
    assert_eq!(
        SuggestionSettings::validate(&four(["-2", "-1", "0", "2"])).unwrap()[0],
        d("-2"),
        "the bounds are inclusive"
    );
    assert!(
        SuggestionSettings::validate(&[d("0.1"), d("0.2"), d("0.3")]).is_err(),
        "exactly four"
    );
    assert!(
        SuggestionSettings::validate(&four(["0", "0.1", "0.2", "2.01"])).is_err(),
        "above 2"
    );
    assert!(
        SuggestionSettings::validate(&four(["-2.5", "0", "0", "0"])).is_err(),
        "below −2"
    );
    assert!(
        SuggestionSettings::validate(&four(["0.2", "0.1", "0.3", "0.4"])).is_err(),
        "a higher tier must not be worse"
    );
    let scaled = SuggestionSettings::validate(&four(["0.1", "0.14", "0.2", "0.3"])).unwrap();
    assert_eq!(scaled[1].scale(), 4, "stored at scale 4");
}

#[test]
fn d1_better_for_with_and_min_spread_for_with_return_none_when_not_set() {
    let s = SuggestionSettings {
        branch_default: Some(four(["0.05", "0.1", "0.1", "0.2"])),
        by_currency: HashMap::from([("USD".to_string(), four(["0", "0.1", "0.2", "0.3"]))]),
    };
    assert_eq!(
        s.better_for_with("USD"),
        Some((four(["0", "0.1", "0.2", "0.3"]), BetterFrom::Currency))
    );
    assert_eq!(
        s.better_for_with("EUR"),
        Some((four(["0.05", "0.1", "0.1", "0.2"]), BetterFrom::Branch))
    );
    assert_eq!(SuggestionSettings::default().better_for_with("EUR"), None);
    assert_eq!(SuggestionSettings::min_spread_for_with(None, None), None);
    assert_eq!(
        SuggestionSettings::min_spread_for_with(Some(d("0.5")), Some(d("0.4"))),
        Some(d("0.5"))
    );
    assert_eq!(
        SuggestionSettings::min_spread_for_with(None, Some(d("0.4"))),
        Some(d("0.4"))
    );
}

#[test]
fn g12_and_g13_validate_with_and_validate_min_spread_with_use_owner_ranges_and_refuse_none() {
    use xc_rate::domain::policy::pricing_rules::PctRange;

    let err =
        SuggestionSettings::validate_with(&four(["0.1", "0.2", "0.3", "0.4"]), None).unwrap_err();
    assert_eq!(err.code(), "pricingRuleNotSet");
    assert_eq!(err.to_string(), "betterPctRange");

    let r = Some(PctRange {
        min: d("-1"),
        max: d("1"),
    });
    assert_eq!(
        SuggestionSettings::validate_with(&four(["-1", "0", "0.5", "1"]), r).unwrap(),
        four(["-1.0000", "0.0000", "0.5000", "1.0000"])
    );
    assert_eq!(
        SuggestionSettings::validate_with(&four(["0", "0.1", "0.2", "1.01"]), r)
            .unwrap_err()
            .code(),
        "validationFailed"
    );

    assert_eq!(
        SuggestionSettings::validate_min_spread_with(None, None).unwrap(),
        None,
        "clearing min_spread_pct is allowed when minSpreadPctRange is not set"
    );
    let err_ms = SuggestionSettings::validate_min_spread_with(Some(d("0.3")), None).unwrap_err();
    assert_eq!(err_ms.code(), "pricingRuleNotSet");
    assert_eq!(err_ms.to_string(), "minSpreadPctRange");

    let ms_r = Some(PctRange {
        min: d("0.1"),
        max: d("2.5"),
    });
    assert_eq!(
        SuggestionSettings::validate_min_spread_with(Some(d("2.5")), ms_r).unwrap(),
        Some(d("2.5"))
    );
    assert_eq!(
        SuggestionSettings::validate_min_spread_with(Some(d("2.6")), ms_r)
            .unwrap_err()
            .code(),
        "validationFailed"
    );
    assert_eq!(
        SuggestionSettings::validate(&four(["0", "0.1", "0.2", "2.01"])).unwrap_err(),
        "ส่วนต่างแต่ละระดับต้องอยู่ระหว่าง -2% ถึง 2%"
    );
}

#[tokio::test]
async fn the_store_upserts_reads_and_deletes_rows() {
    let db = TestDb::start().await;
    let at = Utc.with_ymd_and_hms(2026, 10, 22, 1, 0, 0).unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let branch: String = sqlx::query_scalar("SELECT id FROM branches ORDER BY id LIMIT 1")
        .fetch_one(&mut *conn)
        .await
        .expect("a seeded branch");
    assert_eq!(
        PricingSuggestionSettingsStore::for_branch(&mut conn, &branch)
            .await
            .unwrap(),
        SuggestionSettings::default()
    );

    PricingSuggestionSettingsStore::save(
        &mut conn,
        &branch,
        "*",
        Some(four(["0.05", "0.1", "0.1", "0.2"])),
        "owner-1",
        at,
    )
    .await
    .unwrap();
    PricingSuggestionSettingsStore::save(
        &mut conn,
        &branch,
        "USD",
        Some(four(["0", "0.1", "0.2", "0.3"])),
        "owner-1",
        at,
    )
    .await
    .unwrap();
    PricingSuggestionSettingsStore::save(
        &mut conn,
        &branch,
        "USD",
        Some(four(["0", "0", "0", "0"])),
        "owner-1",
        at,
    )
    .await
    .unwrap();
    let s = PricingSuggestionSettingsStore::for_branch(&mut conn, &branch)
        .await
        .unwrap();
    assert_eq!(s.branch_default, Some(four(["0.05", "0.1", "0.1", "0.2"])));
    assert_eq!(
        s.by_currency.get("USD"),
        Some(&four(["0", "0", "0", "0"])),
        "the second save replaced the first"
    );

    PricingSuggestionSettingsStore::save(&mut conn, &branch, "USD", None, "owner-1", at)
        .await
        .unwrap();
    let s = PricingSuggestionSettingsStore::for_branch(&mut conn, &branch)
        .await
        .unwrap();
    assert!(s.by_currency.is_empty(), "None deletes the row");
    assert!(s.branch_default.is_some());
}
