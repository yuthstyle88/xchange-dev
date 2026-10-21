//! MK-7 (#1406): `GET /api/v3/views/campaign?id=` (M02, design `marketing-campaigns.md` §4.5): one campaign of the
//! caller's organisation with its versions, rules, simulations and approval; `campaign: null` for any other id.
//! Gate: `MarketingCampaigns` (policy row `campaign`). Served named-branch-first, as `campaigns` (see there).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::{HttpRequest, web};
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::campaign::{
    CampaignApprovalDetail, CampaignData, CampaignDetail, CampaignFilter, CampaignRuleDetail,
    CampaignSimulationDetail, CampaignVersionDetail,
};
use crate::presentation::http::v3::campaigns::serve_branch_first;
use crate::presentation::http::v3::view::{BranchView, ViewFilter};

impl ViewFilter for CampaignFilter {}

pub struct CampaignView {
    pool: PgPool,
}

impl CampaignView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for CampaignView {
    const NAME: &'static str = "campaign";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaign");
    type Data = CampaignData;
    type Filter = CampaignFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let org_id = who.organization_id.clone();
        let filter = filter.clone();

        async move {
            let Some(id_str) = filter.id.as_deref() else {
                return Ok(CampaignData { campaign: None });
            };
            let Ok(campaign_id) = uuid::Uuid::parse_str(id_str) else {
                return Ok(CampaignData { campaign: None });
            };

            use sqlx::Row;
            let campaign_row = sqlx::query(
                "SELECT id, name, current_version_id FROM campaigns WHERE id = $1 AND organization_id = $2::uuid",
            )
            .bind(campaign_id)
            .bind(&org_id)
            .fetch_optional(&pool)
            .await?;

            let Some(c_row) = campaign_row else {
                return Ok(CampaignData { campaign: None });
            };

            let name: String = c_row.get("name");
            let current_version_id: Option<uuid::Uuid> = c_row.get("current_version_id");

            let version_rows = sqlx::query(
                r#"
                SELECT
                    id, version, status, starts_at, ends_at, budget_thb, max_uses,
                    stack_group, priority, approval_id
                FROM campaign_versions
                WHERE campaign_id = $1
                ORDER BY version DESC
                "#,
            )
            .bind(campaign_id)
            .fetch_all(&pool)
            .await?;

            let version_ids: Vec<uuid::Uuid> = version_rows.iter().map(|v| v.get("id")).collect();
            let approval_ids: Vec<uuid::Uuid> = version_rows
                .iter()
                .filter_map(|v| v.get("approval_id"))
                .collect();

            let branch_rows = sqlx::query(
                "SELECT campaign_version_id, branch_id FROM campaign_branches WHERE campaign_version_id = ANY($1) ORDER BY branch_id ASC",
            )
            .bind(&version_ids)
            .fetch_all(&pool)
            .await?;

            let mut branches_by_version: std::collections::HashMap<uuid::Uuid, Vec<String>> =
                std::collections::HashMap::with_capacity(version_ids.len());
            for b in branch_rows {
                let vid: uuid::Uuid = b.get("campaign_version_id");
                let bid: String = b.get("branch_id");
                branches_by_version.entry(vid).or_default().push(bid);
            }

            let rule_rows = sqlx::query(
                r#"
                SELECT campaign_version_id, id, rule_no, benefit_type, benefit_class, requires_coupon, eligibility, calculation, caps
                FROM benefit_rules
                WHERE campaign_version_id = ANY($1)
                ORDER BY rule_no ASC
                "#,
            )
            .bind(&version_ids)
            .fetch_all(&pool)
            .await?;

            let mut rules_by_version: std::collections::HashMap<
                uuid::Uuid,
                Vec<CampaignRuleDetail>,
            > = std::collections::HashMap::with_capacity(version_ids.len());
            for r in rule_rows {
                let vid: uuid::Uuid = r.get("campaign_version_id");
                let r_id: uuid::Uuid = r.get("id");
                let rule_no: i32 = r.get("rule_no");
                let benefit_type: String = r.get("benefit_type");
                let benefit_class: String = r.get("benefit_class");
                let requires_coupon: bool = r.get("requires_coupon");
                let eligibility: serde_json::Value = r.get("eligibility");
                let calculation: serde_json::Value = r.get("calculation");
                let caps: serde_json::Value = r.get("caps");

                rules_by_version
                    .entry(vid)
                    .or_default()
                    .push(CampaignRuleDetail {
                        id: r_id.to_string(),
                        rule_no: rule_no as i64,
                        benefit_type,
                        benefit_class,
                        requires_coupon,
                        eligibility,
                        calculation,
                        caps,
                    });
            }

            let sim_rows = sqlx::query(
                r#"
                SELECT campaign_version_id, id, cost_source, simulated_at, input, result
                FROM campaign_simulations
                WHERE campaign_version_id = ANY($1)
                ORDER BY simulated_at DESC, id DESC
                "#,
            )
            .bind(&version_ids)
            .fetch_all(&pool)
            .await?;

            let mut simulations_by_version: std::collections::HashMap<
                uuid::Uuid,
                Vec<CampaignSimulationDetail>,
            > = std::collections::HashMap::with_capacity(version_ids.len());
            for s in sim_rows {
                let vid: uuid::Uuid = s.get("campaign_version_id");
                let s_id: uuid::Uuid = s.get("id");
                let cost_source: String = s.get("cost_source");
                let simulated_at: chrono::DateTime<chrono::Utc> = s.get("simulated_at");
                let input: serde_json::Value = s.get("input");
                let result: serde_json::Value = s.get("result");

                simulations_by_version
                    .entry(vid)
                    .or_default()
                    .push(CampaignSimulationDetail {
                        id: s_id.to_string(),
                        cost_source,
                        simulated_at: simulated_at.to_rfc3339(),
                        input,
                        result,
                    });
            }

            let ar_rows =
                sqlx::query("SELECT id, status FROM approval_requests WHERE id = ANY($1)")
                    .bind(&approval_ids)
                    .fetch_all(&pool)
                    .await?;

            let mut approvals_by_id: std::collections::HashMap<uuid::Uuid, CampaignApprovalDetail> =
                std::collections::HashMap::with_capacity(ar_rows.len());
            for r in ar_rows {
                let aid: uuid::Uuid = r.get("id");
                let ar_status: String = r.get("status");
                approvals_by_id.insert(
                    aid,
                    CampaignApprovalDetail {
                        id: aid.to_string(),
                        status: ar_status,
                    },
                );
            }

            let now = chrono::Utc::now();
            let mut versions = Vec::with_capacity(version_rows.len());

            for v in version_rows {
                let v_id: uuid::Uuid = v.get("id");
                let v_version: i32 = v.get("version");
                let stored_status: String = v.get("status");
                let starts_at: chrono::DateTime<chrono::Utc> = v.get("starts_at");
                let ends_at: chrono::DateTime<chrono::Utc> = v.get("ends_at");
                let mut budget_thb: rust_decimal::Decimal = v.get("budget_thb");
                budget_thb.rescale(2);
                let max_uses: Option<i32> = v.get("max_uses");
                let stack_group: String = v.get("stack_group");
                let priority: i32 = v.get("priority");
                let approval_id: Option<uuid::Uuid> = v.get("approval_id");

                let status = crate::presentation::http::v3::campaigns::derive_version_status(
                    &stored_status,
                    starts_at,
                    ends_at,
                    now,
                );

                let branches = branches_by_version.remove(&v_id).unwrap_or_default();
                let rules = rules_by_version.remove(&v_id).unwrap_or_default();
                let simulations = simulations_by_version.remove(&v_id).unwrap_or_default();
                let approval = approval_id.and_then(|aid| approvals_by_id.remove(&aid));

                versions.push(CampaignVersionDetail {
                    id: v_id.to_string(),
                    version: v_version as i64,
                    status,
                    starts_at: starts_at.to_rfc3339(),
                    ends_at: ends_at.to_rfc3339(),
                    budget_thb: crate::presentation::dto::v3::common::Money(format!(
                        "{budget_thb:.2}"
                    )),
                    max_uses: max_uses.map(|m| m as i64),
                    stack_group,
                    priority: priority as i64,
                    branches,
                    rules,
                    simulations,
                    approval,
                });
            }

            Ok(CampaignData {
                campaign: Some(CampaignDetail {
                    id: campaign_id.to_string(),
                    name,
                    current_version_id: current_version_id.map(|u| u.to_string()),
                    versions,
                }),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(CampaignView::new(state.db.clone()));
    cfg.route(
        &format!("/api/v3/views/{}", CampaignView::NAME),
        web::get().to(move |req: HttpRequest| {
            let view = view.clone();
            async move { serve_branch_first(&req, view.as_ref()).await }
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignFilter::export_all(&cfg)?;
    CampaignData::export_all(&cfg)?;
    CampaignDetail::export_all(&cfg)?;
    CampaignVersionDetail::export_all(&cfg)?;
    CampaignRuleDetail::export_all(&cfg)?;
    CampaignSimulationDetail::export_all(&cfg)?;
    CampaignApprovalDetail::export_all(&cfg)?;
    Ok(())
}
