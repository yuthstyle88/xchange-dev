//! `PgLoyaltyFactsSource` (design §6, D-2, D-4): reads facts from Postgres for Loyalty events.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_customer::application::customer::SystemReadPurpose;
use xc_customer::application::customer::service::CustomerService;
use xc_kernel::Mode;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xc_loyalty::domain::facts::{CompletedFacts, CompletedLine, ReversedFacts};
use xc_loyalty::domain::facts_source::LoyaltyFactsSource;
use xc_loyalty::infrastructure::exchange_handler::LoyaltyPhoneReader;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn end_of_business_day(business_date: NaiveDate, tz: chrono_tz::Tz) -> Option<DateTime<Utc>> {
    business_date
        .checked_add_days(Days::new(1))
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|d| tz.from_local_datetime(&d).earliest())
        .map(|d| d.with_timezone(&Utc))
}

#[derive(Clone)]
pub struct PgLoyaltyFactsSource {
    pool: PgPool,
}

impl PgLoyaltyFactsSource {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl LoyaltyFactsSource for PgLoyaltyFactsSource {
    fn completed(&self, tx_id: Uuid) -> BoxFuture<'_, AppResult<CompletedFacts>> {
        Box::pin(async move {
            let tx_row = sqlx::query(
                "SELECT t.organization_id, t.document_ref, t.customer_id, t.mode, t.branch_id, \
                        t.completed_at, t.value_date, b.timezone \
                 FROM transactions t \
                 LEFT JOIN branches b ON b.id = t.branch_id \
                 WHERE t.id = $1",
            )
            .bind(tx_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let tx_row = match tx_row {
                Some(r) => r,
                None => {
                    return Err(AppError::NotFound {
                        entity: "transaction",
                        id: tx_id.to_string(),
                    });
                }
            };

            let organization_id: Uuid = tx_row
                .try_get("organization_id")
                .map_err(AppError::Database)?;
            let document_ref: String =
                tx_row.try_get("document_ref").map_err(AppError::Database)?;
            let customer_id: Option<String> =
                tx_row.try_get("customer_id").map_err(AppError::Database)?;
            let mode_str: String = tx_row.try_get("mode").map_err(AppError::Database)?;
            let branch_id: String = tx_row.try_get("branch_id").map_err(AppError::Database)?;
            let completed_at = tx_row.try_get("completed_at").map_err(AppError::Database)?;
            let business_date: NaiveDate =
                tx_row.try_get("value_date").map_err(AppError::Database)?;
            let timezone: Option<String> =
                tx_row.try_get("timezone").map_err(AppError::Database)?;

            let mode = match mode_str.as_str() {
                "LIVE" => Mode::Live,
                _ => Mode::Demo,
            };

            let Some(timezone) = timezone else {
                return Err(AppError::Internal(format!(
                    "branch {branch_id} does not exist"
                )));
            };
            let tz: chrono_tz::Tz = timezone
                .parse()
                .map_err(|_| AppError::Internal(format!("branch timezone {timezone}")))?;
            let reversible_until = end_of_business_day(business_date, tz)
                .ok_or_else(|| AppError::Internal(format!("the end of {business_date}")))?;

            let line_rows = sqlx::query(
                "SELECT currency, shop_side, quantity, unit_size, thb_amount, settle_thb, realized_margin_thb \
                 FROM transaction_lines WHERE transaction_id = $1 \
                 ORDER BY line_no ASC, created_at ASC",
            )
            .bind(tx_id)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let mut lines = Vec::with_capacity(line_rows.len());
            for lr in line_rows {
                let currency: String = lr.try_get("currency").map_err(AppError::Database)?;
                let shop_side_str: String = lr.try_get("shop_side").map_err(AppError::Database)?;
                let quantity: Decimal = lr.try_get("quantity").map_err(AppError::Database)?;
                let unit_size: Decimal = lr.try_get("unit_size").map_err(AppError::Database)?;
                let thb_amount: Decimal = lr.try_get("thb_amount").map_err(AppError::Database)?;
                let settle_thb: Decimal = lr.try_get("settle_thb").map_err(AppError::Database)?;
                let realized_margin_thb: Option<Decimal> = lr
                    .try_get("realized_margin_thb")
                    .map_err(AppError::Database)?;

                let shop_side = match shop_side_str.as_str() {
                    "BUY" => Side::Buy,
                    "SELL" => Side::Sell,
                    other => return Err(AppError::Internal(format!("unknown shop_side: {other}"))),
                };

                lines.push(CompletedLine {
                    currency,
                    shop_side,
                    quantity,
                    unit_size,
                    thb_amount,
                    settle_thb,
                    realized_margin_thb,
                });
            }

            Ok(CompletedFacts {
                organization_id,
                transaction_id: tx_id,
                document_ref,
                customer_id,
                mode,
                branch_id,
                completed_at,
                business_date,
                reversible_until,
                lines,
            })
        })
    }

    fn reversed(&self, rev_id: Uuid) -> BoxFuture<'_, AppResult<ReversedFacts>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT r.organization_id, r.original_transaction_id, r.posted_at, r.amount_thb, t.customer_id \
                 FROM reversals r \
                 JOIN transactions t ON t.id = r.original_transaction_id \
                 WHERE r.id = $1",
            )
            .bind(rev_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let row = match row {
                Some(r) => r,
                None => {
                    return Err(AppError::NotFound {
                        entity: "reversal",
                        id: rev_id.to_string(),
                    });
                }
            };

            let organization_id: Uuid =
                row.try_get("organization_id").map_err(AppError::Database)?;
            let transaction_id: Uuid = row
                .try_get("original_transaction_id")
                .map_err(AppError::Database)?;
            let posted_at = row.try_get("posted_at").map_err(AppError::Database)?;
            let customer_id: Option<String> =
                row.try_get("customer_id").map_err(AppError::Database)?;

            let cumulative_sum: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount_thb), 0) FROM reversals \
                 WHERE original_transaction_id = $1 AND status = 'POSTED'",
            )
            .bind(transaction_id)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;

            Ok(ReversedFacts {
                organization_id,
                reversal_id: rev_id,
                transaction_id,
                customer_id,
                posted_at,
                amount_thb: cumulative_sum,
                full: true,
            })
        })
    }
}

impl LoyaltyPhoneReader for PgLoyaltyFactsSource {
    fn verified_phone<'a>(
        &'a self,
        organization_id: Uuid,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<String>>> {
        Box::pin(async move {
            let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
            let customer_service = CustomerService::new(branch_tx);
            customer_service
                .verified_phone_for_system(
                    organization_id,
                    customer_id,
                    SystemReadPurpose::Loyalty,
                    Utc::now(),
                )
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_of_business_day_uses_branch_timezone_and_earliest_instant() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        assert_eq!(
            end_of_business_day(day, chrono_tz::Asia::Bangkok),
            Some(Utc.with_ymd_and_hms(2026, 10, 7, 17, 0, 0).unwrap())
        );
        assert_eq!(
            end_of_business_day(day, chrono_tz::Asia::Tokyo),
            Some(Utc.with_ymd_and_hms(2026, 10, 7, 15, 0, 0).unwrap())
        );
        assert_eq!(
            end_of_business_day(day, chrono_tz::Europe::London),
            Some(Utc.with_ymd_and_hms(2026, 10, 7, 23, 0, 0).unwrap())
        );

        // DST transition at midnight (America/Havana): fall-back fold on 2026-11-01 00:00 returns earliest (04:00Z),
        // and spring-forward gap on 2026-03-08 00:00 returns None without panicking (same as branch_day_in).
        let havana_fold_eve = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(
            end_of_business_day(havana_fold_eve, chrono_tz::America::Havana),
            Some(Utc.with_ymd_and_hms(2026, 11, 1, 4, 0, 0).unwrap())
        );
        let havana_gap_eve = NaiveDate::from_ymd_opt(2026, 3, 7).unwrap();
        assert_eq!(
            end_of_business_day(havana_gap_eve, chrono_tz::America::Havana),
            None
        );
    }
}
