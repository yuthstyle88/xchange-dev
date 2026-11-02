//! What a command answers when its posting refuses `stockInsufficient` (design `ledger-inventory.md` §5.1.2):
//! `409 stockInsufficient` naming each short place; no approval request is opened.

use actix_web::http::StatusCode;
use pos_error::AppError;
use xc_cash::domain::shortfall::{ShortPlace, StockShortfall};

use crate::presentation::http::v3::error::V3Error;

/// The refusal's message when `e` is the posting port's `stockInsufficient`.
pub fn stock_insufficient(e: &AppError) -> Option<&str> {
    match e {
        AppError::Domain(m) if m.starts_with("stockInsufficient") => Some(m.as_str()),
        _ => None,
    }
}

/// Answers `409 stockInsufficient` naming each short place (`ledger-inventory.md` §5.1.2).
pub fn stock_refusal_answer(refusal: &str) -> V3Error {
    let items: Vec<String> = StockShortfall::from_refusal(refusal)
        .lines
        .into_iter()
        .map(|l| {
            let place = match l.place {
                ShortPlace::Drawer => "ลิ้นชัก",
                ShortPlace::Safe => "ตู้เซฟ",
                ShortPlace::Position => "สต็อก",
            };
            format!(
                "{} {place} ขาด {}",
                l.currency.as_str(),
                l.short_by.normalize()
            )
        })
        .collect();
    V3Error::new(
        "stockInsufficient",
        None,
        format!("ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: {}", items.join(", ")),
        StatusCode::CONFLICT,
    )
}
