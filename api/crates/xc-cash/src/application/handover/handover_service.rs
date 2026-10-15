//! Handover application service (CA-4 #620; design §3.1, §5).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::append_cash_exception_in;

use crate::application::count::CashCountService;
use crate::application::count::count_service::{held_of, post_opening_in};
use crate::domain::count::{CashCount, CountKind, CountLine};
use crate::domain::error::CashError;
use crate::domain::handover::{Handover, HandoverError, HandoverStatus};
use crate::domain::person_rules::PersonRules;
use crate::domain::posting::CashPlace;
use crate::domain::repository::{CashRepository, CountTotal};

pub struct StartHandover {
    pub branch: BranchId,
    pub from_shift_id: String,
    pub receiver_id: String,
    pub lines: Vec<CountLine>,
    pub note: Option<String>,
    pub idempotency_key: String,
}

pub struct HandoverService {
    pool: PgPool,
    counts: CashCountService,
}

fn dom(e: impl ToString) -> AppError {
    AppError::Domain(e.to_string())
}

type Row = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
);

impl HandoverService {
    pub fn pg(pool: PgPool) -> Self {
        Self {
            counts: CashCountService::pg(pool.clone()),
            pool,
        }
    }

    /// The hand-over as this transaction sees it (CA-7c: never through the pool).
    async fn load_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
    ) -> AppResult<Handover> {
        let r: Row = sqlx::query_as(
            "SELECT id::text, branch_id, from_shift_id, to_shift_id, count_id::text, receiver_id::text, status \
             FROM handovers WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
        Ok(Handover {
            id: r.0,
            branch_id: BranchId::new(r.1),
            from_shift_id: r.2,
            to_shift_id: r.3,
            count_id: r.4,
            receiver_id: r.5,
            status: HandoverStatus::parse(&r.6),
        })
    }

    pub async fn start(
        &self,
        actor: &Actor,
        cmd: StartHandover,
        now: DateTime<Utc>,
    ) -> AppResult<Handover> {
        let mut tx = self.pool.begin().await?;
        let handover = self.start_in(&mut tx, actor, cmd, now).await?;
        tx.commit().await?;
        Ok(handover)
    }

    pub async fn reject(&self, actor: &Actor, id: &str, now: DateTime<Utc>) -> AppResult<Handover> {
        let mut tx = self.pool.begin().await?;
        let handover = self.reject_in(&mut tx, actor, id, now).await?;
        tx.commit().await?;
        Ok(handover)
    }

    pub async fn accept(&self, actor: &Actor, id: &str, now: DateTime<Utc>) -> AppResult<Handover> {
        let mut tx = self.pool.begin().await?;
        let handover = self.accept_in(&mut tx, actor, id, now).await?;
        tx.commit().await?;
        Ok(handover)
    }
}

/// The CA-7c forms (design §4.9): all their work, reads included, on the caller's transaction. They open no
/// transaction and take no lock; the caller (the CM-1 seam, or the wrapper above) holds both.
impl HandoverService {
    pub async fn start_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &Actor,
        cmd: StartHandover,
        now: DateTime<Utc>,
    ) -> AppResult<Handover> {
        if cmd.receiver_id == actor.user_id {
            return Err(dom(CashError::SameCounter));
        }

        let permission = Permission::ShiftOperate;
        let target = Target::new(&actor.organization_id, Some(cmd.branch.clone()), None);
        Authorizer::check(Some(actor), permission, &target, now)
            .map_err(|r| AppError::Domain(CashError::Refused(r).to_string()))?;

        let shift_teller: Option<String> =
            sqlx::query_scalar("SELECT teller_id::text FROM shifts WHERE id = $1")
                .bind(&cmd.from_shift_id)
                .fetch_optional(&mut **tx)
                .await?;
        let teller_id = shift_teller.ok_or(ShiftError::NoOpenShift)?;
        PersonRules::teller_owns_shift(&teller_id, actor)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        if let Some(existing_count) = self
            .counts
            .repo()
            .find_count_by_idempotency_key(&mut *tx, &cmd.branch, &cmd.idempotency_key)
            .await?
        {
            let handover_id: Option<String> =
                sqlx::query_scalar("SELECT id::text FROM handovers WHERE count_id = $1::uuid")
                    .bind(&existing_count.id)
                    .fetch_optional(&mut **tx)
                    .await?;

            if let Some(h_id) = handover_id {
                return self.load_in(tx, &h_id).await;
            }
        }

        let mut totals_map: std::collections::BTreeMap<String, Decimal> =
            std::collections::BTreeMap::new();
        let mut sanitized_lines = Vec::with_capacity(cmd.lines.len());

        let denom_ids: Vec<i64> = cmd
            .lines
            .iter()
            .take_while(|l| l.qty >= 0)
            .map(|l| l.denomination_id)
            .collect();
        let denom_rows: Vec<(i64, Decimal, String)> = if denom_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as(
                "SELECT id, face_value, currency_code FROM denominations WHERE id = ANY($1)",
            )
            .bind(&denom_ids)
            .fetch_all(&mut **tx)
            .await?
        };
        let denoms: std::collections::HashMap<i64, (Decimal, String)> = denom_rows
            .into_iter()
            .map(|(id, face_value, currency_code)| (id, (face_value, currency_code)))
            .collect();

        for line in &cmd.lines {
            if line.qty < 0 {
                return Err(AppError::Validation(format!(
                    "line qty cannot be negative: {}",
                    line.qty
                )));
            }

            let &(face_value, ref denom_currency) = match denoms.get(&line.denomination_id) {
                Some(r) => r,
                None => {
                    return Err(AppError::NotFound {
                        entity: "denomination",
                        id: line.denomination_id.to_string(),
                    });
                }
            };

            let denom_currency = denom_currency.trim();
            let line_currency = line.currency.trim();
            if !denom_currency.eq_ignore_ascii_case(line_currency) {
                return Err(AppError::Validation(format!(
                    "line currency '{line_currency}' does not match denomination currency '{denom_currency}'"
                )));
            }

            let line_total = Decimal::from(line.qty) * face_value;
            *totals_map
                .entry(denom_currency.to_string())
                .or_insert(Decimal::ZERO) += line_total;

            sanitized_lines.push(CountLine {
                currency: denom_currency.to_string(),
                denomination_id: line.denomination_id,
                qty: line.qty,
            });
        }

        let currency_codes: Vec<String> = totals_map.keys().cloned().collect();
        let currency_rows: Vec<(String, i16)> = if currency_codes.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as("SELECT trim(code), minor_unit FROM currencies WHERE code = ANY($1)")
                .bind(&currency_codes)
                .fetch_all(&mut **tx)
                .await?
        };
        let minor_units: std::collections::HashMap<String, i16> =
            currency_rows.into_iter().collect();

        for (curr, counted) in &totals_map {
            let minor = minor_units.get(curr).copied().ok_or_else(|| {
                AppError::Internal(format!("currency not found in register: {curr}"))
            })?;

            let max_scale = (minor as u32).min(2);
            if counted.normalize().scale() > max_scale {
                return Err(CashError::TooManyDecimals.into());
            }
        }

        let held = self.counts.drawer_in(tx, &cmd.from_shift_id).await?;
        let mut totals = Vec::new();
        for (curr, counted) in totals_map {
            let exp = held_of(&held, &curr);

            totals.push(CountTotal {
                currency: curr,
                counted,
                expected: exp,
                difference: None,
            });
        }

        let count_id = uuid::Uuid::new_v4().to_string();
        let cash_count = CashCount {
            id: count_id.clone(),
            organization_id: actor.organization_id.clone(),
            branch_id: cmd.branch.clone(),
            shift_id: Some(cmd.from_shift_id.clone()),
            kind: CountKind::HandoverOut,
            counted_by: actor.user_id.clone(),
            counted_at: now,
            verified_by: None,
            verified_at: None,
            note: cmd.note,
            idempotency_key: cmd.idempotency_key,
            created_at: now,
            updated_at: now,
        };

        self.counts
            .repo()
            .insert_count(&mut *tx, &cash_count, &sanitized_lines, &totals)
            .await?;

        let id: String = sqlx::query_scalar(
            "INSERT INTO handovers (organization_id, branch_id, from_shift_id, count_id, receiver_id, status) \
             VALUES ($1::uuid, $2, $3, $4::uuid, $5::uuid, 'PENDING') \
             RETURNING id::text",
        )
        .bind(&actor.organization_id)
        .bind(cmd.branch.as_str())
        .bind(&cmd.from_shift_id)
        .bind(&count_id)
        .bind(&cmd.receiver_id)
        .fetch_one(&mut **tx)
        .await?;

        self.load_in(tx, &id).await
    }

    pub async fn accept_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &Actor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Handover> {
        let r: Row = sqlx::query_as(
            "SELECT id::text, branch_id, from_shift_id, to_shift_id, count_id::text, receiver_id::text, status \
             FROM handovers WHERE id = $1::uuid FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;

        let h = Handover {
            id: r.0,
            branch_id: BranchId::new(r.1),
            from_shift_id: r.2,
            to_shift_id: r.3,
            count_id: r.4,
            receiver_id: r.5,
            status: HandoverStatus::parse(&r.6),
        };

        if h.status != HandoverStatus::Pending {
            return Err(dom(HandoverError::NotPending));
        }

        PersonRules::receiver_verifies_handover(&h.receiver_id, actor).map_err(dom)?;

        let repo = self.counts.repo().clone();
        let mut totals = repo.get_count_totals(&mut *tx, &h.count_id).await?;
        let held = self.counts.drawer_in(tx, &h.from_shift_id).await?;
        for (code, exp) in &held {
            if !totals.iter().any(|t| t.currency.trim() == code.trim()) {
                totals.push(CountTotal {
                    currency: code.clone(),
                    counted: Decimal::ZERO,
                    expected: Some(*exp),
                    difference: None,
                });
            }
        }
        for t in &totals {
            let cur = held_of(&held, &t.currency);
            if cur != t.expected {
                return Err(dom(CashError::BalancesChanged));
            }
        }
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN' AND opened_by_count IS NOT NULL \
             AND id <> $2",
        )
        .bind(&h.receiver_id)
        .bind(&h.from_shift_id)
        .fetch_one(&mut **tx)
        .await?;
        if open > 0 {
            return Err(dom(CashError::ShiftAlreadyOpen));
        }
        let mut updated = Vec::new();
        let (mut counted, mut diffs, mut handed) = (Vec::new(), Vec::new(), Vec::new());
        let mut delta_pairs = Vec::new();
        for t in totals {
            let diff = t.counted - t.expected.unwrap_or(Decimal::ZERO);
            let code = CurrencyCode::parse(t.currency.trim()).map_err(dom)?;
            if !diff.is_zero() {
                let mut d = diff;
                d.rescale(2);
                delta_pairs.push((code, MoneyDelta::from_minor(d.mantissa())));
            }
            if !t.counted.is_zero() {
                counted.push((t.currency.clone(), t.counted));
                handed.push((code, t.counted));
            }
            diffs.push((t.currency.clone(), t.counted, diff));
            updated.push(CountTotal {
                difference: Some(diff),
                ..t
            });
        }
        if !delta_pairs.is_empty() {
            let exc = CashException {
                id: format!("EX-{}", uuid::Uuid::new_v4()),
                request_id: h.count_id.clone(),
                status: ExceptionStatus::Open,
                target_difference: None,
                cash_variance: Some(StockDelta::of(&delta_pairs)),
                safe_variance: None,
                note: format!("HANDOVER {}", h.id),
                at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
                reviewed_by: None,
                reviewed_at: None,
            };
            append_cash_exception_in(tx, &h.branch_id, &exc).await?;
        }
        let shifts = self.counts.shifts();
        shifts
            .close_on_verified(&mut *tx, &h.from_shift_id, &actor.user_id, &diffs, "", now)
            .await?;
        let b = shifts
            .open_on_verified(
                &mut *tx,
                &h.branch_id,
                &h.receiver_id,
                &h.count_id,
                &counted,
                now,
            )
            .await?;
        // §4.4 row 2: what the hand-over counted moves from the giver's drawer into the new one.
        post_opening_in(
            self.counts.postings().as_ref(),
            tx,
            CashPlace::Drawer(h.from_shift_id.clone()),
            &b.id,
            &handed,
            &h.id,
        )
        .await?;
        repo.update_count_verified(&mut *tx, &h.count_id, &actor.user_id, now, None, &updated)
            .await?;
        let res = sqlx::query(
            "UPDATE handovers SET status = 'ACCEPTED', to_shift_id = $2, updated_at = now() WHERE id = $1::uuid AND status = 'PENDING'",
        )
        .bind(id)
        .bind(&b.id)
        .execute(&mut **tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(dom(HandoverError::NotPending));
        }
        self.load_in(tx, id).await
    }

    pub async fn reject_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &Actor,
        id: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<Handover> {
        let r: Row = sqlx::query_as(
            "SELECT id::text, branch_id, from_shift_id, to_shift_id, count_id::text, receiver_id::text, status \
             FROM handovers WHERE id = $1::uuid FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;

        let h = Handover {
            id: r.0,
            branch_id: BranchId::new(r.1),
            from_shift_id: r.2,
            to_shift_id: r.3,
            count_id: r.4,
            receiver_id: r.5,
            status: HandoverStatus::parse(&r.6),
        };

        if h.status != HandoverStatus::Pending {
            return Err(dom(HandoverError::NotPending));
        }

        PersonRules::receiver_verifies_handover(&h.receiver_id, actor).map_err(dom)?;

        let res = sqlx::query(
            "UPDATE handovers SET status = 'REJECTED', updated_at = now() WHERE id = $1::uuid AND status = 'PENDING'",
        )
        .bind(id)
        .execute(&mut **tx)
        .await?;

        if res.rows_affected() == 0 {
            return Err(dom(HandoverError::NotPending));
        }

        self.load_in(tx, id).await
    }
}
