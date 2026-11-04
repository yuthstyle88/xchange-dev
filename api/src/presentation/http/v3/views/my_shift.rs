//! The `my_shift` view (CA-8 #624; design `cash-shift-transfer.md` §4.7, SD B6): the caller's own OPEN shift in the
//! branch (`shifts.teller_id` = the caller's `users.id`) with its balances, or none. Area row `myShift` (G2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::infrastructure::drawer_low_levels::DrawerLowLevels;
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash::{MyShift, MyShiftBalance};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::cash_transfer_rows::{
    TRANSFER_COLUMNS, TransferSqlRow, with_lines,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

/// `id, status, opened_at, template_name, business_date` of the caller's open shift.
type MyShiftRow = (String, String, DateTime<Utc>, Option<String>, NaiveDate);

struct MyShiftView {
    pool: PgPool,
    shift_balances: Arc<dyn ShiftBalances>,
}

impl BranchView for MyShiftView {
    const NAME: &'static str = "my_shift";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "myShift");
    type Data = Option<MyShift>;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let shift_balances = self.shift_balances.clone();
        let branch_id = branch.clone();
        let branch = branch.as_str().to_owned();
        let me = who.user_id.clone();
        async move {
            let shift: Option<MyShiftRow> = sqlx::query_as(
                "SELECT id, status, opened_at, template_name, business_date FROM shifts \
                 WHERE branch_id = $1 AND teller_id::text = $2 AND status = 'OPEN' \
                 ORDER BY opened_at DESC LIMIT 1",
            )
            .bind(&branch)
            .bind(&me)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;
            let Some((id, status, opened_at, shift_name, business_date)) = shift else {
                return Ok(None);
            };
            let balances = shift_balances.shift_balances(&id).await?;
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            let low_levels = DrawerLowLevels::for_branch_in(&mut tx, &branch_id).await?;
            tx.commit().await.map_err(AppError::Database)?;
            let transfer_rows: Vec<TransferSqlRow> = sqlx::query_as(AssertSqlSafe(format!(
                "SELECT {TRANSFER_COLUMNS} \
                 FROM cash_transfers t \
                 WHERE t.to_shift_id = $1 \
                 ORDER BY t.created_at, t.id"
            )))
            .bind(&id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            let incoming_transfers = with_lines(&pool, transfer_rows).await?;
            Ok(Some(MyShift {
                id,
                status,
                opened_at: opened_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                balances: balances
                    .into_iter()
                    .map(|b| {
                        let level = low_levels
                            .iter()
                            .find(|(c, _)| *c == b.currency)
                            .map(|(_, l)| *l);
                        MyShiftBalance {
                            code: b.currency.as_str().to_owned(),
                            opening: b.opening.to_string(),
                            expected: b.expected.to_string(),
                            low_cash_level: level.map(|l| l.normalize().to_string()),
                            low: level.is_some_and(|l| b.expected.value() <= l),
                        }
                    })
                    .collect(),
                incoming_transfers,
                shift_name,
                business_date: business_date.to_string(),
            }))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(MyShiftView {
            pool: state.db.clone(),
            shift_balances: state.shift_balances.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyShiftBalance::export(&cfg)?;
    MyShift::export(&cfg)?;
    Ok(())
}
