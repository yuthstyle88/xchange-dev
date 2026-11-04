//! What the Bank of Thailand's table does not say (`docs/design/board-currencies.md` §5).

/// Whether BoT's table is whole: a table or period missing one of these is cut off. Not an offered list: what a
/// branch offers is `branch_currencies`.
pub const BOT_TABLE_SENTINELS: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

/// How many units BoT's printed rate is for. The table has no unit field. JPY and IDR name theirs; VND, LAK and
/// KHR do not and are inferred from the ratio to SR and TT (§5).
pub fn bot_unit(code: &str) -> u64 {
    match code {
        "IDR" => 1000,
        "JPY" | "VND" | "LAK" | "KHR" => 100,
        _ => 1,
    }
}
