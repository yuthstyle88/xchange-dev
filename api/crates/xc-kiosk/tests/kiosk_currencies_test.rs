//! Ticket #4031 (A3a) — `KioskCurrencies` (which currencies the kiosk offers) without a database.

use xc_kernel::money::CurrencyCode;
use xc_kiosk::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use xc_kiosk::domain::kiosk::value_objects::kiosk_currencies::{DEFAULT_CODES, KioskCurrencies};

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).expect("valid currency code")
}

/// Row 1: `[]` -> offers each of `DEFAULT_CODES`.
#[test]
fn row1_empty_switches_offers_default_currencies() {
    let kc = KioskCurrencies::new(&[]);
    for default_code in DEFAULT_CODES {
        assert!(
            kc.offers(code(default_code)),
            "default currency {default_code} should be offered"
        );
    }
    assert!(
        !kc.offers(code("SGD")),
        "non-default currency SGD should not be offered by default"
    );
}

/// Row 2: `[USD off]` -> `!offers(USD)`, EUR still offered.
#[test]
fn row2_switch_off_disables_default_currency() {
    let switches = [CurrencySwitch {
        code: code("USD"),
        enabled: false,
    }];
    let kc = KioskCurrencies::new(&switches);

    assert!(
        !kc.offers(code("USD")),
        "USD should not be offered when switched off"
    );
    assert!(kc.offers(code("EUR")), "EUR should still be offered");
}

/// Row 3: `[SGD on]` -> offers SGD.
#[test]
fn row3_switch_on_adds_non_default_currency() {
    let switches = [CurrencySwitch {
        code: code("SGD"),
        enabled: true,
    }];
    let kc = KioskCurrencies::new(&switches);

    assert!(
        kc.offers(code("SGD")),
        "SGD should be offered when switched on"
    );
    assert!(
        kc.offers(code("USD")),
        "default USD should still be offered"
    );
}

/// Row 4: `[USD off, USD on]` -> later switch wins (on / off).
#[test]
fn row4_later_switch_wins() {
    let off_then_on = [
        CurrencySwitch {
            code: code("USD"),
            enabled: false,
        },
        CurrencySwitch {
            code: code("USD"),
            enabled: true,
        },
    ];
    let kc_on = KioskCurrencies::new(&off_then_on);
    assert!(
        kc_on.offers(code("USD")),
        "later switch (enabled: true) should win over earlier switch"
    );

    let on_then_off = [
        CurrencySwitch {
            code: code("USD"),
            enabled: true,
        },
        CurrencySwitch {
            code: code("USD"),
            enabled: false,
        },
    ];
    let kc_off = KioskCurrencies::new(&on_then_off);
    assert!(
        !kc_off.offers(code("USD")),
        "later switch (enabled: false) should win over earlier switch"
    );
}

/// Row 5 (ticket): `[USD on, USD on]` -> `all()` lists USD once (no duplicates).
#[test]
fn row5_duplicate_switches_all_lists_usd_once() {
    let switches = [
        CurrencySwitch {
            code: code("USD"),
            enabled: true,
        },
        CurrencySwitch {
            code: code("USD"),
            enabled: true,
        },
    ];
    let kc = KioskCurrencies::new(&switches);

    let all = kc.all();
    let usd_count = all.iter().filter(|s| s.code == code("USD")).count();
    assert_eq!(usd_count, 1, "USD should appear exactly once in all()");

    let mut codes: Vec<CurrencyCode> = all.iter().map(|s| s.code).collect();
    let len_before = codes.len();
    codes.dedup();
    assert_eq!(
        codes.len(),
        len_before,
        "all() must not contain duplicate currency codes"
    );
}

/// Row 6: `all()` of a mixed list -> no THB, sorted by code, correct enabled status.
#[test]
fn row6_all_excludes_thb_and_is_sorted_by_code() {
    let switches = [
        CurrencySwitch {
            code: code("SGD"),
            enabled: true,
        },
        CurrencySwitch {
            code: code("USD"),
            enabled: false,
        },
    ];
    let kc = KioskCurrencies::new(&switches);

    let all = kc.all();

    // 1. Excludes THB
    assert!(
        all.iter().all(|s| !s.code.is_thb()),
        "all() must not include THB"
    );
    assert!(
        !all.iter().any(|s| s.code.as_str() == "THB"),
        "all() must not contain currency code THB"
    );

    // 2. Sorted by code
    let codes: Vec<&str> = all.iter().map(|s| s.code.as_str()).collect();
    let mut sorted_codes = codes.clone();
    sorted_codes.sort_unstable();
    assert_eq!(codes, sorted_codes, "all() must be sorted by currency code");

    // 3. Status matches offers()
    for s in &all {
        assert_eq!(
            s.enabled,
            kc.offers(s.code),
            "switch enabled status for {} must match kc.offers()",
            s.code
        );
    }
}
