//! XC-PRICING PX-13 (#4605): out-of-range margins must not become 0 silently in `PricingSuggestion::of`.

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_flag::PricingFlag;
use xc_rate::domain::pricing::pricing_suggestion::{
    PricingSuggestion, SuggestionConfig, SuggestionInput,
};

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("rate")
}

#[test]
fn out_of_range_margin_sets_margins_none_and_flags_precision_once() {
    let input = SuggestionInput {
        currency: CurrencyCode::parse("USD").expect("currency"),
        quotes: vec![MarketQuote {
            source: MarketSource::Tt,
            buy: Some(rate("33.2")),
            sell: Some(rate("33.9")),
            unit_size: Decimal::ONE,
            fetched_at: Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap(),
            stale: false,
        }],
        tt_decimals: Some(2),
        unit_size: Decimal::ONE,
        base_buy: Some(rate("0.0001")),
        base_sell: Some(rate("34")),
        buy_adjust_pct: RatePercent::ZERO,
        sell_adjust_pct: RatePercent::ZERO,
        partner_cost: None,
        published_tier1_buy: None,
        published_tier1_sell: None,
        config: SuggestionConfig::v9_default(),
    };

    let s = PricingSuggestion::of(&input);
    assert_eq!(s.margins, None);
    assert!(s.tiers.is_some());
    // A base is given, so the anchor exists: an unstorable margin is Precision, not NoAnchor.
    assert!(!s.flags.contains(&PricingFlag::NoAnchor));
    assert_eq!(
        s.flags
            .iter()
            .filter(|&&f| f == PricingFlag::Precision)
            .count(),
        1
    );
}
