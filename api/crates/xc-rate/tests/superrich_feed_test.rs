//! XC-PRICING PX-3 (pricing-tiers.md §4): `SuperRichFeed::parse`, ported from V.9 `test/pricing.mjs:41-46,52-56`
//! (`parseSuperRich`). Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not
//! edit, delete or weaken; add tests in a file of your own. Pure: no network.

use rust_decimal::Decimal;
use serde_json::json;
use xc_rate::domain::pricing::market_feed::{MarketBody, MarketFeed};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::infrastructure::market::superrich_feed::{SR_URL, SuperRichFeed};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

/// V.9 `test/pricing.mjs:41-46`: the page's flight data, with the list split across two chunks as on the live page.
fn page() -> String {
    let obj = json!({"x": 1, "date": "2026-10-18", "exchangeList": {
        "USD": [{"denomRem": "100", "buyText": "33.33", "sellText": "33.40", "branchCode": "H01"},
                {"denomRem": "5-1", "buyText": "32.10", "sellText": "33.40"}],
        "GBP": [{"denomRem": "50", "buyText": "44.40", "sellText": "44.55"}],
        "LKR": [{"denomRem": "5000", "buyText": "0", "sellText": "0"}]}})
    .to_string();
    let flight = &obj[1..obj.len() - 1];
    let half = flight.len() / 2;
    let first = format!("5:[{{{}", &flight[..half]);
    let second = format!("{}}}]", &flight[half..]);
    format!(
        "<html><script>self.__next_f.push([1,{}])</script><script>self.__next_f.push([1,{}])</script></html>",
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    )
}

fn body(html: &str) -> Vec<MarketBody> {
    vec![MarketBody {
        url: SR_URL.to_string(),
        body: Some(html.to_string()),
    }]
}

#[test]
fn live_feed_asks_for_the_exchange_rate_page() {
    let feed = SuperRichFeed::live();
    assert_eq!(feed.source(), MarketSource::SuperRich);
    let req = feed.requests();
    assert_eq!(req.len(), 1);
    assert_eq!(
        req[0].url,
        "https://www.superrichthailand.com/exchange-rate"
    );
    assert!(!req[0].optional);
}

#[test]
fn reads_the_list_across_two_flight_chunks() {
    let snap = SuperRichFeed::live().parse(&body(&page())).expect("parsed");
    assert_eq!(snap.source, MarketSource::SuperRich);
    assert_eq!(snap.source_updated_at.as_deref(), Some("2026-10-18"));
    assert_eq!(
        snap.rows.len(),
        3,
        "USD ×2 and GBP; LKR has zero prices and is dropped"
    );
    let usd0 = snap
        .rows
        .iter()
        .find(|r| r.code == "USD" && r.band == "0")
        .unwrap();
    assert_eq!(
        (usd0.buy, usd0.sell, usd0.denoms.as_str()),
        (Some(d("33.33")), Some(d("33.40")), "100")
    );
    let usd1 = snap
        .rows
        .iter()
        .find(|r| r.code == "USD" && r.band == "1")
        .unwrap();
    assert_eq!(usd1.buy, Some(d("32.10")));
    assert!(snap.rows.iter().all(|r| r.decimals.is_none()));
    assert!(!snap.rows.iter().any(|r| r.code == "LKR"));
}

#[test]
fn a_page_without_the_list_is_an_error() {
    let err = SuperRichFeed::live()
        .parse(&body("<html>maintenance</html>"))
        .unwrap_err();
    assert!(
        err.to_string().contains("SR: exchangeList not found"),
        "{err}"
    );
}

#[test]
fn a_list_cut_off_mid_object_is_an_error() {
    let cut = format!(
        "<html><script>self.__next_f.push([1,{}])</script></html>",
        serde_json::to_string(r#"5:[{"exchangeList":{"USD":[{"buyText":"33.3""#).unwrap()
    );
    let err = SuperRichFeed::live().parse(&body(&cut)).unwrap_err();
    assert!(
        err.to_string().contains("SR: exchangeList is cut off"),
        "{err}"
    );
}

#[test]
fn a_list_with_only_zero_prices_has_no_rates() {
    let only_zero = format!(
        "<html><script>self.__next_f.push([1,{}])</script></html>",
        serde_json::to_string(
            r#"5:[{"exchangeList":{"LKR":[{"denomRem":"5000","buyText":"0","sellText":"0"}]}}]"#
        )
        .unwrap()
    );
    let err = SuperRichFeed::live().parse(&body(&only_zero)).unwrap_err();
    assert!(err.to_string().contains("SR: no rates"), "{err}");
}
