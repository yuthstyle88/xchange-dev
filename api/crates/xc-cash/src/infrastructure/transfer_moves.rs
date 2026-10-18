//! Shared reader of moved `cash_transfers` as `StockMovement` rows (LG-6.r6 #1046, ruling A).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{FromRow, PgPool};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};
use xc_shift::domain::shift::entities::stock_movement::StockMovement;
use xc_shift::domain::shift::value_objects::stock_delta::Direction;

use crate::domain::transfer::cash_transfer::{TransferKind, TransferLine, TransferStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AmountSource {
    Sent,
    Received,
}

/// Maps a transfer's kind, status, and endpoints to the drawer-referenced direction, shift id, and
/// amount source for `branch`. Returns `None` when the transfer is not a move visible on `branch`.
fn move_for_branch<'a>(
    branch: &str,
    kind: TransferKind,
    status: TransferStatus,
    from_branch: &str,
    to_branch: Option<&str>,
    from_shift: Option<&'a str>,
    to_shift: Option<&'a str>,
) -> Option<(Direction, &'a str, AmountSource)> {
    if !matches!(
        status,
        TransferStatus::Dispatched | TransferStatus::Received | TransferStatus::Reconciled
    ) {
        return None;
    }
    match kind {
        TransferKind::DrawerToSafe if from_branch == branch => {
            Some((Direction::Out, from_shift.unwrap_or(""), AmountSource::Sent))
        }
        TransferKind::SafeToDrawer if from_branch == branch => {
            Some((Direction::In, to_shift.unwrap_or(""), AmountSource::Sent))
        }
        TransferKind::ExternalIn if from_branch == branch => {
            Some((Direction::In, "", AmountSource::Sent))
        }
        TransferKind::ExternalOut if from_branch == branch => {
            Some((Direction::Out, "", AmountSource::Sent))
        }
        TransferKind::Branch if from_branch == branch => {
            Some((Direction::Out, "", AmountSource::Sent))
        }
        TransferKind::Branch
            if to_branch == Some(branch)
                && matches!(
                    status,
                    TransferStatus::Received | TransferStatus::Reconciled
                ) =>
        {
            Some((Direction::In, "", AmountSource::Received))
        }
        _ => None,
    }
}

#[derive(FromRow)]
struct MoveLineRow {
    transfer_id: String,
    kind: String,
    from_branch_id: String,
    to_branch_id: Option<String>,
    from_shift_id: Option<String>,
    to_shift_id: Option<String>,
    status: String,
    dispatched_at: Option<DateTime<Utc>>,
    dispatched_by: Option<String>,
    note: Option<String>,
    currency: String,
    denomination_id: i64,
    face_value: Decimal,
    qty_sent: i32,
    qty_received: Option<i32>,
}

/// All moved `(transfer, currency)` rows of `branch`, oldest dispatched first.
pub async fn of_branch(pool: &PgPool, branch: &BranchId) -> AppResult<Vec<StockMovement>> {
    let rows: Vec<MoveLineRow> = sqlx::query_as(
        "SELECT t.id::text AS transfer_id, t.kind, t.from_branch_id, t.to_branch_id, \
                t.from_shift_id, t.to_shift_id, t.status, t.dispatched_at, \
                t.dispatched_by::text AS dispatched_by, t.note, \
                l.currency, l.denomination_id, d.face_value, l.qty_sent, l.qty_received \
         FROM cash_transfers t \
         JOIN cash_transfer_lines l ON l.transfer_id = t.id \
         JOIN denominations d ON d.id = l.denomination_id \
         WHERE (t.from_branch_id = $1 OR t.to_branch_id = $1) \
           AND t.status IN ('DISPATCHED', 'RECEIVED', 'RECONCILED') \
         ORDER BY t.dispatched_at ASC, t.id::text ASC, l.currency ASC, l.denomination_id ASC",
    )
    .bind(branch.as_str())
    .fetch_all(pool)
    .await?;

    let mut movements: Vec<StockMovement> = Vec::new();
    let mut current_minor: i64 = 0;

    for row in rows {
        let kind = TransferKind::parse(&row.kind).ok_or_else(|| {
            AppError::Internal(format!(
                "cash_transfer {} has unknown kind {:?}",
                row.transfer_id, row.kind
            ))
        })?;
        let status = TransferStatus::parse(&row.status).ok_or_else(|| {
            AppError::Internal(format!(
                "cash_transfer {} has unknown status {:?}",
                row.transfer_id, row.status
            ))
        })?;
        let Some((direction, shift_id, amount_source)) = move_for_branch(
            branch.as_str(),
            kind,
            status,
            &row.from_branch_id,
            row.to_branch_id.as_deref(),
            row.from_shift_id.as_deref(),
            row.to_shift_id.as_deref(),
        ) else {
            continue;
        };

        let currency = CurrencyCode::parse(row.currency.trim()).map_err(|e| {
            AppError::Internal(format!(
                "cash_transfer {} line has invalid currency {:?}: {e}",
                row.transfer_id, row.currency
            ))
        })?;
        let face_value = Money::parse(&row.face_value.to_string()).map_err(|e| {
            AppError::Internal(format!(
                "invalid face_value {} for denomination {}: {e}",
                row.face_value, row.denomination_id
            ))
        })?;
        let line = TransferLine {
            currency,
            denomination_id: row.denomination_id,
            face_value,
            qty_sent: row.qty_sent,
            qty_received: row.qty_received,
        };
        let line_minor = match amount_source {
            AmountSource::Sent => line.sent_minor(),
            AmountSource::Received => line.received_minor(),
        }
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let id = format!("{}:{currency}", row.transfer_id);
        if let Some(last) = movements.last_mut()
            && last.id == id
        {
            current_minor = current_minor.checked_add(line_minor).ok_or_else(|| {
                AppError::Internal("transfer currency total overflows i64".into())
            })?;
            last.amount =
                Money::from_minor(current_minor).map_err(|e| AppError::Internal(e.to_string()))?;
            continue;
        }

        let at = row.dispatched_at.ok_or_else(|| {
            AppError::Internal(format!(
                "cash_transfer {} is {} without dispatched_at",
                row.transfer_id, row.status
            ))
        })?;
        let by = row.dispatched_by.ok_or_else(|| {
            AppError::Internal(format!(
                "cash_transfer {} is {} without dispatched_by",
                row.transfer_id, row.status
            ))
        })?;
        current_minor = line_minor;
        let amount =
            Money::from_minor(current_minor).map_err(|e| AppError::Internal(e.to_string()))?;
        movements.push(StockMovement {
            id,
            shift_id: shift_id.to_owned(),
            currency,
            direction,
            amount,
            reason: row.note.unwrap_or_default(),
            request_id: String::new(),
            at,
            by,
        });
    }

    Ok(movements)
}

/// Moved `(transfer, currency)` rows belonging to `shift` on `branch`.
pub async fn of_shift(
    pool: &PgPool,
    branch: &BranchId,
    shift: &str,
) -> AppResult<Vec<StockMovement>> {
    if shift.is_empty() {
        return Ok(Vec::new());
    }
    let moves = of_branch(pool, branch).await?;
    Ok(moves.into_iter().filter(|m| m.shift_id == shift).collect())
}

/// Keyset page of moved `(transfer, currency)` rows of `branch`, newest dispatched first.
/// An unknown `before` id (or one belonging to another branch) answers an empty page.
pub async fn page(
    pool: &PgPool,
    branch: &BranchId,
    before: Option<&str>,
    limit: u16,
) -> AppResult<Vec<StockMovement>> {
    let mut moves = of_branch(pool, branch).await?;
    moves.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
    let take = usize::from(limit);
    match before {
        Some(before_id) => {
            let Some(pos) = moves.iter().position(|m| m.id == before_id) else {
                return Ok(Vec::new());
            };
            Ok(moves.into_iter().skip(pos + 1).take(take).collect())
        }
        None => Ok(moves.into_iter().take(take).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawer_and_external_directions_and_branch_transit_visibility() {
        assert_eq!(
            move_for_branch(
                "MAIN",
                TransferKind::DrawerToSafe,
                TransferStatus::Dispatched,
                "MAIN",
                None,
                Some("SH-1"),
                None,
            ),
            Some((Direction::Out, "SH-1", AmountSource::Sent))
        );
        assert_eq!(
            move_for_branch(
                "MAIN",
                TransferKind::SafeToDrawer,
                TransferStatus::Received,
                "MAIN",
                None,
                None,
                Some("SH-1"),
            ),
            Some((Direction::In, "SH-1", AmountSource::Sent))
        );
        assert_eq!(
            move_for_branch(
                "MAIN",
                TransferKind::ExternalIn,
                TransferStatus::Received,
                "MAIN",
                None,
                None,
                None,
            ),
            Some((Direction::In, "", AmountSource::Sent))
        );
        assert_eq!(
            move_for_branch(
                "MAIN",
                TransferKind::ExternalOut,
                TransferStatus::Reconciled,
                "MAIN",
                None,
                None,
                None,
            ),
            Some((Direction::Out, "", AmountSource::Sent))
        );
        for unmoved in [TransferStatus::Draft, TransferStatus::Approved] {
            assert_eq!(
                move_for_branch(
                    "MAIN",
                    TransferKind::DrawerToSafe,
                    unmoved,
                    "MAIN",
                    None,
                    Some("SH-1"),
                    None,
                ),
                None
            );
        }
        // BRANCH: OUT on sender from DISPATCHED; hidden on receiver until RECEIVED, then IN with received amount
        assert_eq!(
            move_for_branch(
                "MAIN",
                TransferKind::Branch,
                TransferStatus::Dispatched,
                "MAIN",
                Some("B9"),
                None,
                None,
            ),
            Some((Direction::Out, "", AmountSource::Sent))
        );
        assert_eq!(
            move_for_branch(
                "B9",
                TransferKind::Branch,
                TransferStatus::Dispatched,
                "MAIN",
                Some("B9"),
                None,
                None,
            ),
            None
        );
        assert_eq!(
            move_for_branch(
                "B9",
                TransferKind::Branch,
                TransferStatus::Received,
                "MAIN",
                Some("B9"),
                None,
                None,
            ),
            Some((Direction::In, "", AmountSource::Received))
        );
    }
}
