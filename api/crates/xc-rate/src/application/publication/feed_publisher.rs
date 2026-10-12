//! Rate feed publisher: publishes board batch per branch (§2, §3.3; R-2, R-4, R-5, R-6, R-9).

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::currency::denomination_groups::DenominationGroups;
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::publication::publication::PublicationId;
use crate::domain::publication::published_rates::PublicationRef;
use crate::domain::publication::value_tier::{TierBand, TierCode};

#[derive(Clone)]
struct RefRow {
    currency_code: String,
    unit_size: Decimal,
    buy: Option<Decimal>,
    sell: Option<Decimal>,
    raw: Value,
    instrument_key: Option<String>,
    value_tier: Option<String>,
    tier_lower_exclusive_thb: Option<Decimal>,
    tier_upper_inclusive_thb: Option<Decimal>,
    base_buy: Option<Decimal>,
    base_sell: Option<Decimal>,
}

struct PreparedLine {
    currency_code: String,
    group_code: String,
    unit_size: Decimal,
    buy_rate: Option<Decimal>,
    sell_rate: Option<Decimal>,
    source_snapshot: Value,
    value_tier: String,
    tier_lower_exclusive_thb: Option<Decimal>,
    tier_upper_inclusive_thb: Option<Decimal>,
    base_buy_rate: Option<Decimal>,
    base_sell_rate: Option<Decimal>,
}

pub struct FeedPublisher {
    _pool: PgPool,
    groups: Arc<dyn DenominationGroups>,
    approvals: Arc<ApprovalService>,
}

impl FeedPublisher {
    pub fn new(
        pool: PgPool,
        groups: Arc<dyn DenominationGroups>,
        approvals: Arc<ApprovalService>,
    ) -> Self {
        Self {
            _pool: pool,
            groups,
            approvals,
        }
    }

    pub async fn publish_latest(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        provider: &str,
    ) -> Result<Option<PublicationRef>, RateEngineError> {
        let branch_str = branch.as_str();

        // 1. Lock branches FOR UPDATE
        let branch_row: Option<(Option<Uuid>,)> =
            sqlx::query_as("SELECT current_publication_id FROM branches WHERE id = $1 FOR UPDATE")
                .bind(branch_str)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (current_pub_id,) = branch_row
            .ok_or_else(|| RateEngineError::NotFound(format!("branch not found: {branch_str}")))?;

        let (current_version, current_req_info, current_line_checksum): (
            Option<i32>,
            Option<(String, String, String)>,
            Option<String>,
        ) = if let Some(pub_id) = current_pub_id {
            let row: Option<(
                i32,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<Value>,
            )> = sqlx::query_as(
                "SELECT p.version, r.type, r.object_id, r.object_version_hash, \
                        (SELECT l.source_snapshot FROM published_rate_lines l WHERE l.publication_id = p.id LIMIT 1) \
                 FROM rate_publications p \
                 LEFT JOIN approval_requests r ON r.id = p.approved_request_id \
                 WHERE p.id = $1",
            )
            .bind(pub_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            if let Some((v, req_type, obj_id, hash, snap)) = row {
                let req_info = match (req_type, obj_id, hash) {
                    (Some(t), Some(o), Some(h)) => Some((t, o, h)),
                    _ => None,
                };
                let line_chk = snap
                    .as_ref()
                    .and_then(|s| s.get("batch_checksum"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());
                (Some(v), req_info, line_chk)
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        // Query newest COMPLETE batch for provider
        let batch_row: Option<(Uuid, String, DateTime<Utc>, Option<DateTime<Utc>>)> =
            sqlx::query_as(
                "SELECT id, raw_checksum, fetched_at, source_published_at \
                 FROM rate_reference_batches \
                 WHERE provider = $1 AND status = 'COMPLETE' \
                 ORDER BY COALESCE(source_published_at, fetched_at) DESC, fetched_at DESC \
                 LIMIT 1",
            )
            .bind(provider)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (batch_id, raw_checksum, fetched_at, source_published_at) =
            batch_row.ok_or_else(|| {
                RateEngineError::RateUnavailable(format!(
                    "no complete batch for provider {provider}"
                ))
            })?;

        // Deduplication: if same checksum or request as current publication, nothing to do
        if let Some((ref req_type, ref obj_id, ref hash)) = current_req_info {
            let expected_obj_id = format!("{branch}:{batch_id}");
            if req_type == "RATE_FEED_PUBLICATION"
                && (hash == &raw_checksum || obj_id == &expected_obj_id)
            {
                return Ok(None);
            }
        }
        if let Some(ref cur_chk) = current_line_checksum {
            if cur_chk == &raw_checksum {
                return Ok(None);
            }
        }

        // Query branch's current stale policy
        let policy_row: Option<(Uuid, Option<i32>)> = sqlx::query_as(
            "SELECT id, max_reference_age_hours \
             FROM rate_policy_versions \
             WHERE branch_id = $1 \
               AND valid_from <= now() \
               AND (valid_to IS NULL OR valid_to > now()) \
             ORDER BY version DESC \
             LIMIT 1",
        )
        .bind(branch_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // 2. Fetch reference rows for the batch
        let rows = sqlx::query(
            "SELECT currency_code, unit_size, buy, sell, raw, instrument_key, value_tier, \
                    tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy, base_sell \
             FROM rate_reference_lines \
             WHERE batch_id = $1",
        )
        .bind(batch_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut ref_rows = Vec::with_capacity(rows.len());
        for r in rows {
            ref_rows.push(RefRow {
                currency_code: r.get(0),
                unit_size: r.get(1),
                buy: r.get(2),
                sell: r.get(3),
                raw: r.get(4),
                instrument_key: r.get(5),
                value_tier: r.get(6),
                tier_lower_exclusive_thb: r.get(7),
                tier_upper_inclusive_thb: r.get(8),
                base_buy: r.get(9),
                base_sell: r.get(10),
            });
        }

        // Query currencies
        let currencies: Vec<(String, Decimal)> =
            sqlx::query_as("SELECT code, unit_size FROM currencies ORDER BY code ASC")
                .fetch_all(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let effective_fetched_at = source_published_at.unwrap_or(fetched_at);
        let mut lines_to_publish = Vec::new();

        for (curr_code_str, curr_unit_size) in currencies {
            let curr_code = match CurrencyCode::parse(curr_code_str.trim()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let groups = self.groups.groups_of(tx, curr_code).await?;
            for group in groups {
                let mapped_key = self
                    .groups
                    .source_of(tx, curr_code, &group, provider)
                    .await?;
                let instrument_key = match mapped_key {
                    Some(k) => k,
                    None => continue,
                };

                let mut matching_rows: Vec<&RefRow> = ref_rows
                    .iter()
                    .filter(|r| {
                        r.currency_code.trim() == curr_code_str.trim()
                            && r.instrument_key.as_deref() == Some(&instrument_key)
                    })
                    .collect();

                if matching_rows.is_empty() {
                    continue;
                }

                // Sort matching rows by lower bound (None first, then ascending)
                matching_rows.sort_by(|a, b| {
                    match (a.tier_lower_exclusive_thb, b.tier_lower_exclusive_thb) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (Some(x), Some(y)) => x.cmp(&y),
                    }
                });

                // Validate tier bands
                let mut bands = Vec::with_capacity(matching_rows.len());
                for r in &matching_rows {
                    let tier_name = r.value_tier.as_deref().unwrap_or("ALL");
                    let tier_code =
                        TierCode::new(tier_name).map_err(RateEngineError::ValidationFailed)?;
                    let lo = match r.tier_lower_exclusive_thb {
                        Some(d) => Some(Money::parse(&format!("{d:.2}")).map_err(|e| {
                            RateEngineError::ValidationFailed(format!("invalid lower band: {e}"))
                        })?),
                        None => None,
                    };
                    let hi = match r.tier_upper_inclusive_thb {
                        Some(d) => Some(Money::parse(&format!("{d:.2}")).map_err(|e| {
                            RateEngineError::ValidationFailed(format!("invalid upper band: {e}"))
                        })?),
                        None => None,
                    };
                    bands.push(TierBand {
                        code: tier_code,
                        lower_exclusive_thb: lo,
                        upper_inclusive_thb: hi,
                    });
                }

                if !validate_tier_bands(&bands) {
                    return Err(RateEngineError::ValidationFailed(format!(
                        "tier bands overlap or leave a gap for currency {curr_code_str} group {group}"
                    )));
                }

                for r in matching_rows {
                    let convert_rate = |rate: Option<Decimal>| -> Option<Decimal> {
                        rate.map(|val| {
                            if r.unit_size.is_zero() {
                                val
                            } else {
                                val * curr_unit_size / r.unit_size
                            }
                        })
                    };

                    let buy_rate = convert_rate(r.buy);
                    let sell_rate = convert_rate(r.sell);
                    let base_buy_rate = convert_rate(r.base_buy);
                    let base_sell_rate = convert_rate(r.base_sell);

                    let tier_str = r.value_tier.clone().unwrap_or_else(|| "ALL".to_string());

                    let mut snapshot = json!({
                        "provider": provider,
                        "batch_id": batch_id.to_string(),
                        "batch_checksum": raw_checksum,
                        "instrument_key": r.instrument_key,
                        "value_tier": r.value_tier,
                        "source_published_at": source_published_at.map(|t| t.to_rfc3339()),
                        "fetched_at": effective_fetched_at.to_rfc3339(),
                        "raw": r.raw,
                    });

                    if let Some(status) = r.raw.get("status") {
                        snapshot["status"] = status.clone();
                    }

                    if let Some((ref pol_id, max_age)) = policy_row {
                        snapshot["policy_version_id"] = json!(pol_id.to_string());
                        if let Some(age) = max_age {
                            snapshot["max_reference_age_hours"] = json!(age);
                        }
                    }

                    lines_to_publish.push(PreparedLine {
                        currency_code: curr_code_str.clone(),
                        group_code: group.to_string(),
                        unit_size: curr_unit_size,
                        buy_rate,
                        sell_rate,
                        source_snapshot: snapshot,
                        value_tier: tier_str,
                        tier_lower_exclusive_thb: r.tier_lower_exclusive_thb,
                        tier_upper_inclusive_thb: r.tier_upper_inclusive_thb,
                        base_buy_rate,
                        base_sell_rate,
                    });
                }
            }
        }

        // 3. Approval request as SYSTEM
        let system_id = format!("SYSTEM:rate-feed:{provider}");
        let system_actor = Actor::new(
            &system_id,
            "00000000-0000-0000-0000-000000000001",
            vec![ScopedRole::new(
                Role::Admin,
                Some(branch.clone()),
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
            None,
        );
        let object_id = format!("{branch}:{batch_id}");
        let idempotency_key = format!("{branch}:{raw_checksum}");
        let payload = json!({
            "branch": branch.as_str(),
            "batch_id": batch_id.to_string(),
            "provider": provider,
        });

        let req = self
            .approvals
            .request(
                tx,
                "RATE_FEED_PUBLICATION",
                &object_id,
                &payload,
                &system_actor,
                &idempotency_key,
            )
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        if req.status == "PENDING" {
            self.approvals
                .decide_self(tx, &req.id, &system_actor, "rate feed")
                .await
                .map_err(|e| RateEngineError::Internal(e.to_string()))?;
        }

        let request_uuid = Uuid::parse_str(req.id.as_str())
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        // 4. Insert rate_publications
        let next_version = current_version.unwrap_or(0) + 1;
        let pub_id = PublicationId::generate();

        sqlx::query(
            "INSERT INTO rate_publications \
             (id, branch_id, version, published_at, approved_request_id, draft_id, published_by) \
             VALUES ($1, $2, $3, now(), $4, NULL, $5)",
        )
        .bind(pub_id.as_uuid())
        .bind(branch_str)
        .bind(next_version)
        .bind(request_uuid)
        .bind(&system_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // Insert published lines
        for line in lines_to_publish {
            sqlx::query(
                "INSERT INTO published_rate_lines \
                 (id, publication_id, currency_code, denomination_group, unit_size, \
                  buy_rate, sell_rate, source_snapshot, value_tier, \
                  tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy_rate, base_sell_rate) \
                 VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .bind(pub_id.as_uuid())
            .bind(&line.currency_code)
            .bind(&line.group_code)
            .bind(line.unit_size)
            .bind(line.buy_rate)
            .bind(line.sell_rate)
            .bind(&line.source_snapshot)
            .bind(&line.value_tier)
            .bind(line.tier_lower_exclusive_thb)
            .bind(line.tier_upper_inclusive_thb)
            .bind(line.base_buy_rate)
            .bind(line.base_sell_rate)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        }

        // Update branches.current_publication_id
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(pub_id.as_uuid())
            .bind(branch_str)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        Ok(Some(PublicationRef {
            id: pub_id,
            version: next_version,
        }))
    }
}

fn validate_tier_bands(bands: &[TierBand]) -> bool {
    if bands.is_empty() {
        return false;
    }

    for b in bands {
        if let (Some(lo), Some(hi)) = (b.lower_exclusive_thb, b.upper_inclusive_thb) {
            if lo >= hi {
                return false;
            }
        }
    }

    let count_lower_none = bands
        .iter()
        .filter(|b| b.lower_exclusive_thb.is_none())
        .count();
    let count_upper_none = bands
        .iter()
        .filter(|b| b.upper_inclusive_thb.is_none())
        .count();
    if count_lower_none != 1 || count_upper_none != 1 {
        return false;
    }

    let mut seen_codes = HashSet::new();
    for b in bands {
        if !seen_codes.insert(&b.code) {
            return false;
        }
    }

    let mut sorted = bands.to_vec();
    sorted.sort_by(
        |a, b| match (a.lower_exclusive_thb, b.lower_exclusive_thb) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(&y),
        },
    );

    if sorted[0].lower_exclusive_thb.is_some() {
        return false;
    }

    for i in 0..sorted.len() - 1 {
        let curr_upper = match sorted[i].upper_inclusive_thb {
            Some(u) => u,
            None => return false,
        };
        let next_lower = match sorted[i + 1].lower_exclusive_thb {
            Some(l) => l,
            None => return false,
        };
        if curr_upper != next_lower {
            return false;
        }
    }

    if sorted.last().unwrap().upper_inclusive_thb.is_some() {
        return false;
    }

    true
}
