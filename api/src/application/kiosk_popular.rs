//! Popular currencies for the kiosk's currency tiles (owner 2026-11-05: "ให้ ตั้ง ว่า อันไหน นิยม ซื้อขาย
//! ดูจาก ประวัติ การ ซื้อ ขายแล้วให้ เลือก มาแสดง").
//!
//! The rule, as the kiosk `rates` answer's `popular` list applies it:
//! - a currency's popularity at a branch is the number of COMPLETED, LIVE transactions at that branch
//!   with a line in that currency, on either side (BUY and SELL), whose `value_date` falls in the
//!   last `popularDays` days (the branch's owner-set value, `application::kiosk_screen`; 30 when not
//!   set), today (Bangkok) included;
//! - more transactions first; equal counts by more THB (Σ `transaction_lines.thb_amount`), then in
//!   the order the kiosk already shows its currencies;
//! - only currencies the kiosk offers right now are listed (`rates`), so one that is switched off
//!   is never listed, however much it was traded; a currency with no such transaction is not listed;
//! - the counts are read once per branch per 10 minutes ([`PopularCache`]), so the kiosk's 10 s
//!   rates poll does not count every time. A failed read lists nothing and is not kept.
//!
//! No table and no migration: one query over `transactions` and `transaction_lines`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Days, NaiveDate};
use pos_error::AppResult;
use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long a branch's counts are kept.
pub const POPULAR_TTL: Duration = Duration::from_secs(600);

/// One currency's use at a branch over the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrencyUse {
    pub code: String,
    /// COMPLETED, LIVE transactions with a line in this currency.
    pub completed: i64,
    /// Σ `thb_amount` of those lines.
    pub thb: Decimal,
}

/// Reads the counts; `PgCurrencyUses` in production.
pub trait CurrencyUseSource: Send + Sync {
    /// Every currency used at `branch` with `value_date >= since`.
    fn uses<'a>(
        &'a self,
        branch: &'a BranchId,
        since: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Vec<CurrencyUse>>>;
}

/// The first day of the `days`-day window that ends on `today`.
pub fn window_start(today: NaiveDate, days: u32) -> NaiveDate {
    today
        .checked_sub_days(Days::new(u64::from(days.max(1)) - 1))
        .unwrap_or(today)
}

/// The popular codes among `offered` (the kiosk's codes, in the order it shows them).
pub fn popular_order(offered: &[&str], uses: &[CurrencyUse]) -> Vec<String> {
    let mut ranked: Vec<&CurrencyUse> = offered
        .iter()
        .filter_map(|code| uses.iter().find(|u| u.code == *code && u.completed > 0))
        .collect();
    // stable: equal count and THB keep the offered order
    ranked.sort_by(|a, b| b.completed.cmp(&a.completed).then(b.thb.cmp(&a.thb)));
    ranked.into_iter().map(|u| u.code.clone()).collect()
}

/// Each branch's counts, kept for [`POPULAR_TTL`].
pub struct PopularCache {
    source: Arc<dyn CurrencyUseSource>,
    ttl: Duration,
    kept: Mutex<HashMap<String, (Instant, Vec<CurrencyUse>)>>,
}

impl PopularCache {
    pub fn new(source: Arc<dyn CurrencyUseSource>) -> Self {
        Self::with_ttl(source, POPULAR_TTL)
    }

    pub fn with_ttl(source: Arc<dyn CurrencyUseSource>, ttl: Duration) -> Self {
        Self {
            source,
            ttl,
            kept: Mutex::new(HashMap::new()),
        }
    }

    /// The branch's counts since `since` ([`window_start`]) as of `now`, read again only when the kept
    /// ones are older than the TTL.
    pub async fn uses_at(
        &self,
        branch: &BranchId,
        since: NaiveDate,
        now: Instant,
    ) -> Vec<CurrencyUse> {
        if let Some(kept) = self.kept_for(branch, now) {
            return kept;
        }
        match self.source.uses(branch, since).await {
            Ok(uses) => {
                if let Ok(mut kept) = self.kept.lock() {
                    kept.insert(branch.as_str().to_owned(), (now, uses.clone()));
                }
                uses
            }
            Err(err) => {
                tracing::warn!(error = %err, branch = branch.as_str(), "kiosk popular currencies: read failed");
                Vec::new()
            }
        }
    }

    fn kept_for(&self, branch: &BranchId, now: Instant) -> Option<Vec<CurrencyUse>> {
        let kept = self.kept.lock().ok()?;
        let (at, uses) = kept.get(branch.as_str())?;
        (now.saturating_duration_since(*at) < self.ttl).then(|| uses.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn use_of(code: &str, completed: i64, thb: &str) -> CurrencyUse {
        CurrencyUse {
            code: code.to_owned(),
            completed,
            thb: Decimal::from_str(thb).unwrap(),
        }
    }

    #[test]
    fn more_transactions_first_then_more_thb_then_the_offered_order() {
        let uses = [
            use_of("JPY", 2, "900"),
            use_of("USD", 5, "100"),
            use_of("EUR", 3, "50"),
            use_of("GBP", 3, "80"),
            use_of("SGD", 2, "900"),
        ];
        let offered = ["EUR", "GBP", "JPY", "SGD", "USD", "CNY"];
        assert_eq!(
            popular_order(&offered, &uses),
            ["USD", "GBP", "EUR", "JPY", "SGD"]
        );
    }

    #[test]
    fn a_currency_the_kiosk_does_not_offer_is_never_listed() {
        let uses = [use_of("GBP", 99, "1000000"), use_of("USD", 1, "1")];
        assert_eq!(popular_order(&["USD", "EUR"], &uses), ["USD"]);
    }

    #[test]
    fn no_history_lists_nothing() {
        assert!(popular_order(&["USD", "EUR"], &[]).is_empty());
    }

    #[test]
    fn the_window_is_30_days_today_included() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        assert_eq!(
            window_start(today, 30),
            NaiveDate::from_ymd_opt(2026, 10, 7).unwrap()
        );
    }

    struct Counting(AtomicUsize);

    impl CurrencyUseSource for Counting {
        fn uses<'a>(
            &'a self,
            _branch: &'a BranchId,
            _since: NaiveDate,
        ) -> BoxFuture<'a, AppResult<Vec<CurrencyUse>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(vec![use_of("USD", 1, "1")]) })
        }
    }

    #[actix_web::test]
    async fn one_read_per_branch_per_ten_minutes() {
        let source = Arc::new(Counting(AtomicUsize::new(0)));
        let cache = PopularCache::new(source.clone());
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let main = BranchId::new("MAIN");
        let t0 = Instant::now();

        // the kiosk polls every 10 s: one read for the whole 10 minutes
        for poll in 0..60 {
            let at = t0 + Duration::from_secs(10 * poll);
            assert_eq!(cache.uses_at(&main, today, at).await.len(), 1);
        }
        assert_eq!(source.0.load(Ordering::SeqCst), 1);

        // another branch is read on its own
        cache.uses_at(&BranchId::new("B2"), today, t0).await;
        assert_eq!(source.0.load(Ordering::SeqCst), 2);

        // ten minutes later MAIN is read again
        cache
            .uses_at(&main, today, t0 + POPULAR_TTL + Duration::from_secs(1))
            .await;
        assert_eq!(source.0.load(Ordering::SeqCst), 3);
    }

    struct Failing(AtomicUsize);

    impl CurrencyUseSource for Failing {
        fn uses<'a>(
            &'a self,
            _branch: &'a BranchId,
            _since: NaiveDate,
        ) -> BoxFuture<'a, AppResult<Vec<CurrencyUse>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(pos_error::AppError::Internal("down".into())) })
        }
    }

    #[actix_web::test]
    async fn a_failed_read_lists_nothing_and_is_not_kept() {
        let source = Arc::new(Failing(AtomicUsize::new(0)));
        let cache = PopularCache::new(source.clone());
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let t0 = Instant::now();
        assert!(
            cache
                .uses_at(&BranchId::new("MAIN"), today, t0)
                .await
                .is_empty()
        );
        assert!(
            cache
                .uses_at(&BranchId::new("MAIN"), today, t0)
                .await
                .is_empty()
        );
        assert_eq!(source.0.load(Ordering::SeqCst), 2);
    }
}
