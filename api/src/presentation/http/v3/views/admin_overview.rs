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
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, round_cents};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_overview::{
    AdminOverviewData, AdminOverviewFilter, AdminOverviewStatus, AdminOverviewTotals,
};
use crate::presentation::dto::v3::common::Period;
use crate::presentation::dto::v3::transaction_row::{
    TransactionRecord, TransactionRow, TxSqlRow,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

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
            let active = shifts.active(&branch).await?;
            let (shift_open, opened_by) = match active {
                Some(shift) => (true, Some(shift.opened_by)),
                None => (false, None),
            };

            let mut builder = sqlx::QueryBuilder::new(
                "SELECT t.document_ref, t.value_date, t.recorded_at, t.type, t.code::text AS code, \
                 t.amount::text AS amount, t.rate::text AS rate, t.reference_rate::text AS reference_rate, \
                 t.fee::text AS fee, t.bank_fee::text AS bank_fee, t.method, t.payment_ref, \
                 t.customer_id, t.customer_name, t.staff_id, t.notes, t.shift_id, \
                 (SELECT file_key FROM transaction_files f WHERE f.transaction_id = t.id AND f.kind = 'SLIP' LIMIT 1) AS slip_key \
                 FROM transactions t \
                 WHERE t.branch_id = ",
            );
            builder.push_bind(branch.as_str());
            builder.push(" AND t.mode = 'LIVE'");

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
            let mut completed_count = 0u32;

            let mut latest = Vec::with_capacity(std::cmp::min(5, sql_rows.len()));

            for sql_row in sql_rows {
                let record = TransactionRecord::try_from(sql_row)?;
                let is_buy = record.side == "BUY";
                let raw_principal = record.amount * record.rate;
                let principal_cents = round_cents(raw_principal)
                    .ok_or_else(|| AppError::Internal("principal overflow".into()))?;
                let fee_cents = round_cents(record.fee)
                    .ok_or_else(|| AppError::Internal("fee overflow".into()))?;

                volume_cents += principal_cents;
                fees_cents += fee_cents;
                completed_count += 1;

                if is_buy {
                    buy_principal_cents += principal_cents;
                    buy_count += 1;
                    paid_cents += principal_cents - fee_cents;
                } else {
                    sell_principal_cents += principal_cents;
                    sell_count += 1;
                    received_cents += principal_cents + fee_cents;
                }

                if latest.len() < 5 {
                    latest.push(TransactionRow::from_record(&record));
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
                completed_count,
                cancelled_count: 0,
                pending_count: 0,
            };

            let period = Period {
                from: filter.from.unwrap_or_default(),
                to: filter.to.unwrap_or_default(),
            };

            let status = AdminOverviewStatus {
                shift_open,
                pending_count: 0,
                opened_by,
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
        Arc::new(PgShiftRepository::new(state.db.clone())),
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
