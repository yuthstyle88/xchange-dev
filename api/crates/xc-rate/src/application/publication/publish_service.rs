//! Publication service: publishing approved drafts into immutable publications.

use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, UnitSize};

use crate::domain::draft::rate_draft::{DraftId, DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::draft::rate_proposal::RateProposal;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::publication::publication::PublicationId;
use crate::domain::publication::published_rates::PublicationRef;
use crate::domain::reference::reference_batch::ReferenceLine;

pub struct PublishService {
    _pool: PgPool,
}

impl PublishService {
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }

    pub async fn publish(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &Actor,
        id: &DraftId,
        expected_version: Option<i32>,
    ) -> Result<PublicationRef, RateEngineError> {
        let branch_str = branch.as_str();

        // Lock branch row to prevent concurrent publish races
        let branch_row: Option<(Option<Uuid>, String, String)> = sqlx::query_as(
            "SELECT current_publication_id, price_source, organization_id::text FROM branches WHERE id = $1 FOR UPDATE",
        )
        .bind(branch_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (current_pub_id, price_source, org_id) = branch_row.ok_or_else(|| {
            RateEngineError::ValidationFailed(format!("branch not found: {branch_str}"))
        })?;

        if price_source == "RATE_BOARD" {
            return Err(RateEngineError::ValidationFailed(format!(
                "cannot publish BoT draft on branch {branch_str}: branch is configured for rate board"
            )));
        }

        let current_version: Option<i32> = if let Some(pub_id) = current_pub_id {
            let ver_row: Option<(i32,)> =
                sqlx::query_as("SELECT version FROM rate_publications WHERE id = $1")
                    .bind(pub_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;
            ver_row.map(|r| r.0)
        } else {
            None
        };

        if current_version != expected_version {
            return Err(RateEngineError::StaleVersion(format!(
                "expected version {expected_version:?}, got {current_version:?}"
            )));
        }

        // Lock and check draft
        let draft_row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT status, evaluation_hash FROM rate_drafts WHERE id = $1 AND branch_id = $2 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .bind(branch_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (status, eval_hash) =
            draft_row.ok_or_else(|| RateEngineError::NotFound(format!("draft not found: {id}")))?;

        if status != "APPROVED" {
            return Err(RateEngineError::ApprovalRequired(format!(
                "draft is in {status} status, must be APPROVED"
            )));
        }

        // Check if draft already published
        let already_pub: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM rate_publications WHERE draft_id = $1")
                .bind(id.as_uuid())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if already_pub.is_some() {
            return Err(RateEngineError::StaleVersion(
                "draft is already published".into(),
            ));
        }

        // Check approval request
        let req_row: Option<(Uuid, String, String)> = sqlx::query_as(
            "SELECT id, status, object_version_hash FROM approval_requests \
             WHERE type = 'RATE_PUBLICATION' AND object_id = $1 \
             ORDER BY requested_at DESC LIMIT 1",
        )
        .bind(id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (req_id, req_status, req_hash) = req_row.ok_or_else(|| {
            RateEngineError::ApprovalRequired("no approval request found for draft".into())
        })?;

        if req_status != "APPROVED" {
            return Err(RateEngineError::ApprovalRequired(
                "approval request is not APPROVED".into(),
            ));
        }

        if eval_hash.as_deref() != Some(&req_hash) {
            return Err(RateEngineError::ApprovalRequired(
                "evaluation hash moved".into(),
            ));
        }

        let next_version = current_version.unwrap_or(0) + 1;
        let pub_id = PublicationId::generate();

        sqlx::query(
            "INSERT INTO rate_publications \
             (id, branch_id, version, published_at, approved_request_id, draft_id, published_by) \
             VALUES ($1, $2, $3, now(), $4, $5, $6)",
        )
        .bind(pub_id.as_uuid())
        .bind(branch_str)
        .bind(next_version)
        .bind(req_id)
        .bind(id.as_uuid())
        .bind(who.user_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // Load draft lines
        let db_lines = sqlx::query(
            "SELECT l.currency_code, l.buy_mode, l.sell_mode, l.manual_buy, l.manual_sell, l.reason, \
                    l.reference_line_id, l.policy_version_id \
             FROM rate_draft_lines l \
             JOIN rate_drafts d ON d.id = l.draft_id \
             WHERE l.draft_id = $1 AND d.branch_id = $2 ORDER BY l.id",
        )
        .bind(id.as_uuid())
        .bind(branch_str)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

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

            let buy = match buy_mode.as_str() {
                "AUTO" => LineSide::Auto,
                _ => {
                    let mb = manual_buy.ok_or_else(|| {
                        RateEngineError::Internal("missing manual buy rate".to_string())
                    })?;
                    LineSide::Manual(
                        Rate::parse(&mb.to_string())
                            .map_err(|e| RateEngineError::Internal(e.to_string()))?,
                    )
                }
            };
            let sell = match sell_mode.as_str() {
                "AUTO" => LineSide::Auto,
                _ => {
                    let ms = manual_sell.ok_or_else(|| {
                        RateEngineError::Internal("missing manual sell rate".to_string())
                    })?;
                    LineSide::Manual(
                        Rate::parse(&ms.to_string())
                            .map_err(|e| RateEngineError::Internal(e.to_string()))?,
                    )
                }
            };
            let line = DraftLine::new(c, buy, sell, reason);

            // Load policy if any
            let (policy, max_ref_age, pol_buy_pct, pol_sell_pct) = if let Some(pid) = pol_id {
                let pol_row = sqlx::query(
                    "SELECT id::text, buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours \
                     FROM rate_policy_versions WHERE id = $1",
                )
                .bind(pid)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

                if let Some(pr) = pol_row {
                    let pid_str: String = pr.get(0);
                    let buy_pct: Decimal = pr.get(1);
                    let sell_pct: Decimal = pr.get(2);
                    let guards: serde_json::Value = pr.get(3);
                    let age: Option<i32> = pr.get(4);
                    let buy_pct_rate = RatePercent::parse(&buy_pct.to_string())
                        .map_err(|e| RateEngineError::Internal(e.to_string()))?;
                    let sell_pct_rate = RatePercent::parse(&sell_pct.to_string())
                        .map_err(|e| RateEngineError::Internal(e.to_string()))?;
                    (
                        Some(RatePolicy::new(
                            pid_str,
                            buy_pct_rate,
                            sell_pct_rate,
                            &guards,
                        )?),
                        age,
                        Some(buy_pct),
                        Some(sell_pct),
                    )
                } else {
                    (None, None, None, None)
                }
            } else {
                (None, None, None, None)
            };

            // Load reference if any
            let (reference, fetched_at, source_date) = if let Some(rf) = ref_id {
                let ref_row = sqlx::query(
                    "SELECT l.currency_code, l.unit_size, l.buy, l.sell, b.fetched_at, b.source_date \
                     FROM rate_reference_lines l JOIN rate_reference_batches b ON b.id = l.batch_id \
                     WHERE l.id = $1",
                )
                .bind(rf)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

                if let Some(rr) = ref_row {
                    let code_s: String = rr.get(0);
                    let curr = CurrencyCode::parse(code_s.trim())
                        .map_err(|e| RateEngineError::Internal(e.to_string()))?;
                    let u_dec: Decimal = rr.get(1);
                    let u = UnitSize::parse(&u_dec.to_string())
                        .map_err(|e| RateEngineError::Internal(e.to_string()))?;
                    let b_dec: Option<Decimal> = rr.get(2);
                    let s_dec: Option<Decimal> = rr.get(3);
                    let fat: chrono::DateTime<chrono::Utc> = rr.get(4);
                    let sdate: chrono::NaiveDate = rr.get(5);
                    let b_rate = match b_dec {
                        Some(d) => Some(
                            Rate::parse(&d.to_string())
                                .map_err(|e| RateEngineError::Internal(e.to_string()))?,
                        ),
                        None => None,
                    };
                    let s_rate = match s_dec {
                        Some(d) => Some(
                            Rate::parse(&d.to_string())
                                .map_err(|e| RateEngineError::Internal(e.to_string()))?,
                        ),
                        None => None,
                    };
                    (
                        Some(ReferenceLine::new(curr, u, b_rate, s_rate)),
                        Some(fat),
                        Some(sdate),
                    )
                } else {
                    (None, None, None)
                }
            } else {
                (None, None, None)
            };

            // Currency unit size and minor unit
            let c_row = sqlx::query("SELECT unit_size, minor_unit FROM currencies WHERE code = $1")
                .bind(c.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (unit_val, minor_val) = match c_row {
                Some(cr) => (cr.get::<Decimal, _>(0), cr.get::<i16, _>(1) as u8),
                None => (Decimal::ONE, 2u8),
            };

            let u_size = UnitSize::parse(&unit_val.to_string())
                .map_err(|e| RateEngineError::Internal(e.to_string()))?;
            let m_unit =
                MinorUnit::new(minor_val).map_err(|e| RateEngineError::Internal(e.to_string()))?;
            let units = CurrencyUnits::new(c, u_size, m_unit);

            let prop = RateProposal::of(&line, reference.as_ref(), policy.as_ref(), &units)?;

            // rates.md §4.1 "Partner cost": the newest 'PARTNER' reference line of the currency
            // (by its batch's `fetched_at`), copied as it stands; `null` when there is none.
            // `name` and `cost` (the partner's sell per 1 unit, 8 places) are what the v2
            // readers show as `partnerName` / `partnerCost` (#604 ruling Q2).
            let partner_row = sqlx::query(
                "SELECT l.id::text, b.provider, l.unit_size, l.buy, l.sell, b.source_date, b.fetched_at \
                 FROM rate_reference_lines l JOIN rate_reference_batches b ON b.id = l.batch_id \
                 WHERE l.currency_code = $1 AND l.reference_type = 'PARTNER' \
                 ORDER BY b.fetched_at DESC, l.id DESC LIMIT 1",
            )
            .bind(c.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let partner = partner_row.map(|pr| {
                let line_id: String = pr.get(0);
                let provider: String = pr.get(1);
                let unit: Decimal = pr.get(2);
                let p_buy: Option<Decimal> = pr.get(3);
                let p_sell: Option<Decimal> = pr.get(4);
                let p_date: chrono::NaiveDate = pr.get(5);
                let p_fetched: chrono::DateTime<chrono::Utc> = pr.get(6);
                let text = |d: Decimal| d.normalize().to_string();
                let cost = p_sell
                    .filter(|_| !unit.is_zero())
                    .map(|sell| format!("{:.8}", (sell / unit).round_dp(8)));
                json!({
                    "provider": provider.clone(),
                    "reference_line_id": line_id,
                    "unit_size": text(unit),
                    "buy": p_buy.map(text),
                    "sell": p_sell.map(text),
                    "source_date": p_date.to_string(),
                    "fetched_at": p_fetched.to_rfc3339(),
                    "name": provider,
                    "cost": cost,
                })
            });

            let mut snapshot = json!({
                "currency": c.as_str(),
                "buy_mode": buy_mode,
                "sell_mode": sell_mode,
                "reason": line.reason,
                "fetched_at": fetched_at.map(|t| t.to_rfc3339()),
                "source_date": source_date.map(|d| d.to_string()),
                "max_reference_age_hours": max_ref_age,
                "reference_line_id": ref_id.map(|u| u.to_string()),
                "policy_version_id": pol_id.map(|u| u.to_string()),
                "buy_adjust_pct": pol_buy_pct,
                "sell_adjust_pct": pol_sell_pct,
            });

            if let Some(p) = partner {
                snapshot["partner"] = p;
            } else {
                snapshot["partner"] = serde_json::Value::Null;
            }

            sqlx::query(
                "INSERT INTO published_rate_lines \
                 (id, publication_id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
                 VALUES (gen_random_uuid(), $1, $2, 'ALL', $3, $4, $5, $6)",
            )
            .bind(pub_id.as_uuid())
            .bind(c.as_str())
            .bind(unit_val)
            .bind(prop.buy.map(|r| r.value()))
            .bind(prop.sell.map(|r| r.value()))
            .bind(&snapshot)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        }

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(pub_id.as_uuid())
            .bind(branch_str)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let payload = json!({
            "branchId": branch_str,
            "publicationId": pub_id.to_string(),
            "version": next_version,
        });
        xc_outbox::application::writer::OutboxWriter::enqueue_in(
            tx,
            &org_id,
            &crate::application::publication::alert_event::RATE_ALERTS_PUBLICATION,
            &format!("branch:{branch_str}"),
            &pub_id.to_string(),
            &payload,
        )
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        sqlx::query(
            "UPDATE rate_drafts SET status = 'PUBLISHED', updated_at = now() WHERE id = $1",
        )
        .bind(id.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        Ok(PublicationRef {
            id: pub_id,
            version: next_version,
        })
    }
}
