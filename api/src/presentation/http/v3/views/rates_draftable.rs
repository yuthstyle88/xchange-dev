//! The `rates_draftable` view (`docs/design/board-currencies.md` D-8).
//! It inherits the reference route, as `rates_reference` does.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rates_draftable::{DraftableCurrency, RatesDraftableData};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct RatesDraftableView {
    pool: PgPool,
}

impl BranchView for RatesDraftableView {
    const NAME: &'static str = "rates_draftable";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/reference-rates");
    type Data = RatesDraftableData;
    type Filter = NoFilter;

    #[allow(clippy::manual_async_fn)]
    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        async move {
            let rows: Vec<(String, String, Decimal, bool)> = sqlx::query_as(
                "WITH newest_bot AS ( \
                     SELECT id FROM rate_reference_batches \
                     WHERE provider = 'BOT' \
                     ORDER BY COALESCE(source_published_at, fetched_at) DESC, fetched_at DESC, id DESC \
                     LIMIT 1 \
                 ) \
                 SELECT trim(bc.currency_code) AS code, bc.group_code, c.unit_size, \
                        EXISTS ( \
                            SELECT 1 FROM rate_reference_lines l \
                            WHERE l.batch_id = (SELECT id FROM newest_bot) \
                              AND l.currency_code = bc.currency_code \
                        ) AS has_reference \
                 FROM branch_currencies bc \
                 JOIN currencies c ON c.code = bc.currency_code \
                 WHERE bc.branch_id = $1 \
                   AND bc.display IN ('SHOW', 'ASK') \
                   AND trim(bc.currency_code) <> 'THB' \
                 ORDER BY bc.sort_order, bc.currency_code, bc.group_code",
            )
            .bind(&branch)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            Ok(RatesDraftableData {
                rows: rows
                    .into_iter()
                    .map(|(code, group, unit_size, has_reference)| DraftableCurrency {
                        code,
                        group,
                        unit_size: unit_size.normalize().to_string(),
                        has_reference,
                    })
                    .collect(),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(RatesDraftableView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DraftableCurrency::export_all(&cfg)?;
    RatesDraftableData::export_all(&cfg)?;
    Ok(())
}
