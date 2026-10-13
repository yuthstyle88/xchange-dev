//! PgQuoteRepository: PostgreSQL implementation of QuoteRepository.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Rate, Side, UnitSize};

use crate::domain::quote::quote::{Quote, QuoteChannel, QuoteStatus};
use crate::domain::quote::quote_line::QuoteLine;
use crate::domain::quote::repository::QuoteRepository;
use crate::infrastructure::parse_stored_money;

pub struct PgQuoteRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgQuoteRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl QuoteRepository for PgQuoteRepository {
    async fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote: &'a Quote,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, \
             status, publication_id, expires_at, benefit_snapshot, created_by, accepted_by_transaction_id, \
             created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(quote.id)
        .bind(quote.organization_id)
        .bind(quote.branch_id.as_str())
        .bind(&quote.customer_id)
        .bind(&quote.quote_type)
        .bind(quote.channel.as_str())
        .bind(quote.status.as_str())
        .bind(quote.publication_id)
        .bind(quote.expires_at)
        .bind(&quote.benefit_snapshot)
        .bind(&quote.created_by)
        .bind(quote.accepted_by_transaction_id)
        .bind(quote.created_at)
        .bind(quote.updated_at)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        for line in &quote.lines {
            let shop_side_str = match line.shop_side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };
            let thb_amount = Decimal::new(line.thb_amount.minor(), 2);
            let fee_thb = Decimal::new(line.fee_thb.minor(), 2);
            let settle_thb = Decimal::new(line.settle_thb.minor(), 2);

            sqlx::query(
                "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, \
                 shop_side, currency, denomination_group, quantity, unit_size, base_rate, final_rate, \
                 thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            )
            .bind(line.id)
            .bind(line.organization_id)
            .bind(line.quote_id)
            .bind(line.published_rate_line_id)
            .bind(shop_side_str)
            .bind(line.currency.as_str())
            .bind(&line.denomination_group)
            .bind(line.quantity.value())
            .bind(line.unit_size.value())
            .bind(line.base_rate.value())
            .bind(line.final_rate.value())
            .bind(thb_amount)
            .bind(fee_thb)
            .bind(settle_thb)
            .bind(line.line_no)
            .bind(line.created_at)
            .bind(line.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;
        }

        Ok(())
    }

    async fn find_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> AppResult<Option<Quote>> {
        let q_row = sqlx::query(
            "SELECT id, organization_id, branch_id, customer_id, type, channel, status, \
             publication_id, expires_at, benefit_snapshot, created_by, accepted_by_transaction_id, \
             created_at, updated_at \
             FROM quotes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let q_row = match q_row {
            Some(r) => r,
            None => return Ok(None),
        };

        let q_id: Uuid = q_row.get(0);
        let organization_id: Uuid = q_row.get(1);
        let branch_id_str: String = q_row.get(2);
        let customer_id: Option<String> = q_row.get(3);
        let quote_type: String = q_row.get(4);
        let channel_str: String = q_row.get(5);
        let status_str: String = q_row.get(6);
        let publication_id: Uuid = q_row.get(7);
        let expires_at: DateTime<Utc> = q_row.get(8);
        let benefit_snapshot: serde_json::Value = q_row.get(9);
        let created_by: String = q_row.get(10);
        let accepted_by_transaction_id: Option<Uuid> = q_row.get(11);
        let created_at: DateTime<Utc> = q_row.get(12);
        let updated_at: DateTime<Utc> = q_row.get(13);

        let channel = QuoteChannel::parse_db(&channel_str).ok_or_else(|| {
            pos_error::AppError::Domain(format!("invalid channel: {channel_str}"))
        })?;
        let status = QuoteStatus::parse_db(&status_str)
            .ok_or_else(|| pos_error::AppError::Domain(format!("invalid status: {status_str}")))?;

        let line_rows = sqlx::query(
            "SELECT id, organization_id, quote_id, published_rate_line_id, shop_side, currency, \
             denomination_group, quantity, unit_size, base_rate, final_rate, thb_amount, fee_thb, \
             settle_thb, line_no, created_at, updated_at \
             FROM quote_lines WHERE quote_id = $1 ORDER BY line_no",
        )
        .bind(id)
        .fetch_all(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let mut lines = Vec::new();
        for r in line_rows {
            let l_id: Uuid = r.get(0);
            let l_org: Uuid = r.get(1);
            let l_quote_id: Uuid = r.get(2);
            let published_rate_line_id: Uuid = r.get(3);
            let shop_side_str: String = r.get(4);
            let currency_str: String = r.get(5);
            let denomination_group: String = r.get(6);
            let quantity_dec: Decimal = r.get(7);
            let unit_size_dec: Decimal = r.get(8);
            let base_rate_dec: Decimal = r.get(9);
            let final_rate_dec: Decimal = r.get(10);
            let thb_amount_dec: Decimal = r.get(11);
            let fee_thb_dec: Decimal = r.get(12);
            let settle_thb_dec: Decimal = r.get(13);
            let line_no: i32 = r.get(14);
            let l_created: DateTime<Utc> = r.get(15);
            let l_updated: DateTime<Utc> = r.get(16);

            let shop_side = match shop_side_str.as_str() {
                "BUY" => Side::Buy,
                "SELL" => Side::Sell,
                _ => return Err(pos_error::AppError::Domain("invalid shop_side".into())),
            };
            let currency = CurrencyCode::parse(currency_str.trim())
                .map_err(|e| pos_error::AppError::Domain(e.to_string()))?;
            let unit_size = UnitSize::from_decimal(unit_size_dec)
                .map_err(|e| pos_error::AppError::Domain(e.to_string()))?;
            let base_rate = Rate::parse(&base_rate_dec.to_string())
                .map_err(|e| pos_error::AppError::Domain(e.to_string()))?;
            let final_rate = Rate::parse(&final_rate_dec.to_string())
                .map_err(|e| pos_error::AppError::Domain(e.to_string()))?;
            let thb_amount = parse_stored_money(&thb_amount_dec.to_string())?;
            let fee_thb = parse_stored_money(&fee_thb_dec.to_string())?;
            let settle_thb = parse_stored_money(&settle_thb_dec.to_string())?;

            lines.push(QuoteLine {
                id: l_id,
                organization_id: l_org,
                quote_id: l_quote_id,
                published_rate_line_id,
                shop_side,
                currency,
                denomination_group,
                quantity: ForeignAmount::from_decimal(quantity_dec),
                unit_size,
                base_rate,
                final_rate,
                thb_amount,
                fee_thb,
                settle_thb,
                line_no,
                created_at: l_created,
                updated_at: l_updated,
            });
        }

        Ok(Some(Quote {
            id: q_id,
            organization_id,
            branch_id: BranchId::new(branch_id_str),
            customer_id,
            quote_type,
            channel,
            status,
            publication_id,
            expires_at,
            benefit_snapshot,
            created_by,
            accepted_by_transaction_id,
            created_at,
            updated_at,
            lines,
            stale: false,
        }))
    }

    async fn accept_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
        transaction_id: Uuid,
    ) -> AppResult<()> {
        let done = sqlx::query(
            "UPDATE quotes SET status = 'ACCEPTED', accepted_by_transaction_id = $2, \
             updated_at = now() WHERE id = $1 AND status = 'ACTIVE'",
        )
        .bind(id)
        .bind(transaction_id)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;
        if done.rows_affected() == 0 {
            return Err(pos_error::AppError::Domain(format!(
                "staleVersion: quote {id} is not ACTIVE"
            )));
        }
        Ok(())
    }

    async fn cancel_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> AppResult<()> {
        let done = sqlx::query(
            "UPDATE quotes SET status = 'CANCELLED', accepted_by_transaction_id = NULL, \
             updated_at = now() WHERE id = $1 AND status IN ('ACTIVE', 'ACCEPTED')",
        )
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;
        if done.rows_affected() == 0 {
            return Err(pos_error::AppError::Domain(format!(
                "staleVersion: quote {id} is neither ACTIVE nor ACCEPTED"
            )));
        }
        Ok(())
    }
}
