use std::time::Duration;
use uuid::Uuid;
use xc_loyalty::infrastructure::config::LoyaltyConfig;

const VALID_BASE: &str = "http://loyalty.test";
const VALID_TENANT: &str = "11111111-2222-4333-8444-555555555555";
const VALID_SECRET: &str = "secret-key";

#[test]
fn row_1_invalid_uuid_tenant_returns_none() {
    let cfg = LoyaltyConfig::from_raw(
        Some(VALID_BASE),
        Some("not-a-uuid"),
        Some(VALID_SECRET),
        None,
        None,
    );
    assert_eq!(cfg, None);
}

#[test]
fn row_2_blank_tenant_returns_none() {
    let cfg = LoyaltyConfig::from_raw(
        Some(VALID_BASE),
        Some("   "),
        Some(VALID_SECRET),
        None,
        None,
    );
    assert_eq!(cfg, None);
}

#[test]
fn row_3_missing_blank_or_invalid_timeout_defaults_to_5000_ms() {
    for raw_timeout in [None, Some(""), Some("abc"), Some("-1")] {
        let cfg = LoyaltyConfig::from_raw(
            Some(VALID_BASE),
            Some(VALID_TENANT),
            Some(VALID_SECRET),
            raw_timeout,
            None,
        )
        .unwrap_or_else(|| panic!("expected Some for timeout {raw_timeout:?}"));
        assert_eq!(
            cfg.timeout,
            Duration::from_millis(5000),
            "timeout for {raw_timeout:?}"
        );
    }
}

#[test]
fn row_4_trimmed_timeout_is_parsed() {
    let cfg = LoyaltyConfig::from_raw(
        Some(VALID_BASE),
        Some(VALID_TENANT),
        Some(VALID_SECRET),
        Some(" 1500 "),
        None,
    )
    .expect("expected Some for padded timeout");
    assert_eq!(cfg.timeout, Duration::from_millis(1500));
}

#[test]
fn row_5_base_url_and_webhook_secret_are_trimmed() {
    let cfg = LoyaltyConfig::from_raw(
        Some(" http://x "),
        Some(VALID_TENANT),
        Some(" s "),
        None,
        None,
    )
    .expect("expected Some for padded base_url and secret");
    assert_eq!(cfg.base_url, "http://x");
    assert_eq!(cfg.webhook_secret, "s");
}

#[test]
fn row_6_padded_valid_tenant_is_trimmed_and_parsed() {
    let padded = format!("  {VALID_TENANT}  ");
    let cfg = LoyaltyConfig::from_raw(
        Some(VALID_BASE),
        Some(&padded),
        Some(VALID_SECRET),
        None,
        None,
    )
    .expect("expected Some for padded valid tenant UUID");
    assert_eq!(
        cfg.tenant_id,
        Uuid::parse_str(VALID_TENANT).expect("valid UUID")
    );
}
