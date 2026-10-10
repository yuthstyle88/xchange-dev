//! APP-2 (#558): `CustomerSide`, design `customer-app.md` P4. Written before the implementation by the
//! ticket author (claude/5edf7bb5). Do not edit, delete or weaken; add tests in a file of your own.

use xc_kernel::money::{CustomerSide, Side};

#[test]
fn a_customer_who_buys_is_a_shop_that_sells() {
    assert_eq!(CustomerSide::CustomerBuy.to_shop(), Side::Sell);
}

#[test]
fn a_customer_who_sells_is_a_shop_that_buys() {
    assert_eq!(CustomerSide::CustomerSell.to_shop(), Side::Buy);
}

#[test]
fn the_wire_values_round_trip_exactly() {
    for side in [CustomerSide::CustomerBuy, CustomerSide::CustomerSell] {
        assert_eq!(CustomerSide::parse(side.as_str()), Some(side));
    }
    assert_eq!(CustomerSide::CustomerBuy.as_str(), "CUSTOMER_BUY");
    assert_eq!(CustomerSide::CustomerSell.as_str(), "CUSTOMER_SELL");
}

#[test]
fn a_shop_side_or_another_spelling_is_not_a_customer_side() {
    for text in [
        "BUY",
        "SELL",
        "customer_buy",
        "CUSTOMER_BUY ",
        "",
        "CUSTOMER-SELL",
    ] {
        assert_eq!(CustomerSide::parse(text), None, "{text:?} must not parse");
    }
}
