//! Publication service: publishing approved drafts into immutable publications.

use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, Side, UnitSize};
use xc_notify::application::notification_service::NotificationService;
use xc_notify::application::template_registry::TemplateRegistry;

use crate::application::publication::sharp_move_notices::SharpMoveNotices;
use crate::domain::currency::denomination_groups::GroupCode;
use crate::domain::draft::rate_draft::{DraftId, DraftLine, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::draft::rate_proposal::RateProposal;
use crate::domain::draft::tier_proposal::TierProposal;
use crate::domain::policy::policy_tiers::PolicyTiers;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::publication::publication::PublicationId;
use crate::domain::publication::published_rates::PublicationRef;
use crate::domain::publication::sharp_move::{ComparablePrice, SharpMove};
use crate::domain::reference::reference_batch::ReferenceLine;
use crate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;

pub struct PublishService {
    _pool: PgPool,
    notices: SharpMoveNotices,
}

impl PublishService {
    pub fn new(pool: PgPool) -> Self {
        let notices = SharpMoveNotices::new(Arc::new(NotificationService::new(
            pool.clone(),
            Arc::new(TemplateRegistry::generated()),
        )));
        Self {
            _pool: pool,
            notices,
        }
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

        let mut policy_tiers_cache: std::collections::HashMap<Uuid, Option<PolicyTiers>> =
            std::collections::HashMap::new();
        type CachedPolicyRow = (
            RatePolicy,
            Option<i32>,
            Decimal,
            Decimal,
            Option<RatePercent>,
        );
        let mut policy_cache: std::collections::HashMap<Uuid, Option<CachedPolicyRow>> =
            std::collections::HashMap::new();
        let mut line_thresholds: std::collections::HashMap<CurrencyCode, Option<RatePercent>> =
            std::collections::HashMap::new();

        let ref_ids: Vec<Uuid> = db_lines
            .iter()
            .filter_map(|r| r.get::<Option<Uuid>, _>("reference_line_id"))
            .collect();
        let ref_rows = sqlx::query(
            "SELECT l.id, l.currency_code, l.unit_size, l.buy, l.sell, b.fetched_at, b.source_date \
             FROM rate_reference_lines l JOIN rate_reference_batches b ON b.id = l.batch_id \
             WHERE l.id = ANY($1)",
        )
        .bind(&ref_ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        type LoadedReference = (
            ReferenceLine,
            chrono::DateTime<chrono::Utc>,
            chrono::NaiveDate,
        );
        let mut references_by_id: std::collections::HashMap<Uuid, LoadedReference> =
            std::collections::HashMap::with_capacity(ref_rows.len());
        for rr in ref_rows {
            let rid: Uuid = rr.get(0);
            let code_s: String = rr.get(1);
            let curr = CurrencyCode::parse(code_s.trim())
                .map_err(|e| RateEngineError::Internal(format!("invalid currency code: {e}")))?;
            let u_dec: Decimal = rr.get(2);
            let u = UnitSize::parse(&u_dec.to_string())
                .map_err(|e| RateEngineError::Internal(e.to_string()))?;
            let b_dec: Option<Decimal> = rr.get(3);
            let s_dec: Option<Decimal> = rr.get(4);
            let fat: chrono::DateTime<chrono::Utc> = rr.get(5);
            let sdate: chrono::NaiveDate = rr.get(6);
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
            references_by_id.insert(
                rid,
                (ReferenceLine::new(curr, u, b_rate, s_rate), fat, sdate),
            );
        }

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

            let policy_tiers = if let Some(pid) = pol_id {
                if let Some(cached) = policy_tiers_cache.get(&pid) {
                    cached.clone()
                } else {
                    let loaded = PolicyTiersStore::for_version(tx, &pid).await?;
                    policy_tiers_cache.insert(pid, loaded.clone());
                    loaded
                }
            } else {
                None
            };

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

            // Load policy if any (cached per policy_version_id)
            let (policy, max_ref_age, pol_buy_pct, pol_sell_pct) = if let Some(pid) = pol_id {
                let cached_pol = if let Some(cached) = policy_cache.get(&pid) {
                    cached.clone()
                } else {
                    let pol_row = sqlx::query(
                        "SELECT id::text, buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours, sharp_move_alert_pct \
                         FROM rate_policy_versions WHERE id = $1",
                    )
                    .bind(pid)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

                    let loaded = if let Some(pr) = pol_row {
                        let pid_str: String = pr.get(0);
                        let buy_pct: Decimal = pr.get(1);
                        let sell_pct: Decimal = pr.get(2);
                        let guards: serde_json::Value = pr.get(3);
                        let age: Option<i32> = pr.get(4);
                        let alert_thresh: Option<Decimal> = pr.get(5);
                        let thresh =
                            alert_thresh.and_then(|d| RatePercent::parse(&d.to_string()).ok());
                        let buy_pct_rate = RatePercent::parse(&buy_pct.to_string())
                            .map_err(|e| RateEngineError::Internal(e.to_string()))?;
                        let sell_pct_rate = RatePercent::parse(&sell_pct.to_string())
                            .map_err(|e| RateEngineError::Internal(e.to_string()))?;
                        Some((
                            RatePolicy::new(pid_str, buy_pct_rate, sell_pct_rate, &guards)?,
                            age,
                            buy_pct,
                            sell_pct,
                            thresh,
                        ))
                    } else {
                        None
                    };
                    policy_cache.insert(pid, loaded.clone());
                    loaded
                };

                if let Some((pol, age, buy_pct, sell_pct, thresh)) = cached_pol {
                    line_thresholds.insert(c, thresh);
                    (Some(pol), age, Some(buy_pct), Some(sell_pct))
                } else {
                    line_thresholds.insert(c, None);
                    (None, None, None, None)
                }
            } else {
                line_thresholds.insert(c, None);
                (None, None, None, None)
            };

            // Look up preloaded reference if any
            let (reference, fetched_at, source_date) =
                match ref_id.and_then(|rf| references_by_id.get(&rf).cloned()) {
                    Some((ref_line, fat, sdate)) => (Some(ref_line), Some(fat), Some(sdate)),
                    None => (None, None, None),
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

            if let Some(pt) = policy_tiers {
                let tier_lines = TierProposal::of(
                    &prop,
                    &line,
                    reference.as_ref(),
                    policy.as_ref(),
                    &pt,
                    &units,
                )?;

                for (idx, tl) in tier_lines.into_iter().enumerate() {
                    let tier_no = (idx + 1) as u8;
                    let buy_margin = pt.margin(tier_no, Side::Buy);
                    let sell_margin = pt.margin(tier_no, Side::Sell);

                    let mut tier_snapshot = snapshot.clone();
                    tier_snapshot["tier"] = json!({
                        "no": tier_no,
                        "buy_margin_pct": buy_margin.to_string(),
                        "sell_margin_pct": sell_margin.to_string(),
                    });

                    let lower_dec = tl
                        .band
                        .lower_exclusive_thb
                        .map(|m| Decimal::from_str_exact(&m.to_string()).unwrap());
                    let upper_dec = tl
                        .band
                        .upper_inclusive_thb
                        .map(|m| Decimal::from_str_exact(&m.to_string()).unwrap());

                    sqlx::query(
                        "INSERT INTO published_rate_lines \
                         (id, publication_id, currency_code, denomination_group, unit_size, \
                          buy_rate, sell_rate, source_snapshot, value_tier, \
                          tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
                          base_buy_rate, base_sell_rate) \
                         VALUES (gen_random_uuid(), $1, $2, 'ALL', $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                    )
                    .bind(pub_id.as_uuid())
                    .bind(c.as_str())
                    .bind(unit_val)
                    .bind(tl.buy.map(|r| r.value()))
                    .bind(tl.sell.map(|r| r.value()))
                    .bind(&tier_snapshot)
                    .bind(tl.code.as_str())
                    .bind(lower_dec)
                    .bind(upper_dec)
                    .bind(tl.base_buy.map(|r| r.value()))
                    .bind(tl.base_sell.map(|r| r.value()))
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;
                }
            } else {
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
        }

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(pub_id.as_uuid())
            .bind(branch_str)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // Sharp-move check and notices (SM-2 #1584, rates.md §12.8, §12.10)
        let moves = if let Some(old_pub_id) = current_pub_id {
            let old_prices = load_comparable_prices(tx, &old_pub_id).await?;
            let new_prices = load_comparable_prices(tx, &pub_id.as_uuid()).await?;

            let mut all_moves = Vec::new();
            for (curr, thresh) in line_thresholds {
                if thresh.is_none() {
                    continue;
                }
                let old_curr: Vec<_> = old_prices
                    .iter()
                    .filter(|p| p.currency == curr)
                    .cloned()
                    .collect();
                let new_curr: Vec<_> = new_prices
                    .iter()
                    .filter(|p| p.currency == curr)
                    .cloned()
                    .collect();
                let curr_moves = SharpMove::between(&old_curr, &new_curr, thresh);
                all_moves.extend(curr_moves);
            }
            all_moves
        } else {
            Vec::new()
        };

        self.notices.notify_in(tx, branch, &moves).await?;

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

async fn load_comparable_prices(
    tx: &mut Transaction<'_, Postgres>,
    pub_id: &Uuid,
) -> Result<Vec<ComparablePrice>, RateEngineError> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (currency_code, denomination_group) \
                currency_code, denomination_group, \
                COALESCE(base_buy_rate, buy_rate) AS buy, \
                COALESCE(base_sell_rate, sell_rate) AS sell \
         FROM published_rate_lines \
         WHERE publication_id = $1 \
         ORDER BY currency_code, denomination_group, value_tier",
    )
    .bind(pub_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| RateEngineError::Database(e.to_string()))?;

    let mut prices = Vec::new();
    for r in rows {
        let code_str: String = r.get("currency_code");
        let group_str: String = r.get("denomination_group");
        let buy_dec: Option<Decimal> = r.get("buy");
        let sell_dec: Option<Decimal> = r.get("sell");

        let curr = CurrencyCode::parse(code_str.trim())
            .map_err(|e| RateEngineError::Internal(format!("invalid currency: {e}")))?;
        let grp = GroupCode::new(group_str.trim())
            .map_err(|e| RateEngineError::Internal(format!("invalid group: {e}")))?;

        let buy_rate = match buy_dec {
            Some(d) => Some(
                Rate::parse(&d.to_string())
                    .map_err(|e| RateEngineError::Internal(e.to_string()))?,
            ),
            None => None,
        };
        let sell_rate = match sell_dec {
            Some(d) => Some(
                Rate::parse(&d.to_string())
                    .map_err(|e| RateEngineError::Internal(e.to_string()))?,
            ),
            None => None,
        };

        prices.push(ComparablePrice {
            currency: curr,
            group: grp.clone(),
            side: Side::Buy,
            rate: buy_rate,
        });
        prices.push(ComparablePrice {
            currency: curr,
            group: grp,
            side: Side::Sell,
            rate: sell_rate,
        });
    }

    Ok(prices)
}
