//! Rate draft service: create, edit, submit, copy, and get.

use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::domain::request::RequestId;
use xc_auth::domain::access;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, UnitSize};

use super::reference_base::ReferenceBase;
use crate::domain::draft::draft_evaluation::DraftEvaluation;
use crate::domain::draft::rate_draft::{DraftId, DraftLine, DraftStatus, LineSide, RateDraft};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::draft::rate_proposal::RateProposal;
use crate::domain::draft::tier_proposal::{TierLine, TierProposal};
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;
use crate::domain::policy::policy_tiers::PolicyTiers;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::pricing::tier_set::TierRate;
use crate::domain::reference::reference_batch::ReferenceLine;
use crate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use crate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;

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
    pub tiers: Vec<Option<PolicyTiers>>,
    pub tier_lines: Vec<Option<Vec<TierLine>>>,
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

fn refuse_duplicate_currency(lines: &[DraftLine]) -> Result<(), RateEngineError> {
    let mut seen = HashSet::with_capacity(lines.len());
    for line in lines {
        if !seen.insert(line.currency) {
            return Err(RateEngineError::ValidationFailed(format!(
                "duplicate currency in draft lines: {}",
                line.currency
            )));
        }
    }
    Ok(())
}

async fn insert_line_tiers(
    tx: &mut Transaction<'_, Postgres>,
    line_id: Uuid,
    tiers: &[TierRate; 4],
) -> Result<(), RateEngineError> {
    for (idx, tier) in tiers.iter().enumerate() {
        sqlx::query(
            "INSERT INTO rate_draft_line_tiers (draft_line_id, tier_no, buy, sell) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(line_id)
        .bind((idx + 1) as i16)
        .bind(tier.buy.value())
        .bind(tier.sell.value())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;
    }
    Ok(())
}

fn decode_tier_rows(
    rows: Vec<(Uuid, i16, Decimal, Decimal)>,
) -> Result<std::collections::HashMap<Uuid, [TierRate; 4]>, RateEngineError> {
    let mut grouped: std::collections::HashMap<Uuid, Vec<(i16, TierRate)>> =
        std::collections::HashMap::new();
    for (line_id, tier_no, buy_dec, sell_dec) in rows {
        let buy = Rate::parse(&buy_dec.to_string())
            .map_err(|e| RateEngineError::Internal(format!("invalid tier buy: {e}")))?;
        let sell = Rate::parse(&sell_dec.to_string())
            .map_err(|e| RateEngineError::Internal(format!("invalid tier sell: {e}")))?;
        grouped
            .entry(line_id)
            .or_default()
            .push((tier_no, TierRate { buy, sell }));
    }
    let mut map = std::collections::HashMap::with_capacity(grouped.len());
    for (line_id, mut list) in grouped {
        list.sort_by_key(|(no, _)| *no);
        if list.len() == 4
            && list
                .iter()
                .enumerate()
                .all(|(i, (no, _))| *no as usize == i + 1)
        {
            map.insert(line_id, [list[0].1, list[1].1, list[2].1, list[3].1]);
        }
    }
    Ok(map)
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
        refuse_duplicate_currency(&lines)?;

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
            let ref_id = ReferenceBase::newest_in(tx, branch, &line.currency).await?;

            let (buy_mode, manual_buy) = match &line.buy {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };
            let (sell_mode, manual_sell) = match &line.sell {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };

            let line_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO rate_draft_lines \
                 (id, draft_id, branch_id, currency_code, denomination_group, reference_line_id, \
                  policy_version_id, buy_mode, sell_mode, manual_buy, manual_sell, reason) \
                 VALUES ($1, $2, $3, $4, 'ALL', $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(line_id)
            .bind(draft_id.as_uuid())
            .bind(branch_str)
            .bind(line.currency.as_str())
            .bind(ref_id)
            .bind(policy_id)
            .bind(buy_mode)
            .bind(sell_mode)
            .bind(manual_buy)
            .bind(manual_sell)
            .bind(&line.reason)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            if let Some(tiers) = &line.tiers {
                insert_line_tiers(tx, line_id, tiers).await?;
            }
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
            row.ok_or_else(|| RateEngineError::NotFound(format!("draft not found: {id}")))?;

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

        refuse_duplicate_currency(&lines)?;

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
            let ref_id = ReferenceBase::newest_in(tx, branch, &line.currency).await?;

            let (buy_mode, manual_buy) = match &line.buy {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };
            let (sell_mode, manual_sell) = match &line.sell {
                LineSide::Auto => ("AUTO", None),
                LineSide::Manual(rate) => ("MANUAL", Some(rate.value())),
            };

            let line_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO rate_draft_lines \
                 (id, draft_id, branch_id, currency_code, denomination_group, reference_line_id, \
                  policy_version_id, buy_mode, sell_mode, manual_buy, manual_sell, reason) \
                 VALUES ($1, $2, $3, $4, 'ALL', $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(line_id)
            .bind(id.as_uuid())
            .bind(branch_str)
            .bind(line.currency.as_str())
            .bind(ref_id)
            .bind(policy_id)
            .bind(buy_mode)
            .bind(sell_mode)
            .bind(manual_buy)
            .bind(manual_sell)
            .bind(&line.reason)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            if let Some(tiers) = &line.tiers {
                insert_line_tiers(tx, line_id, tiers).await?;
            }
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
            "SELECT id, currency_code, buy_mode, sell_mode, manual_buy, manual_sell, reason, \
                    reference_line_id, policy_version_id \
             FROM rate_draft_lines WHERE draft_id = $1 ORDER BY id",
        )
        .bind(id.as_uuid())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let raw_tiers: Vec<(Uuid, i16, Decimal, Decimal)> = sqlx::query_as(
            "SELECT t.draft_line_id, t.tier_no, t.buy, t.sell \
             FROM rate_draft_line_tiers t \
             JOIN rate_draft_lines l ON l.id = t.draft_line_id \
             WHERE l.draft_id = $1 \
             ORDER BY t.draft_line_id, t.tier_no",
        )
        .bind(id.as_uuid())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;
        let mut tiers_by_line = decode_tier_rows(raw_tiers)?;

        let mut lines = Vec::new();
        let mut line_policy_ids = Vec::new();
        let mut ref_ids: Vec<Uuid> = Vec::new();
        let mut policy_version_id: Option<Uuid> = None;

        for r in &db_lines {
            let line_id: Uuid = r.get("id");
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
            line_policy_ids.push(pol_id);

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

            let mut line = DraftLine::new(c, buy, sell, reason);
            line.tiers = tiers_by_line.remove(&line_id);
            lines.push(line);
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
        let ref_rows = sqlx::query(
            "SELECT id, currency_code, unit_size, buy, sell FROM rate_reference_lines WHERE id = ANY($1)",
        )
        .bind(&ref_ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut references_by_id: std::collections::HashMap<Uuid, ReferenceLine> =
            std::collections::HashMap::with_capacity(ref_rows.len());
        for rr in ref_rows {
            let rid: Uuid = rr.get(0);
            let code_str: String = rr.get(1);
            let c = CurrencyCode::parse(code_str.trim())
                .map_err(|e| RateEngineError::Internal(format!("invalid code: {e}")))?;
            let unit_val: Decimal = rr.get(2);
            let buy_val: Option<Decimal> = rr.get(3);
            let sell_val: Option<Decimal> = rr.get(4);
            let u = UnitSize::parse(&unit_val.to_string())
                .map_err(|e| RateEngineError::Internal(format!("invalid unit: {e}")))?;
            let buy = buy_val.map(|b| Rate::parse(&b.to_string()).unwrap());
            let sell = sell_val.map(|s| Rate::parse(&s.to_string()).unwrap());
            references_by_id.insert(rid, ReferenceLine::new(c, u, buy, sell));
        }
        let references: Vec<ReferenceLine> = ref_ids
            .iter()
            .filter_map(|ref_id| references_by_id.get(ref_id).cloned())
            .collect();

        // Load currency units
        let currency_codes: Vec<&str> = lines.iter().map(|l| l.currency.as_str()).collect();
        let currency_rows =
            sqlx::query("SELECT code, unit_size, minor_unit FROM currencies WHERE code = ANY($1)")
                .bind(&currency_codes)
                .fetch_all(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut currencies_by_code: std::collections::HashMap<String, (Decimal, u8)> =
            std::collections::HashMap::with_capacity(currency_rows.len());
        for r in currency_rows {
            let code_str: String = r.get(0);
            let unit_val: Decimal = r.get(1);
            let minor_val = r.get::<i16, _>(2) as u8;
            currencies_by_code.insert(code_str.trim().to_owned(), (unit_val, minor_val));
        }

        // Calculate proposals and tiers
        let mut proposals = Vec::new();
        let mut tiers = Vec::with_capacity(lines.len());
        let mut tier_lines_vec = Vec::with_capacity(lines.len());
        let mut policy_tiers_cache: std::collections::HashMap<
            Uuid,
            (Option<PolicyTiers>, Vec<CurrencyTierOverride>),
        > = std::collections::HashMap::new();

        for (i, line) in lines.iter().enumerate() {
            // Read currency units
            let (unit_val, minor_val) = currencies_by_code
                .get(line.currency.as_str())
                .copied()
                .unwrap_or((Decimal::ONE, 2u8));

            let units = CurrencyUnits::new(
                line.currency,
                UnitSize::parse(&unit_val.to_string()).unwrap(),
                MinorUnit::new(minor_val).unwrap(),
            );

            let ref_line = references.iter().find(|r| r.currency == line.currency);
            let prop = RateProposal::of(line, ref_line, policy.as_ref(), &units)?;
            proposals.push(prop.clone());

            if line.tiers.is_some() {
                tier_lines_vec.push(None);
                tiers.push(None);
                continue;
            }

            let line_pol_id = line_policy_ids.get(i).copied().flatten();
            let pt_opt = if let Some(pid) = line_pol_id {
                let (branch_tiers, overrides) = if let Some(cached) = policy_tiers_cache.get(&pid) {
                    cached.clone()
                } else {
                    let loaded = if let Some(pt) = PolicyTiersStore::for_version(tx, &pid).await? {
                        let ovs = CurrencyTierOverrideStore::for_version(tx, &pid).await?;
                        (Some(pt), ovs)
                    } else {
                        (None, Vec::new())
                    };
                    policy_tiers_cache.insert(pid, loaded.clone());
                    loaded
                };
                branch_tiers.map(|pt| {
                    let ov = overrides.iter().find(|o| o.currency == line.currency);
                    pt.for_currency(ov)
                })
            } else {
                None
            };

            if let Some(ref pt) = pt_opt {
                let tlines = TierProposal::of(&prop, line, ref_line, policy.as_ref(), pt, &units)?;
                tier_lines_vec.push(Some(tlines));
            } else {
                tier_lines_vec.push(None);
            }
            tiers.push(pt_opt);
        }

        let evaluation =
            DraftEvaluation::of_tiered(&lines, &proposals, &references, policy.as_ref(), &tiers);

        Ok(DraftEvaluationOutcome {
            version,
            status,
            evaluation,
            ref_ids,
            policy,
            lines,
            proposals,
            references,
            tiers,
            tier_lines: tier_lines_vec,
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
        let mut eval_json = json!({
            "draft_id": id.to_string(),
            "policy_version_id": eval_res.policy.as_ref().map(|p| p.id()),
            "reference_line_ids": eval_res.ref_ids.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
            "is_in_bound": eval_res.evaluation.is_in_bound(),
            "out_of_bound": out_of_bound,
        });

        let mut all_tier_lines = Vec::new();
        for (i, opt_lines) in eval_res.tier_lines.iter().enumerate() {
            if let Some(tlines) = opt_lines {
                let currency = eval_res.lines[i].currency.as_str();
                for tl in tlines {
                    all_tier_lines.push(json!({
                        "currency": currency,
                        "tier": tl.code.as_str(),
                        "buy": tl.buy.map(|r| r.to_string()),
                        "sell": tl.sell.map(|r| r.to_string()),
                        "base_buy": tl.base_buy.map(|r| r.to_string()),
                        "base_sell": tl.base_sell.map(|r| r.to_string()),
                    }));
                }
            }
        }
        if !all_tier_lines.is_empty() {
            eval_json["tiers"] = json!(all_tier_lines);
        }

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
            "SELECT l.id, l.currency_code, l.buy_mode, l.sell_mode, l.manual_buy, l.manual_sell, l.reason \
             FROM rate_draft_lines l \
             JOIN rate_drafts d ON d.id = l.draft_id \
             WHERE l.draft_id = $1 AND d.branch_id = $2 ORDER BY l.id",
        )
        .bind(from.as_uuid())
        .bind(branch.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let raw_tiers: Vec<(Uuid, i16, Decimal, Decimal)> = sqlx::query_as(
            "SELECT t.draft_line_id, t.tier_no, t.buy, t.sell \
             FROM rate_draft_line_tiers t \
             JOIN rate_draft_lines l ON l.id = t.draft_line_id \
             WHERE l.draft_id = $1 \
             ORDER BY t.draft_line_id, t.tier_no",
        )
        .bind(from.as_uuid())
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;
        let mut tiers_by_line = decode_tier_rows(raw_tiers)?;

        let mut lines = Vec::new();
        for r in &db_lines {
            let line_id: Uuid = r.get("id");
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

            let mut line = DraftLine::new(c, buy, sell, reason);
            line.tiers = tiers_by_line.remove(&line_id);
            lines.push(line);
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
            "SELECT id, currency_code, buy_mode, sell_mode, manual_buy, manual_sell, reason \
             FROM rate_draft_lines WHERE draft_id = $1 ORDER BY id",
        )
        .bind(id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let raw_tiers: Vec<(Uuid, i16, Decimal, Decimal)> = sqlx::query_as(
            "SELECT t.draft_line_id, t.tier_no, t.buy, t.sell \
             FROM rate_draft_line_tiers t \
             JOIN rate_draft_lines l ON l.id = t.draft_line_id \
             WHERE l.draft_id = $1 \
             ORDER BY t.draft_line_id, t.tier_no",
        )
        .bind(id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;
        let mut tiers_by_line = decode_tier_rows(raw_tiers)?;

        let mut lines = Vec::new();
        for r in &db_lines {
            let line_id: Uuid = r.get("id");
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

            let mut line = DraftLine::new(c, buy, sell, reason);
            line.tiers = tiers_by_line.remove(&line_id);
            lines.push(line);
        }

        Ok(Some(RateDraft {
            id: *id,
            status,
            version,
            lines,
        }))
    }
}
