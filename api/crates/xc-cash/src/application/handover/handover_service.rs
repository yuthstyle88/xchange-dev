//! Handover application service (CA-4 #620; design §3.1, §5).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Role, ScopedRole, Target};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, QuantityDelta};
use xc_kernel::shift_hook::ShiftLifecycleHook;
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

fn handover_id(id: &str) -> AppResult<uuid::Uuid> {
    uuid::Uuid::parse_str(id).map_err(|_| AppError::NotFound {
        entity: "handover",
        id: id.to_string(),
    })
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

    pub fn with_shift_lifecycle_hook(self, hook: Arc<dyn ShiftLifecycleHook>) -> Self {
        Self {
            counts: self.counts.with_shift_lifecycle_hook(hook),
            ..self
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

    /// Q3: true when the user holds an OPEN shift of their own, or is the
    /// sender or the receiver of a PENDING hand-over in this organization.
    pub async fn holds_cash_in(
        tx: &mut sqlx::PgConnection,
        organization_id: &str,
        user_id: &str,
    ) -> AppResult<bool> {
        let user_id: Option<String> =
            sqlx::query_scalar("SELECT id::text FROM users WHERE id::text = $1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?;

        let Some(user_id) = user_id else {
            return Ok(false);
        };

        let has_open_shift: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN')",
        )
        .bind(&user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        if has_open_shift {
            return Ok(true);
        }

        let pending_handover: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 \
                 FROM handovers h \
                 LEFT JOIN shifts s ON s.id = h.from_shift_id \
                 LEFT JOIN cash_counts c ON c.id = h.count_id \
                 WHERE h.organization_id = $1::uuid \
                   AND h.status = 'PENDING' \
                   AND (s.teller_id = $2::uuid OR c.counted_by = $2::uuid OR h.receiver_id = $2::uuid) \
             )",
        )
        .bind(organization_id)
        .bind(&user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        Ok(pending_handover)
    }
}

pub const INVALID_RECEIVER: &str = "ผู้รับมอบกะไม่ถูกต้องหรือไม่มีสิทธิ์ปฏิบัติงานกะในสาขานี้";

/// A person who may receive a hand-over: their valid role rows at `now` that reach `branch` (#2846, #2879).
#[derive(Clone, Debug)]
pub struct ReceiverCandidate {
    pub user_id: String,
    pub name: String,
    pub roles: Vec<ScopedRole>,
}

/// One candidate role row: user, name, role code, branch, valid from, valid to.
type CandidateRole = (
    String,
    String,
    String,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

/// The one loader of hand-over receivers, used by `start_in` and the `handover_receivers` view: the ACTIVE users of
/// `organization_id` with role rows of that organisation valid at `now` (`valid_from <= now`, `valid_to` NULL or
/// `> now`) scoped to `branch` or organisation-wide. `only = Some(id)` narrows to one user. Role codes `Role::parse`
/// cannot read are skipped. Ordered by name, then user id. Who may receive is decided by [`may_receive`], never here.
pub async fn receiver_candidates<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    organization_id: &str,
    branch: &BranchId,
    only: Option<uuid::Uuid>,
    now: DateTime<Utc>,
) -> AppResult<Vec<ReceiverCandidate>> {
    let rows: Vec<CandidateRole> = sqlx::query_as(
        "SELECT u.id::text, COALESCE(u.name, ''), r.role_code, r.branch_id, r.valid_from, r.valid_to \
         FROM users u JOIN user_roles r ON r.user_id = u.id \
         WHERE u.status = 'ACTIVE' \
           AND u.organization_id::text = $1 AND r.organization_id = u.organization_id \
           AND (r.branch_id = $2 OR r.branch_id IS NULL) \
           AND r.valid_from <= $3 AND (r.valid_to IS NULL OR r.valid_to > $3) \
           AND ($4::uuid IS NULL OR u.id = $4)",
    )
    .bind(organization_id)
    .bind(branch.as_str())
    .bind(now)
    .bind(only)
    .fetch_all(ex)
    .await?;

    let mut people: std::collections::BTreeMap<String, ReceiverCandidate> =
        std::collections::BTreeMap::new();
    for (user_id, name, code, role_branch, from, to) in rows {
        let entry = people
            .entry(user_id.clone())
            .or_insert_with(|| ReceiverCandidate {
                user_id,
                name,
                roles: Vec::new(),
            });
        if let Some(role) = Role::parse(&code) {
            entry.roles.push(ScopedRole::new(
                role,
                role_branch.map(BranchId::new),
                from,
                to,
            ));
        }
    }
    let mut candidates: Vec<ReceiverCandidate> = people.into_values().collect();
    candidates.sort_by(|a, b| (&a.name, &a.user_id).cmp(&(&b.name, &b.user_id)));
    Ok(candidates)
}

/// The receiver rule (blueprint `cash-shift-transfer.md`): the Authorizer's answer to whether `c` holds
/// `ShiftOperate` at `branch` of `organization_id` at `now`. No role codes: policy lives in `RoleGrants::seed` (§11.2).
pub fn may_receive(
    c: &ReceiverCandidate,
    organization_id: &str,
    branch: &BranchId,
    now: DateTime<Utc>,
) -> bool {
    let user = Actor::new(&c.user_id, organization_id, c.roles.clone(), None);
    let at = Target::new(organization_id, Some(branch.clone()), None);
    Authorizer::check(Some(&user), Permission::ShiftOperate, &at, now).is_ok()
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

        let receiver_uuid = uuid::Uuid::parse_str(&cmd.receiver_id)
            .map_err(|_| AppError::Validation(INVALID_RECEIVER.to_owned()))?;
        let candidates = receiver_candidates(
            &mut **tx,
            &actor.organization_id,
            &cmd.branch,
            Some(receiver_uuid),
            now,
        )
        .await?;
        let valid_receiver = candidates
            .iter()
            .any(|c| may_receive(c, &actor.organization_id, &cmd.branch, now));
        if !valid_receiver {
            return Err(AppError::Validation(INVALID_RECEIVER.to_owned()));
        }

        if let Some(existing_count) = self
            .counts
            .repo()
            .find_count_by_idempotency_key(&mut *tx, &cmd.branch, &cmd.idempotency_key)
            .await?
        {
            PersonRules::replay_is_own(&existing_count.counted_by, actor)
                .map_err(|e| AppError::Conflict(e.to_string()))?;
            let handover_id: Option<String> =
                sqlx::query_scalar("SELECT id::text FROM handovers WHERE count_id = $1::uuid")
                    .bind(&existing_count.id)
                    .fetch_optional(&mut **tx)
                    .await?;

            if let Some(h_id) = handover_id {
                return self.load_in(tx, &h_id).await;
            }
        }

        let shift_teller: Option<String> = sqlx::query_scalar(
            "SELECT teller_id::text FROM shifts WHERE id = $1 AND status = 'OPEN' AND branch_id = $2",
        )
        .bind(&cmd.from_shift_id)
        .bind(cmd.branch.as_str())
        .fetch_optional(&mut **tx)
        .await?;
        let teller_id = shift_teller.ok_or(ShiftError::NoOpenShift)?;
        PersonRules::teller_owns_shift(&teller_id, actor)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        let mut totals_map: std::collections::BTreeMap<String, Decimal> =
            std::collections::BTreeMap::new();
        let mut sanitized_lines = Vec::with_capacity(cmd.lines.len());
        let mut seen_denominations: std::collections::HashSet<i64> =
            std::collections::HashSet::with_capacity(cmd.lines.len());

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
            if !seen_denominations.insert(line.denomination_id) {
                return Err(AppError::Validation(format!(
                    "duplicate denomination_id in handover lines: {}",
                    line.denomination_id
                )));
            }

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

            let max_scale = minor as u32;
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
        let handover_uuid = handover_id(id)?;
        let r: Option<Row> = sqlx::query_as(
            "SELECT id::text, branch_id, from_shift_id, to_shift_id, count_id::text, receiver_id::text, status \
             FROM handovers WHERE id = $1 FOR UPDATE",
        )
        .bind(handover_uuid)
        .fetch_optional(&mut **tx)
        .await?;

        let r = r.ok_or_else(|| AppError::NotFound {
            entity: "handover",
            id: id.to_string(),
        })?;

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
        Authorizer::check(
            Some(actor),
            Permission::ShiftOperate,
            &Target::new(&actor.organization_id, Some(h.branch_id.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

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
            "SELECT count(*) FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN' \
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
                let mut d = diff.normalize();
                if d.scale() < 2 {
                    d.rescale(2);
                }
                let qd = QuantityDelta::from_decimal(d)
                    .map_err(|e| AppError::Internal(format!("invalid handover diff {d}: {e}")))?;
                delta_pairs.push((code, qd));
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
        now: DateTime<Utc>,
    ) -> AppResult<Handover> {
        let handover_uuid = handover_id(id)?;
        let r: Option<Row> = sqlx::query_as(
            "SELECT id::text, branch_id, from_shift_id, to_shift_id, count_id::text, receiver_id::text, status \
             FROM handovers WHERE id = $1 FOR UPDATE",
        )
        .bind(handover_uuid)
        .fetch_optional(&mut **tx)
        .await?;

        let r = r.ok_or_else(|| AppError::NotFound {
            entity: "handover",
            id: id.to_string(),
        })?;

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
        Authorizer::check(
            Some(actor),
            Permission::ShiftOperate,
            &Target::new(&actor.organization_id, Some(h.branch_id.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

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
