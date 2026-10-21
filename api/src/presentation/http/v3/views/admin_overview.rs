//! The `admin_overview` view (§7, §12.1, §12.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, round_cents};
use xc_shift::application::shift::service::ShiftService;

use crate::application::executive::channel::Channel;
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_overview::{
    AdminOverviewData, AdminOverviewFilter, AdminOverviewStatus, AdminOverviewTotals,
};
use crate::presentation::dto::v3::common::{OpenShiftDto, Period};
use crate::presentation::dto::v3::transaction_row::{
    ROW_COLUMNS, ROW_FROM, TransactionRow, TxSqlRow,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

#[derive(sqlx::FromRow)]
struct StatusCounts {
    completed_count: i64,
    pending_count: i64,
    cancelled_count: i64,
}

struct AdminOverviewBranchView {
    pool: PgPool,
    shifts: ShiftService,
}

impl AdminOverviewBranchView {
    fn new(pool: PgPool, shifts: ShiftService) -> Self {
        Self { pool, shifts }
    }
}

impl BranchView for AdminOverviewBranchView {
    const NAME: &'static str = "admin_overview";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = AdminOverviewData;
    type Filter = AdminOverviewFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let shifts = self.shifts.clone();
        let branch = branch.clone();
        let filter = filter.clone();

        async move {
            let open_shifts: Vec<OpenShiftDto> = shifts
                .open_of_branch(&branch)
                .await?
                .into_iter()
                .map(OpenShiftDto::from)
                .collect();
            let shift_open = !open_shifts.is_empty();

            let has_currency_or_channel = filter.currency.as_ref().is_some_and(|c| !c.is_empty())
                || filter
                    .channel
                    .as_ref()
                    .is_some_and(|ch| Channel::parse(ch).is_some());

            let banner_pending_count: u32 = if has_currency_or_channel {
                let mut banner_builder = sqlx::QueryBuilder::new(
                    "SELECT COALESCE(count(*) FILTER (WHERE t.status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED')), 0) \
                     FROM transactions t \
                     WHERE t.branch_id = ",
                );
                banner_builder.push_bind(branch.as_str());
                banner_builder.push(" AND t.mode = 'LIVE'");

                if let Some(ref from) = filter.from
                    && !from.is_empty()
                    && let Ok(d) = NaiveDate::parse_from_str(from, "%Y-%m-%d")
                {
                    banner_builder.push(" AND t.value_date >= ");
                    banner_builder.push_bind(d);
                }

                if let Some(ref to) = filter.to
                    && !to.is_empty()
                    && let Ok(d) = NaiveDate::parse_from_str(to, "%Y-%m-%d")
                {
                    banner_builder.push(" AND t.value_date <= ");
                    banner_builder.push_bind(d);
                }

                let count: (i64,) = banner_builder
                    .build_query_as()
                    .fetch_one(&pool)
                    .await
                    .map_err(AppError::Database)?;
                count.0 as u32
            } else {
                0
            };

            let mut count_builder = sqlx::QueryBuilder::new(
                "SELECT \
                 COALESCE(count(*) FILTER (WHERE t.status = 'COMPLETED'), 0) AS completed_count, \
                 COALESCE(count(*) FILTER (WHERE t.status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED')), 0) AS pending_count, \
                 COALESCE(count(*) FILTER (WHERE t.status = 'CANCELLED'), 0) AS cancelled_count \
                 FROM transactions t \
                 WHERE t.branch_id = ",
            );
            count_builder.push_bind(branch.as_str());
            count_builder.push(" AND t.mode = 'LIVE'");

            if let Some(ref from) = filter.from
                && !from.is_empty()
                && let Ok(d) = NaiveDate::parse_from_str(from, "%Y-%m-%d")
            {
                count_builder.push(" AND t.value_date >= ");
                count_builder.push_bind(d);
            }

            if let Some(ref to) = filter.to
                && !to.is_empty()
                && let Ok(d) = NaiveDate::parse_from_str(to, "%Y-%m-%d")
            {
                count_builder.push(" AND t.value_date <= ");
                count_builder.push_bind(d);
            }

            if let Some(ref currency) = filter.currency
                && !currency.is_empty()
            {
                count_builder.push(" AND EXISTS (SELECT 1 FROM transaction_lines l WHERE l.transaction_id = t.id AND l.line_no = 1 AND trim(l.currency::text) = ");
                count_builder.push_bind(currency);
                count_builder.push(")");
            }

            if let Some(ref channel) = filter.channel
                && let Some(ch) = Channel::parse(channel)
            {
                count_builder.push(" AND ");
                count_builder.push(ch.condition("t"));
            }

            let counts: StatusCounts = count_builder
                .build_query_as()
                .fetch_one(&pool)
                .await
                .map_err(AppError::Database)?;

            let status_pending_count = if has_currency_or_channel {
                banner_pending_count
            } else {
                counts.pending_count as u32
            };

            let mut builder = sqlx::QueryBuilder::new("SELECT ");
            builder.push(ROW_COLUMNS);
            builder.push(" FROM ");
            builder.push(ROW_FROM);
            builder.push(" WHERE t.branch_id = ");
            builder.push_bind(branch.as_str());
            builder.push(" AND t.mode = 'LIVE' AND t.status = 'COMPLETED'");

            if let Some(ref from) = filter.from
                && !from.is_empty()
                && let Ok(d) = NaiveDate::parse_from_str(from, "%Y-%m-%d")
            {
                builder.push(" AND t.value_date >= ");
                builder.push_bind(d);
            }

            if let Some(ref to) = filter.to
                && !to.is_empty()
                && let Ok(d) = NaiveDate::parse_from_str(to, "%Y-%m-%d")
            {
                builder.push(" AND t.value_date <= ");
                builder.push_bind(d);
            }

            if let Some(ref currency) = filter.currency
                && !currency.is_empty()
            {
                builder.push(" AND trim(l.currency::text) = ");
                builder.push_bind(currency);
            }

            if let Some(ref channel) = filter.channel
                && let Some(ch) = Channel::parse(channel)
            {
                builder.push(" AND ");
                builder.push(ch.condition("t"));
            }

            builder.push(" ORDER BY t.recorded_at DESC, t.seq DESC");

            let sql_rows: Vec<TxSqlRow> = builder
                .build_query_as()
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

            let mut volume_cents = 0i128;
            let mut buy_principal_cents = 0i128;
            let mut sell_principal_cents = 0i128;
            let mut received_cents = 0i128;
            let mut paid_cents = 0i128;
            let mut fees_cents = 0i128;
            let mut buy_count = 0u32;
            let mut sell_count = 0u32;

            let mut latest = Vec::with_capacity(std::cmp::min(5, sql_rows.len()));

            for sql_row in sql_rows {
                let row = TransactionRow::from_sql_row(sql_row.clone())?;
                let is_buy = sql_row.r#type == "BUY";
                let principal = sql_row
                    .principal
                    .parse::<Decimal>()
                    .map_err(|e| AppError::Internal(format!("invalid principal: {e}")))?;
                let principal_cents = round_cents(principal)
                    .ok_or_else(|| AppError::Internal("principal overflow".into()))?;
                let fee = sql_row
                    .fee
                    .parse::<Decimal>()
                    .map_err(|e| AppError::Internal(format!("invalid fee: {e}")))?;
                let fee_cents =
                    round_cents(fee).ok_or_else(|| AppError::Internal("fee overflow".into()))?;
                let settlement = sql_row
                    .settlement
                    .parse::<Decimal>()
                    .map_err(|e| AppError::Internal(format!("invalid settlement: {e}")))?;
                let settle_cents = round_cents(settlement)
                    .ok_or_else(|| AppError::Internal("settlement overflow".into()))?;

                volume_cents += principal_cents;
                fees_cents += fee_cents;

                if is_buy {
                    buy_principal_cents += principal_cents;
                    buy_count += 1;
                    paid_cents += settle_cents;
                } else {
                    sell_principal_cents += principal_cents;
                    sell_count += 1;
                    received_cents += settle_cents;
                }

                if latest.len() < 5 {
                    latest.push(row);
                }
            }

            let net_cents = received_cents - paid_cents;

            let (buy_share, sell_share) = if volume_cents == 0 {
                ("0".to_string(), "0".to_string())
            } else {
                let vol = Decimal::from_i128_with_scale(volume_cents, 2);
                let hundred = Decimal::from(100);
                let buy = Decimal::from_i128_with_scale(buy_principal_cents, 2);
                let sell = Decimal::from_i128_with_scale(sell_principal_cents, 2);

                let buy_share_cents = round_cents((buy * hundred) / vol)
                    .ok_or_else(|| AppError::Internal("buy share overflow".into()))?;
                let sell_share_cents = round_cents((sell * hundred) / vol)
                    .ok_or_else(|| AppError::Internal("sell share overflow".into()))?;

                (fixed2(buy_share_cents), fixed2(sell_share_cents))
            };

            let totals = AdminOverviewTotals {
                volume: fixed2(volume_cents),
                buy_principal: fixed2(buy_principal_cents),
                sell_principal: fixed2(sell_principal_cents),
                received: fixed2(received_cents),
                paid: fixed2(paid_cents),
                net: fixed2(net_cents),
                fees: fixed2(fees_cents),
                buy_share,
                sell_share,
                buy_count,
                sell_count,
                completed_count: counts.completed_count as u32,
                cancelled_count: counts.cancelled_count as u32,
                pending_count: counts.pending_count as u32,
            };

            let period = Period {
                from: filter.from.unwrap_or_default(),
                to: filter.to.unwrap_or_default(),
            };

            let status = AdminOverviewStatus {
                shift_open,
                pending_count: status_pending_count,
                open_shifts,
            };

            Ok(AdminOverviewData {
                period,
                totals,
                latest,
                status,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(pos_infra::transactions::Transactor::new(state.db.clone()));
    let shifts = ShiftService::new(
        Arc::new(
            xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                state.db.clone(),
            ),
        ),
        branch_tx,
    );
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminOverviewBranchView::new(state.db.clone(), shifts)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminOverviewData::export_all(&cfg)?;
    AdminOverviewFilter::export_all(&cfg)?;
    Ok(())
}
