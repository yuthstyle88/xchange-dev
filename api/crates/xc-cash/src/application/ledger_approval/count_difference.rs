//! `COUNT_DIFFERENCE` (V-1; design §5.2, §3.3).

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Row, Transaction};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome, RequestId};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role};
use xc_kernel::money::CurrencyCode;

use crate::domain::count::CashCount;
use crate::domain::inventory::repository::InventoryRepository;

/// The type name.
pub const COUNT_DIFFERENCE: &str = "COUNT_DIFFERENCE";

/// Registers `COUNT_DIFFERENCE` with `registry`.
pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    registry.register(
        Arc::new(CountDifferenceSubject::new(pool)),
        Arc::new(CountDifferencePolicy),
    );
}

pub struct CountDifferenceSubject {
    #[allow(dead_code)]
    pool: PgPool,
}

impl CountDifferenceSubject {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ApprovalSubject for CountDifferenceSubject {
    fn type_name(&self) -> &'static str {
        COUNT_DIFFERENCE
    }

    /// The count's state: its totals and whether it is verified.
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let count_row =
                sqlx::query("SELECT verified_by, verified_at FROM cash_counts WHERE id = $1::uuid")
                    .bind(object_id)
                    .fetch_optional(&mut **tx)
                    .await?;

            let count_row = count_row.ok_or_else(|| AppError::NotFound {
                entity: "CashCount",
                id: object_id.to_string(),
            })?;

            let verified_by: Option<uuid::Uuid> = count_row.try_get("verified_by")?;
            let verified_at: Option<chrono::DateTime<chrono::Utc>> =
                count_row.try_get("verified_at")?;

            let totals = sqlx::query(
                "SELECT currency, counted, expected, difference \
                 FROM cash_count_totals \
                 WHERE count_id = $1::uuid \
                 ORDER BY currency ASC",
            )
            .bind(object_id)
            .fetch_all(&mut **tx)
            .await?;

            let mut repr = format!(
                "verified_by:{:?}|verified_at:{:?}|totals:",
                verified_by, verified_at
            );
            for row in totals {
                let currency: String = row.try_get("currency")?;
                let counted: Decimal = row.try_get("counted")?;
                let expected: Option<Decimal> = row.try_get("expected")?;
                let difference: Option<Decimal> = row.try_get("difference")?;
                repr.push_str(&format!(
                    "[{}:{}:{}:{}],",
                    currency.trim(),
                    counted,
                    expected.map(|d| d.to_string()).unwrap_or_default(),
                    difference.map(|d| d.to_string()).unwrap_or_default(),
                ));
            }

            Ok(repr)
        })
    }

    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

/// `may_decide` ⇔ FINANCE of the request's branch (E-7); never the requester.
pub struct CountDifferencePolicy;

impl ApprovalPolicy for CountDifferencePolicy {
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let req_branch = request.branch_id.as_deref();
        let is_finance_of_branch = who
            .roles
            .iter()
            .any(|r| r.role == Role::Finance && r.covers(req_branch));
        if is_finance_of_branch {
            Ok(())
        } else if !who.roles.iter().any(|r| r.role == Role::Finance) {
            Err(Refusal::new(
                "outOfScope",
                "only finance staff may decide count difference",
            ))
        } else {
            Err(Refusal::new(
                "outOfScope",
                "finance staff can only decide requests for their own branch",
            ))
        }
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        false
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}

/// What the gate decided for a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    /// At or under the limit: nothing written.
    Clear,
    /// Over the limit (or any non-zero difference with no limit): the `COUNT_DIFFERENCE` request.
    AwaitingFinance(RequestId),
}

/// Called by `CashCountService::verify` (LG-5): values the count's difference at the branch average and
/// compares it with `cash_limits.count_difference_limit_thb`.
pub struct CountDifferenceGate {
    approvals: Arc<ApprovalService>,
    inventory: Arc<dyn InventoryRepository>,
}

impl CountDifferenceGate {
    pub fn new(approvals: Arc<ApprovalService>, inventory: Arc<dyn InventoryRepository>) -> Self {
        Self {
            approvals,
            inventory,
        }
    }

    /// `by` is the verifier (AP-1 records the requester and the request's branch from it). The
    /// request's idempotency key is `count-difference:<count id>`, so a second call for the same count
    /// answers the same request.
    pub async fn check_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        count: &CashCount,
        by: &Actor,
    ) -> AppResult<GateResult> {
        let rows = sqlx::query(
            "SELECT currency, difference \
             FROM cash_count_totals \
             WHERE count_id = $1::uuid \
             ORDER BY currency ASC",
        )
        .bind(&count.id)
        .fetch_all(&mut **tx)
        .await?;

        let mut net_difference_thb = Decimal::ZERO;
        let mut unvaluable = false;
        for row in rows {
            let currency: String = row.try_get("currency")?;
            let diff: Option<Decimal> = row.try_get("difference")?;
            let diff = match diff {
                Some(d) if !d.is_zero() => d,
                _ => continue,
            };

            let cur = currency.trim();
            if cur == "THB" {
                net_difference_thb += diff;
            } else {
                let code =
                    CurrencyCode::parse(cur).map_err(|e| AppError::Validation(e.to_string()))?;
                let pos = self
                    .inventory
                    .position_for_update_in(tx, &count.organization_id, &count.branch_id, code)
                    .await?;
                if pos.quantity > Decimal::ZERO {
                    let rate = pos.cost_thb / pos.quantity;
                    net_difference_thb += diff * rate;
                } else {
                    unvaluable = true;
                }
            }
        }

        if !unvaluable {
            let abs_diff = net_difference_thb.abs();
            if abs_diff.is_zero() {
                return Ok(GateResult::Clear);
            }

            let limit_row: Option<Option<Decimal>> = sqlx::query_scalar(
                "SELECT count_difference_limit_thb FROM cash_limits \
                 WHERE organization_id = $1::uuid AND branch_id = $2",
            )
            .bind(&count.organization_id)
            .bind(count.branch_id.as_str())
            .fetch_optional(&mut **tx)
            .await?;

            if let Some(Some(limit)) = limit_row {
                if abs_diff <= limit {
                    return Ok(GateResult::Clear);
                }
            }
        }

        let key = format!("count-difference:{}", count.id);
        let payload = serde_json::json!({
            "count_id": count.id,
            "net_difference_thb": net_difference_thb.to_string(),
        });

        let req = self
            .approvals
            .request(tx, COUNT_DIFFERENCE, &count.id, &payload, by, &key)
            .await
            .map_err(|e| match e {
                xc_approval::application::approval_service::ApprovalError::Database(err) => {
                    AppError::Database(sqlx::Error::Protocol(err))
                }
                _ => AppError::Internal(e.to_string()),
            })?;

        Ok(GateResult::AwaitingFinance(req.id))
    }
}
