//! The `my_shift` view (CA-8 #624; design `cash-shift-transfer.md` §4.7, SD B6): the caller's own OPEN shift in the
//! branch (`shifts.teller_id` = the caller's `users.id`) with its balances, or none. Area row `myShift` (G2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash::{MyShift, MyShiftBalance};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct MyShiftView {
    pool: PgPool,
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
        let branch = branch.as_str().to_owned();
        let me = who.user_id.clone();
        async move {
            let shift: Option<(String, String, DateTime<Utc>)> = sqlx::query_as(
                "SELECT id, status, opened_at FROM shifts \
                 WHERE branch_id = $1 AND teller_id::text = $2 AND status = 'OPEN' \
                 ORDER BY opened_at DESC LIMIT 1",
            )
            .bind(&branch)
            .bind(&me)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;
            let Some((id, status, opened_at)) = shift else {
                return Ok(None);
            };
            let balances: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
                "SELECT code::text, opening, expected FROM shift_balances WHERE shift_id = $1 ORDER BY code",
            )
            .bind(&id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(Some(MyShift {
                id,
                status,
                opened_at: opened_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                balances: balances
                    .into_iter()
                    .map(|(code, opening, expected)| MyShiftBalance {
                        code: code.trim().to_owned(),
                        opening: opening.to_string(),
                        expected: expected.to_string(),
                    })
                    .collect(),
            }))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(MyShiftView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyShiftBalance::export_all(&cfg)?;
    MyShift::export_all(&cfg)?;
    Ok(())
}
