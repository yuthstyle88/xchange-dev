//! Rate draft service: create, edit, submit, copy, and get.

use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::sync::Arc;
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::domain::request::RequestId;
use xc_auth::domain::access;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, UnitSize};

use crate::domain::draft::draft_evaluation::DraftEvaluation;
use crate::domain::draft::rate_draft::{DraftId, DraftLine, DraftStatus, LineSide, RateDraft};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::draft::rate_proposal::RateProposal;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::reference::reference_batch::ReferenceLine;

#[derive(Debug, Clone)]
pub struct SubmitOutcome {
    pub request_id: RequestId,
    pub status: DraftStatus,
}

#[derive(Debug, Clone)]
pub struct DraftEvaluationOutcome {
    pub version: i32,
    pub status: String,
    pub evaluation: crate::domain::draft::draft_evaluation::Evaluation,
    pub ref_ids: Vec<Uuid>,
    pub policy: Option<RatePolicy>,
    pub lines: Vec<DraftLine>,
    pub proposals: Vec<crate::domain::draft::rate_proposal::ProposedLine>,
    pub references: Vec<ReferenceLine>,
}

pub struct RateDraftService {
    pool: PgPool,
    approvals: Arc<ApprovalService>,
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

impl RateDraftService {
    pub fn new(pool: PgPool, approvals: Arc<ApprovalService>) -> Self {
        Self { pool, approvals }
    }

    pub async fn create(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &access::Actor,
        lines: Vec<DraftLine>,
    ) -> Result<RateDraft, RateEngineError> {
        // Validate manual reasons upfront
        for line in &lines {
            if matches!(line.buy, LineSide::Manual(_)) || matches!(line.sell, LineSide::Manual(_)) {
                match &line.reason {
                    Some(r) if !r.trim().is_empty() => (),
                    _ => {
                        return Err(RateEngineError::ValidationFailed(
                            "manual side requires a reason".into(),
                        ));
                    }
                }
            }
        }

        let draft_id = DraftId::generate();
        let branch_str = branch.as_str();
        let who_str = who.user_id.as_str();

        // Find active policy version for branch
        let policy_row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_from <= now() AND (valid_to IS NULL OR valid_to > now()) \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(branch_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let policy_id = policy_row.map(|r| r.0);

        sqlx::query(
            "INSERT INTO rate_drafts (id, branch_id, status, created_by, version) \
             VALUES ($1, $2, 'DRAFT', $3, 1)",
        )
        .bind(draft_id.as_uuid())
        .bind(branch_str)
        .bind(who_str)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        for line in &lines {
            // The newest 'BOT' line of this currency is the AUTO base; a 'PARTNER' line is
            // reference data for `source_snapshot.partner` only (rates.md §4.1 "Partner cost")
            let ref_row: Option<(Uuid,)> = sqlx::query_as(
                "SELECT l.id FROM rate_reference_lines l \
                 JOIN rate_reference_batches b ON b.id = l.batch_id \
                 WHERE l.currency_code = $1 AND l.reference_type = 'BOT' \
                 ORDER BY b.fetched_at DESC LIMIT 1",
            )
            .bind(line.currency.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (buy_mode, manual_buy) = match &line.buy {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };
            let (sell_mode, manual_sell) = match &line.sell {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };

            sqlx::query(
                "INSERT INTO rate_draft_lines \
                 (id, draft_id, branch_id, currency_code, denomination_group, reference_line_id, \
                  policy_version_id, buy_mode, sell_mode, manual_buy, manual_sell, reason) \
                 VALUES (gen_random_uuid(), $1, $2, $3, 'ALL', $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(draft_id.as_uuid())
            .bind(branch_str)
            .bind(line.currency.as_str())
            .bind(ref_row.map(|r| r.0))
            .bind(policy_id)
            .bind(buy_mode)
            .bind(sell_mode)
            .bind(manual_buy)
            .bind(manual_sell)
            .bind(&line.reason)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        }

        Ok(RateDraft {
            id: draft_id,
            status: DraftStatus::Draft,
            version: 1,
            lines,
        })
    }

    pub async fn edit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        _who: &access::Actor,
        id: &DraftId,
        expected_version: i32,
        lines: Vec<DraftLine>,
    ) -> Result<RateDraft, RateEngineError> {
        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT version, status FROM rate_drafts WHERE id = $1 AND branch_id = $2 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (current_version, status) =
            row.ok_or_else(|| RateEngineError::ValidationFailed(format!("draft not found: {id}")))?;

        if status != "DRAFT" {
            return Err(RateEngineError::StaleVersion(
                "an edit after submit is refused".into(),
            ));
        }

        if current_version != expected_version {
            return Err(RateEngineError::StaleVersion(format!(
                "expected version {expected_version}, got {current_version}"
            )));
        }

        // Validate manual reasons
        for line in &lines {
            if matches!(line.buy, LineSide::Manual(_)) || matches!(line.sell, LineSide::Manual(_)) {
                match &line.reason {
                    Some(r) if !r.trim().is_empty() => (),
                    _ => {
                        return Err(RateEngineError::ValidationFailed(
                            "manual side requires a reason".into(),
                        ));
                    }
                }
            }
        }

        let new_version = current_version + 1;
        let branch_str = branch.as_str();

        let policy_row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_from <= now() AND (valid_to IS NULL OR valid_to > now()) \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(branch_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let policy_id = policy_row.map(|r| r.0);

        sqlx::query("UPDATE rate_drafts SET version = $1, updated_at = now() WHERE id = $2")
            .bind(new_version)
            .bind(id.as_uuid())
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        sqlx::query("DELETE FROM rate_draft_lines WHERE draft_id = $1")
            .bind(id.as_uuid())
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        for line in &lines {
            let ref_row: Option<(Uuid,)> = sqlx::query_as(
                "SELECT l.id FROM rate_reference_lines l \
                 JOIN rate_reference_batches b ON b.id = l.batch_id \
                 WHERE l.currency_code = $1 AND l.reference_type = 'BOT' \
                 ORDER BY b.fetched_at DESC LIMIT 1",
            )
            .bind(line.currency.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (buy_mode, manual_buy) = match &line.buy {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };
            let (sell_mode, manual_sell) = match &line.sell {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };

            sqlx::query(
                "INSERT INTO rate_draft_lines \
                 (id, draft_id, branch_id, currency_code, denomination_group, reference_line_id, \
                  policy_version_id, buy_mode, sell_mode, manual_buy, manual_sell, reason) \
                 VALUES (gen_random_uuid(), $1, $2, $3, 'ALL', $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(id.as_uuid())
            .bind(branch_str)
            .bind(line.currency.as_str())
            .bind(ref_row.map(|r| r.0))
            .bind(policy_id)
            .bind(buy_mode)
            .bind(sell_mode)
            .bind(manual_buy)
            .bind(manual_sell)
            .bind(&line.reason)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        }

        Ok(RateDraft {
            id: *id,
            status: DraftStatus::Draft,
            version: new_version,
            lines,
        })
    }

    pub async fn evaluate(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        id: &DraftId,
    ) -> Result<DraftEvaluationOutcome, RateEngineError> {
        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT version, status FROM rate_drafts WHERE id = $1 AND branch_id = $2",
        )
        .bind(id.as_uuid())
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (version, status) =
            row.ok_or_else(|| RateEngineError::NotFound(format!("draft not found: {id}")))?;

        // Load lines
        let db_lines = sqlx::query(
            "SELECT currency_code, buy_mode, sell_mode, manual_buy, manual_sell, reason, \
                    reference_line_id, policy_version_id \
             FROM rate_draft_lines WHERE draft_id = $1 ORDER BY id",
        )
        .bind(id.as_uuid())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut lines = Vec::new();
        let mut ref_ids: Vec<Uuid> = Vec::new();
        let mut policy_version_id: Option<Uuid> = None;

        for r in &db_lines {
            let code_str: String = r.get("currency_code");
            let c = CurrencyCode::parse(code_str.trim())
                .map_err(|e| RateEngineError::Internal(format!("invalid currency code: {e}")))?;
            let buy_mode: String = r.get("buy_mode");
            let manual_buy: Option<Decimal> = r.get("manual_buy");
            let sell_mode: String = r.get("sell_mode");
            let manual_sell: Option<Decimal> = r.get("manual_sell");
            let reason: Option<String> = r.get("reason");
            let ref_id: Option<Uuid> = r.get("reference_line_id");
            let pol_id: Option<Uuid> = r.get("policy_version_id");

            if let Some(rf) = ref_id {
                if !ref_ids.contains(&rf) {
                    ref_ids.push(rf);
                }
            }
            if policy_version_id.is_none() {
                policy_version_id = pol_id;
            }

            let buy = match buy_mode.as_str() {
                "AUTO" => LineSide::Auto,
                _ => {
                    let mb = manual_buy.ok_or_else(|| {
                        RateEngineError::ValidationFailed("missing manual buy".into())
                    })?;
                    LineSide::Manual(Rate::parse(&mb.to_string()).map_err(|e| {
                        RateEngineError::Internal(format!("invalid manual buy: {e}"))
                    })?)
                }
            };
            let sell = match sell_mode.as_str() {
                "AUTO" => LineSide::Auto,
                _ => {
                    let ms = manual_sell.ok_or_else(|| {
                        RateEngineError::ValidationFailed("missing manual sell".into())
                    })?;
                    LineSide::Manual(Rate::parse(&ms.to_string()).map_err(|e| {
                        RateEngineError::Internal(format!("invalid manual sell: {e}"))
                    })?)
                }
            };

            lines.push(DraftLine::new(c, buy, sell, reason));
        }

        // Load policy
        let policy = if let Some(pol_id) = policy_version_id {
            let pol_row = sqlx::query(
                "SELECT id::text, buy_adjust_pct, sell_adjust_pct, guards FROM rate_policy_versions WHERE id = $1",
            )
            .bind(pol_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            if let Some(pr) = pol_row {
                let pid: String = pr.get(0);
                let buy_pct: Decimal = pr.get(1);
                let sell_pct: Decimal = pr.get(2);
                let guards: serde_json::Value = pr.get(3);
                Some(RatePolicy::new(
                    pid,
                    RatePercent::parse(&buy_pct.to_string()).unwrap(),
                    RatePercent::parse(&sell_pct.to_string()).unwrap(),
                    &guards,
                )?)
            } else {
                None
            }
        } else {
            None
        };

        // Load references
        let mut references = Vec::new();
        for ref_id in &ref_ids {
            let ref_row = sqlx::query(
                "SELECT currency_code, unit_size, buy, sell FROM rate_reference_lines WHERE id = $1",
            )
            .bind(ref_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            if let Some(rr) = ref_row {
                let code_str: String = rr.get(0);
                let c = CurrencyCode::parse(code_str.trim())
                    .map_err(|e| RateEngineError::Internal(format!("invalid code: {e}")))?;
                let unit_val: Decimal = rr.get(1);
                let buy_val: Option<Decimal> = rr.get(2);
                let sell_val: Option<Decimal> = rr.get(3);
                let u = UnitSize::parse(&unit_val.to_string())
                    .map_err(|e| RateEngineError::Internal(format!("invalid unit: {e}")))?;
                let buy = buy_val.map(|b| Rate::parse(&b.to_string()).unwrap());
                let sell = sell_val.map(|s| Rate::parse(&s.to_string()).unwrap());
                references.push(ReferenceLine::new(c, u, buy, sell));
            }
        }

        // Calculate proposals
        let mut proposals = Vec::new();
        for line in &lines {
            // Read currency units
            let c_row = sqlx::query("SELECT unit_size, minor_unit FROM currencies WHERE code = $1")
                .bind(line.currency.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (unit_val, minor_val) = match c_row {
                Some(r) => (r.get::<Decimal, _>(0), r.get::<i16, _>(1) as u8),
                None => (Decimal::ONE, 2u8),
            };

            let units = CurrencyUnits::new(
                line.currency,
                UnitSize::parse(&unit_val.to_string()).unwrap(),
                MinorUnit::new(minor_val).unwrap(),
            );

            let ref_line = references.iter().find(|r| r.currency == line.currency);
            let prop = RateProposal::of(line, ref_line, policy.as_ref(), &units)?;
            proposals.push(prop);
        }

        let evaluation = DraftEvaluation::of(&lines, &proposals, &references, policy.as_ref());

        Ok(DraftEvaluationOutcome {
            version,
            status,
            evaluation,
            ref_ids,
            policy,
            lines,
            proposals,
            references,
        })
    }

    pub async fn submit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &access::Actor,
        id: &DraftId,
        key: &str,
    ) -> Result<SubmitOutcome, RateEngineError> {
        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT version, status FROM rate_drafts WHERE id = $1 AND branch_id = $2 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (_version, status) =
            row.ok_or_else(|| RateEngineError::NotFound(format!("draft not found: {id}")))?;

        if status != "DRAFT" {
            return Err(RateEngineError::StaleVersion(
                "draft is not in DRAFT status".into(),
            ));
        }

        let eval_res = self.evaluate(tx, branch, id).await?;

        let out_of_bound = eval_res
            .evaluation
            .out_of_bound()
            .iter()
            .map(|(c, s)| {
                json!({
                    "currency": c.as_str(),
                    "side": match s {
                        xc_kernel::money::Side::Buy => "BUY",
                        xc_kernel::money::Side::Sell => "SELL",
                    },
                })
            })
            .collect::<Vec<_>>();

        // Canonical JSON representation for evaluation and hash
        let eval_json = json!({
            "draft_id": id.to_string(),
            "policy_version_id": eval_res.policy.as_ref().map(|p| p.id()),
            "reference_line_ids": eval_res.ref_ids.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
            "is_in_bound": eval_res.evaluation.is_in_bound(),
            "out_of_bound": out_of_bound,
        });
        let eval_hash = sha256_hex(&serde_json::to_vec(&eval_json).unwrap());

        sqlx::query(
            "UPDATE rate_drafts SET status = 'PENDING_APPROVAL', evaluation = $1, evaluation_hash = $2, updated_at = now() \
             WHERE id = $3",
        )
        .bind(&eval_json)
        .bind(&eval_hash)
        .bind(id.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let request_res = self
            .approvals
            .request(
                tx,
                "RATE_PUBLICATION",
                &id.to_string(),
                &eval_json,
                who,
                key,
            )
            .await;

        let request = match request_res {
            Ok(req) => req,
            Err(e) => {
                return Err(match e {
                    xc_approval::application::approval_service::ApprovalError::IdempotencyConflict(s) => {
                        RateEngineError::IdempotencyConflict(s)
                    }
                    xc_approval::application::approval_service::ApprovalError::OutOfScope(s) => {
                        RateEngineError::OutOfScope(s)
                    }
                    xc_approval::application::approval_service::ApprovalError::StaleVersion(s) => {
                        RateEngineError::StaleVersion(s)
                    }
                    _ => RateEngineError::Internal(e.to_string()),
                });
            }
        };

        let final_status = if eval_res.evaluation.is_in_bound() {
            let pol_ver = eval_res.policy.as_ref().map(|p| p.id()).unwrap_or("1");
            let decide_res = self
                .approvals
                .decide_by_policy(tx, &request.id, pol_ver)
                .await;
            decide_res.map_err(|e| RateEngineError::Internal(e.to_string()))?;
            DraftStatus::Approved
        } else if who.roles.iter().any(|r| r.role == access::Role::Executive) {
            let decide_res = self
                .approvals
                .decide_self(tx, &request.id, who, "proposer_is_owner")
                .await;
            decide_res.map_err(|e| match e {
                xc_approval::application::approval_service::ApprovalError::OutOfScope(s) => {
                    RateEngineError::OutOfScope(s)
                }
                _ => RateEngineError::Internal(e.to_string()),
            })?;
            DraftStatus::Approved
        } else {
            DraftStatus::PendingApproval
        };

        Ok(SubmitOutcome {
            request_id: request.id,
            status: final_status,
        })
    }

    pub async fn copy(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &access::Actor,
        from: &DraftId,
    ) -> Result<RateDraft, RateEngineError> {
        let from_row: Option<(String,)> =
            sqlx::query_as("SELECT branch_id FROM rate_drafts WHERE id = $1 AND branch_id = $2")
                .bind(from.as_uuid())
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if from_row.is_none() {
            return Err(RateEngineError::NotFound(format!(
                "draft not found: {from}"
            )));
        }

        let db_lines = sqlx::query(
            "SELECT l.currency_code, l.buy_mode, l.sell_mode, l.manual_buy, l.manual_sell, l.reason \
             FROM rate_draft_lines l \
             JOIN rate_drafts d ON d.id = l.draft_id \
             WHERE l.draft_id = $1 AND d.branch_id = $2 ORDER BY l.id",
        )
        .bind(from.as_uuid())
        .bind(branch.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut lines = Vec::new();
        for r in &db_lines {
            let code_str: String = r.get("currency_code");
            let c = CurrencyCode::parse(code_str.trim())
                .map_err(|e| RateEngineError::Internal(format!("invalid currency code: {e}")))?;
            let buy_mode: String = r.get("buy_mode");
            let manual_buy: Option<Decimal> = r.get("manual_buy");
            let sell_mode: String = r.get("sell_mode");
            let manual_sell: Option<Decimal> = r.get("manual_sell");
            let reason: Option<String> = r.get("reason");

            let buy = match buy_mode.as_str() {
                "AUTO" => LineSide::Auto,
                _ => {
                    let mb = manual_buy.ok_or_else(|| {
                        RateEngineError::Internal("missing manual buy rate".to_string())
                    })?;
                    LineSide::Manual(Rate::parse(&mb.to_string()).map_err(|e| {
                        RateEngineError::Internal(format!("invalid manual buy: {e}"))
                    })?)
                }
            };
            let sell = match sell_mode.as_str() {
                "AUTO" => LineSide::Auto,
                _ => {
                    let ms = manual_sell.ok_or_else(|| {
                        RateEngineError::Internal("missing manual sell rate".to_string())
                    })?;
                    LineSide::Manual(Rate::parse(&ms.to_string()).map_err(|e| {
                        RateEngineError::Internal(format!("invalid manual sell: {e}"))
                    })?)
                }
            };

            lines.push(DraftLine::new(c, buy, sell, reason));
        }

        self.create(tx, branch, who, lines).await
    }

    pub async fn get(
        &self,
        branch: &BranchId,
        id: &DraftId,
    ) -> Result<Option<RateDraft>, RateEngineError> {
        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT version, status FROM rate_drafts WHERE id = $1 AND branch_id = $2",
        )
        .bind(id.as_uuid())
        .bind(branch.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (version, status_str) = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let status = DraftStatus::parse_db(&status_str)
            .ok_or_else(|| RateEngineError::Internal(format!("invalid status: {status_str}")))?;

        let db_lines = sqlx::query(
            "SELECT currency_code, buy_mode, sell_mode, manual_buy, manual_sell, reason \
             FROM rate_draft_lines WHERE draft_id = $1 ORDER BY id",
        )
        .bind(id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut lines = Vec::new();
        for r in &db_lines {
            let code_str: String = r.get("currency_code");
            let c = CurrencyCode::parse(code_str.trim())
                .map_err(|e| RateEngineError::Internal(format!("invalid currency code: {e}")))?;
            let buy_mode: String = r.get("buy_mode");
            let manual_buy: Option<Decimal> = r.get("manual_buy");
            let sell_mode: String = r.get("sell_mode");
            let manual_sell: Option<Decimal> = r.get("manual_sell");
            let reason: Option<String> = r.get("reason");

            let buy =
                match buy_mode.as_str() {
                    "AUTO" => LineSide::Auto,
                    _ => LineSide::Manual(Rate::parse(&manual_buy.unwrap().to_string()).map_err(
                        |e| RateEngineError::Internal(format!("invalid manual buy: {e}")),
                    )?),
                };
            let sell =
                match sell_mode.as_str() {
                    "AUTO" => LineSide::Auto,
                    _ => LineSide::Manual(Rate::parse(&manual_sell.unwrap().to_string()).map_err(
                        |e| RateEngineError::Internal(format!("invalid manual sell: {e}")),
                    )?),
                };

            lines.push(DraftLine::new(c, buy, sell, reason));
        }

        Ok(Some(RateDraft {
            id: *id,
            status,
            version,
            lines,
        }))
    }
}
