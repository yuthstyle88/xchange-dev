//! MK-7 (#1406): `GET /api/v3/views/campaign_performance` (M04, design `marketing-campaigns.md` D-15, §6.1): per
//! campaign version over the period, in the branch the core resolved, through `PerformanceReader`. Attributed
//! contribution is unavailable (`awaitsLedger`, else `awaitsBroker` while Broker is not connected), never zero.
//! Gate: `MarketingPerformance` (policy row `campaignPerformance`).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_marketing::domain::performance::commission::{BrokerNotConnected, CampaignCommission};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::campaign_performance::{
    CampaignPerformanceData, CampaignPerformanceFilter, PerformanceFigure, VersionPerformanceRow,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};
use actix_web::http::StatusCode;

impl ViewFilter for CampaignPerformanceFilter {
    fn check(&self) -> Result<(), V3Error> {
        let Ok(from) = chrono::NaiveDate::parse_from_str(&self.from, "%Y-%m-%d") else {
            return Err(V3Error::new(
                "invalidRequest",
                Some("from"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        };
        let Ok(to) = chrono::NaiveDate::parse_from_str(&self.to, "%Y-%m-%d") else {
            return Err(V3Error::new(
                "invalidRequest",
                Some("to"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        };
        if from > to {
            return Err(V3Error::new(
                "periodOrder",
                Some("to"),
                "วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

pub struct CampaignPerformanceView {
    pool: PgPool,
    commission: Arc<dyn CampaignCommission>,
}

impl CampaignPerformanceView {
    pub fn new(pool: PgPool, commission: Arc<dyn CampaignCommission>) -> Self {
        Self { pool, commission }
    }
}

impl BranchView for CampaignPerformanceView {
    const NAME: &'static str = "campaign_performance";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignPerformance");
    type Data = CampaignPerformanceData;
    type Filter = CampaignPerformanceFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let commission = self.commission.clone();
        let branch = branch.clone();
        let _who = who.clone();
        let filter = filter.clone();

        async move {
            let from = chrono::NaiveDate::parse_from_str(&filter.from, "%Y-%m-%d")
                .map_err(|_| pos_error::AppError::Validation("invalid from date".into()))?;
            let to = chrono::NaiveDate::parse_from_str(&filter.to, "%Y-%m-%d")
                .map_err(|_| pos_error::AppError::Validation("invalid to date".into()))?;

            let scope =
                xc_marketing::domain::performance::period::PerformanceScope::new(vec![branch]);
            let period =
                xc_marketing::domain::performance::period::PerformancePeriod::new(from, to);

            let reader = xc_marketing::infrastructure::pg_performance::PerformanceReader::new(
                pool, commission,
            );
            let perf_rows = reader.by_version(&scope, &period).await?;

            let versions = perf_rows
                .into_iter()
                .map(|r| VersionPerformanceRow {
                    campaign_id: r.campaign_id.to_string(),
                    campaign_name: r.campaign_name,
                    version_id: r.version_id.to_string(),
                    version: r.version as i64,
                    redemptions: r.redemptions,
                    price_cost_thb: crate::presentation::dto::v3::common::Money(
                        r.price_cost.to_string(),
                    ),
                    reward_expense_thb: crate::presentation::dto::v3::common::Money(
                        r.reward_expense.to_string(),
                    ),
                    volume_thb: crate::presentation::dto::v3::common::Money(
                        r.volume_thb.to_string(),
                    ),
                    realized_margin_thb: match r.realized_margin {
                        Ok(margin) => PerformanceFigure {
                            value: Some(crate::presentation::dto::v3::common::Money(
                                margin.to_string(),
                            )),
                            unavailable: None,
                        },
                        Err(u) => PerformanceFigure {
                            value: None,
                            unavailable: Some(u.as_str().to_string()),
                        },
                    },
                    attributed_contribution_thb: match r.contribution {
                        Ok(contrib) => PerformanceFigure {
                            value: Some(crate::presentation::dto::v3::common::Money(
                                contrib.to_string(),
                            )),
                            unavailable: None,
                        },
                        Err(u) => PerformanceFigure {
                            value: None,
                            unavailable: Some(u.as_str().to_string()),
                        },
                    },
                })
                .collect();

            Ok(CampaignPerformanceData { versions })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(CampaignPerformanceView::new(
            state.db.clone(),
            Arc::new(BrokerNotConnected),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignPerformanceFilter::export_all(&cfg)?;
    CampaignPerformanceData::export_all(&cfg)?;
    VersionPerformanceRow::export_all(&cfg)?;
    PerformanceFigure::export_all(&cfg)?;
    Ok(())
}
