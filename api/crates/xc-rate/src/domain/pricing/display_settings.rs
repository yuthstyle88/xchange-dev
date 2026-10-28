//! What the storefront shows (pricing-tiers.md §15.8; V9:store `rate_display_settings`, V9:display `DEFAULT_DISPLAY`).
//!
//! SKELETON (XC-V9-PRICING): `DisplaySettings::{validate, order}` are PV-a3's; the default and the wire strings are
//! the contract and are written here. Types and signatures stay.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use crate::domain::pricing::tier_set::StoreFuture;

/// One branch's display settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySettings {
    /// Which tiers the board shows, 1..=4 each, 1 to 4 of them, no repeats.
    pub tiers: Vec<u8>,
    /// Currency codes shown first, in this order; at most 10.
    pub featured: Vec<String>,
    pub highlight: bool,
    /// List currencies Super Rich quotes that the branch has no price for, as "contact the counter" (§15.8 `more`).
    pub all_market: bool,
}

/// Where the effective settings come from (§15.8a `display.from`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayFrom {
    Branch,
    Main,
    Default,
}

impl DisplayFrom {
    /// `"branch"` · `"MAIN"` · `"default"`.
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayFrom::Branch => "branch",
            DisplayFrom::Main => "MAIN",
            DisplayFrom::Default => "default",
        }
    }
}

/// The board's currency order and the `more` list (§15.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayOrder {
    /// The priced currencies: featured first in `featured` order, then Super Rich's market order, then by code.
    pub ordered: Vec<String>,
    /// With `all_market`: currencies in `sr_order` the branch has no price for, in `sr_order`'s order; else empty.
    pub more: Vec<String>,
}

impl DisplaySettings {
    /// The most `featured` codes (`H:` §8).
    pub const MAX_FEATURED: usize = 10;

    /// V.9's `DEFAULT_DISPLAY`: tiers `[1,2,3,4]`; featured `MYR, CNY, INR, RUB, KRW, JPY, GBP, USD, TWD, EUR`;
    /// highlight on; `all_market` on.
    pub fn v9_default() -> Self {
        Self {
            tiers: vec![1, 2, 3, 4],
            featured: [
                "MYR", "CNY", "INR", "RUB", "KRW", "JPY", "GBP", "USD", "TWD", "EUR",
            ]
            .iter()
            .map(|c| c.to_string())
            .collect(),
            highlight: true,
            all_market: true,
        }
    }

    /// `rateDisplaySave`'s checks: 1..=4 tiers, each 1..=4, no repeats; at most 10 featured, each a 3-letter
    /// upper-case code, no repeats. `Err` carries the reason for `validationFailed`.
    pub fn validate(
        tiers: &[u8],
        featured: &[String],
        highlight: bool,
        all_market: bool,
    ) -> Result<Self, String> {
        if !(1..=4).contains(&tiers.len()) {
            return Err("tiers must have 1 to 4 entries".to_string());
        }
        for (i, &t) in tiers.iter().enumerate() {
            if !(1..=4).contains(&t) {
                return Err(format!("invalid tier {t}: must be 1..=4"));
            }
            if tiers[..i].contains(&t) {
                return Err(format!("duplicate tier {t}"));
            }
        }
        if featured.len() > Self::MAX_FEATURED {
            return Err(format!("at most {} featured codes", Self::MAX_FEATURED));
        }
        for (i, code) in featured.iter().enumerate() {
            if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
                return Err(format!(
                    "invalid featured code {code}: must be 3 uppercase letters"
                ));
            }
            if featured[..i].contains(code) {
                return Err(format!("duplicate featured code {code}"));
            }
        }
        Ok(Self {
            tiers: tiers.to_vec(),
            featured: featured.to_vec(),
            highlight,
            all_market,
        })
    }

    /// `priced` is every currency the branch has a published price for; `sr_order` is Super Rich's market order.
    pub fn order(&self, priced: &[String], sr_order: &[String]) -> DisplayOrder {
        let mut ordered = priced.to_vec();
        ordered.sort_by_key(|c| {
            let feat_pos = self.featured.iter().position(|f| f == c);
            let sr_pos = sr_order.iter().position(|s| s == c);
            (
                feat_pos.is_none(),
                feat_pos,
                sr_pos.is_none(),
                sr_pos,
                c.clone(),
            )
        });
        let more = if self.all_market {
            sr_order
                .iter()
                .filter(|c| !priced.contains(c))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        DisplayOrder { ordered, more }
    }
}

/// `rate_display_settings` (§15.2a). The Pg impl is `infrastructure::pricing::display_settings_store`.
pub trait DisplaySettingsStore: Send + Sync {
    /// The branch's row, else MAIN's (branch id `"MAIN"`), else `DisplaySettings::v9_default()`, and which.
    fn effective_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
    ) -> StoreFuture<'a, (DisplaySettings, DisplayFrom)>;

    /// Upserts the branch's row.
    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        settings: &'a DisplaySettings,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, ()>;

    /// Deletes the branch's row (back to MAIN's, then the default).
    fn clear_in<'a>(&'a self, conn: &'a mut PgConnection, branch: &'a str) -> StoreFuture<'a, ()>;

    /// `max(updated_at)` over every row, for `pricing_board`'s version (§15.9).
    fn newest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<DateTime<Utc>>>;
}
