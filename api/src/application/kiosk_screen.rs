//! The customer kiosk's owner-set screen values (owner 2026-11-05: "ว่า อันไหน คอนฟิก ได้ อย่า ฝังใหน code";
//! owner principle #5993, `docs/design/blueprint/user-set-settings.md`, `owner-set-values.md`): stored data,
//! never constants in `api/static/kiosk/kiosk.js`.
//!
//! Stored per branch in `branch_settings` under the name [`SETTING`] (a JSON body; no migration). The kiosk
//! reads them from its `rates` answer (`screen`) and, before the first answer, from the page (`/kiosk`
//! puts [`KioskScreen::not_set`] into `window.XC_KIOSK_SCREEN`).
//!
//! "Not set" (no row, or a field missing or out of range) answers the values below, each one marked. The
//! answer says `source: "default"` for a branch with no row and lists, in `defaults`, every field that is
//! answering its default, so a screen can show "ค่าเริ่มต้น / default" beside it.
//! owner-set: no save screen yet. Until one exists a value changes only by writing the branch's
//! `kioskScreen` row; the defaults are today's numbers, the ones the owner approved on the preview.

use serde_json::{Value, json};
use xc_kernel::ids::BranchId;

use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;

/// The `branch_settings.name` of these values.
pub const SETTING: &str = "kioskScreen";

/// The quick-add chips under the amount field (owner: "100 500 1000 5000"). owner-set: not-set default.
pub const DEFAULT_QUICK_AMOUNTS: [u32; 4] = [100, 500, 1000, 5000];
/// How many currency tiles show (their design's 3 x 2 grid). owner-set: not-set default.
pub const DEFAULT_TILES: u32 = 6;
/// Seconds without a touch before the screen resets (their kiosk's 2 minutes). owner-set: not-set default.
pub const DEFAULT_IDLE_SECONDS: u32 = 120;
/// Seconds the review screen holds Confirm (their kiosk's 3). owner-set: not-set default.
pub const DEFAULT_REVIEW_SECONDS: u32 = 3;
/// Seconds between rate refreshes (their kiosk's 10). owner-set: not-set default.
pub const DEFAULT_REFRESH_SECONDS: u32 = 10;
/// The longest refresh period the kiosk is given. `kiosk.js` greys the rate out ("not available", Review off) once
/// its last load is 30 s old, and it waits `refreshSeconds` only after a load ends, which can take up to its 10 s
/// fetch timeout. So the period plus one load must stay under 30 s: 19 + 10. A saved value above this is clamped.
pub const MAX_REFRESH_SECONDS: u32 = 19;
const _: () = assert!(
    MAX_REFRESH_SECONDS + 10 < 30,
    "one refresh period plus one 10 s load must stay inside kiosk.js's 30 s freshness"
);
/// Days of trading history that decide the popular currencies. owner-set: not-set default.
pub const DEFAULT_POPULAR_DAYS: u32 = 30;
/// The payment method the kiosk preselects (owner: "Payment method ให้ default"). owner-set: not-set
/// default. When the side does not allow it the kiosk takes Cash, then the side's first enabled method in
/// the answer's order.
pub const DEFAULT_PAYMENT_METHOD: &str = "Cash";

/// The field names, as the kiosk reads them.
const FIELDS: [&str; 7] = [
    "quickAmounts",
    "tiles",
    "idleSeconds",
    "reviewSeconds",
    "refreshSeconds",
    "popularDays",
    "defaultPaymentMethod",
];

/// One branch's values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskScreen {
    pub quick_amounts: Vec<u32>,
    pub tiles: u32,
    pub idle_seconds: u32,
    pub review_seconds: u32,
    pub refresh_seconds: u32,
    pub popular_days: u32,
    pub default_payment_method: String,
    /// Whether the branch has a saved row.
    pub set: bool,
    /// The fields answering their default.
    pub defaults: Vec<&'static str>,
}

fn whole(
    body: &Value,
    key: &'static str,
    range: std::ops::RangeInclusive<u32>,
    default: u32,
    defaults: &mut Vec<&'static str>,
) -> u32 {
    body.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .unwrap_or_else(|| {
            defaults.push(key);
            default
        })
}

impl KioskScreen {
    /// The values of a branch with no saved row.
    pub fn not_set() -> Self {
        Self {
            quick_amounts: DEFAULT_QUICK_AMOUNTS.to_vec(),
            tiles: DEFAULT_TILES,
            idle_seconds: DEFAULT_IDLE_SECONDS,
            review_seconds: DEFAULT_REVIEW_SECONDS,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
            popular_days: DEFAULT_POPULAR_DAYS,
            default_payment_method: DEFAULT_PAYMENT_METHOD.to_owned(),
            set: false,
            defaults: FIELDS.to_vec(),
        }
    }

    /// The values of a saved row; a field that is missing or out of range answers its default.
    pub fn from_body(body: &Value) -> Self {
        let mut defaults = Vec::new();
        let quick_amounts = body
            .get("quickAmounts")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_u64)
                    .filter_map(|n| u32::try_from(n).ok())
                    .filter(|n| (1..=10_000_000).contains(n))
                    .take(8)
                    .collect::<Vec<_>>()
            })
            .filter(|list| !list.is_empty())
            .unwrap_or_else(|| {
                defaults.push("quickAmounts");
                DEFAULT_QUICK_AMOUNTS.to_vec()
            });
        let d = &mut defaults;
        let tiles = whole(body, "tiles", 1..=60, DEFAULT_TILES, d);
        let idle_seconds = whole(body, "idleSeconds", 30..=3600, DEFAULT_IDLE_SECONDS, d);
        let review_seconds = whole(body, "reviewSeconds", 0..=30, DEFAULT_REVIEW_SECONDS, d);
        let refresh_seconds = whole(body, "refreshSeconds", 5..=300, DEFAULT_REFRESH_SECONDS, d)
            .min(MAX_REFRESH_SECONDS);
        let popular_days = whole(body, "popularDays", 1..=365, DEFAULT_POPULAR_DAYS, d);
        let default_payment_method = body
            .get("defaultPaymentMethod")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|m| !m.is_empty() && m.len() <= 40)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                d.push("defaultPaymentMethod");
                DEFAULT_PAYMENT_METHOD.to_owned()
            });
        Self {
            quick_amounts,
            tiles,
            idle_seconds,
            review_seconds,
            refresh_seconds,
            popular_days,
            default_payment_method,
            set: true,
            defaults,
        }
    }

    /// The branch's owner-set screen values; "not set" when it has none or they cannot be read.
    pub async fn read(pool: &sqlx::PgPool, branch: &BranchId) -> Self {
        match BranchSettingStore::new(pool.clone())
            .read(branch, SETTING)
            .await
        {
            Ok(Some(saved)) => Self::from_body(&saved.body),
            Ok(None) => Self::not_set(),
            Err(err) => {
                tracing::warn!(error = %err, branch = branch.as_str(), "kiosk screen values: read failed");
                Self::not_set()
            }
        }
    }

    /// What the kiosk reads (`screen` of the `rates` answer, `window.XC_KIOSK_SCREEN`).
    pub fn to_json(&self) -> Value {
        json!({
            "quickAmounts": self.quick_amounts,
            "tiles": self.tiles,
            "idleSeconds": self.idle_seconds,
            "reviewSeconds": self.review_seconds,
            "refreshSeconds": self.refresh_seconds,
            "popularDays": self.popular_days,
            "defaultPaymentMethod": self.default_payment_method,
            "set": self.set,
            "defaults": self.defaults,
            "source": if self.set { "owner" } else { "default" },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_set_answers_the_defaults_and_says_so() {
        let screen = KioskScreen::not_set().to_json();
        assert_eq!(screen["quickAmounts"], json!([100, 500, 1000, 5000]));
        assert_eq!(screen["tiles"], 6);
        assert_eq!(screen["idleSeconds"], 120);
        assert_eq!(screen["reviewSeconds"], 3);
        assert_eq!(screen["refreshSeconds"], 10);
        assert_eq!(screen["popularDays"], 30);
        assert_eq!(screen["set"], false);
        assert_eq!(screen["source"], "default");
        assert_eq!(screen["defaultPaymentMethod"], "Cash");
        assert_eq!(screen["defaults"].as_array().unwrap().len(), FIELDS.len());
    }

    #[test]
    fn a_saved_row_wins_and_a_bad_field_falls_back_alone() {
        let screen = KioskScreen::from_body(&json!({
            "quickAmounts": [200, 700],
            "tiles": 9,
            "idleSeconds": 5,
            "reviewSeconds": 4,
        }));
        assert_eq!(screen.quick_amounts, [200, 700]);
        assert_eq!(screen.tiles, 9);
        assert_eq!(
            screen.idle_seconds, DEFAULT_IDLE_SECONDS,
            "5 s is below the floor"
        );
        assert_eq!(screen.review_seconds, 4);
        assert_eq!(screen.refresh_seconds, DEFAULT_REFRESH_SECONDS);
        assert_eq!(screen.default_payment_method, "Cash");
        assert!(screen.set);
        assert_eq!(
            screen.defaults,
            [
                "idleSeconds",
                "refreshSeconds",
                "popularDays",
                "defaultPaymentMethod"
            ]
        );
        let owner = KioskScreen::from_body(&json!({ "defaultPaymentMethod": "PromptPay" }));
        assert_eq!(owner.default_payment_method, "PromptPay");
    }

    /// A refresh period the kiosk would outlive its 30 s rate freshness with is clamped to the longest safe one;
    /// a period within it is kept, and one below the floor still falls back to the default.
    #[test]
    fn a_long_refresh_period_is_clamped_below_the_kiosk_staleness() {
        for saved in [20, 30, 60, 300] {
            let screen = KioskScreen::from_body(&json!({ "refreshSeconds": saved }));
            assert_eq!(screen.refresh_seconds, MAX_REFRESH_SECONDS, "{saved} s");
            assert!(
                !screen.defaults.contains(&"refreshSeconds"),
                "{saved} s is the owner's value, clamped, not the default"
            );
        }
        let kept = KioskScreen::from_body(&json!({ "refreshSeconds": 15 }));
        assert_eq!(kept.refresh_seconds, 15);
        let edge = KioskScreen::from_body(&json!({ "refreshSeconds": MAX_REFRESH_SECONDS }));
        assert_eq!(edge.refresh_seconds, MAX_REFRESH_SECONDS);
        let low = KioskScreen::from_body(&json!({ "refreshSeconds": 4 }));
        assert_eq!(low.refresh_seconds, DEFAULT_REFRESH_SECONDS);
        assert!(low.defaults.contains(&"refreshSeconds"));
    }

    #[test]
    fn an_empty_chip_list_is_not_set() {
        let screen = KioskScreen::from_body(&json!({ "quickAmounts": [] }));
        assert_eq!(screen.quick_amounts, DEFAULT_QUICK_AMOUNTS);
    }
}
