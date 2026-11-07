// The cash-count rules both /staff (ShiftCard) and /desk (DeskShiftGate) read (docs/design/desk-shift-open.md §2).
// Lifted unchanged from ShiftCard.tsx (`isWaitingCount`, `countsToVerify`, `waitingOpenCount`, `waitingCloseCount`,
// `formatCountTime`) so the two pages decide "waiting" the same way.
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import { formatBangkokDateTime } from '../../../shared/format/bangkok-time';

/** `dd/mm/yyyy HH:mm` in Bangkok time; the input itself when it is not a date. */
export function formatCountTime(iso: string): string {
  return formatBangkokDateTime(iso);
}

/** WAITING and not yet verified. */
export function isWaitingCount(c: CashCountRow): boolean {
  return c.status === 'WAITING' && c.verifiedBy === null;
}

/** My OPEN count that waits for a manager. */
export function myWaitingOpenCount(rows: readonly CashCountRow[], userId: string): CashCountRow | undefined {
  return rows.find((c) => c.kind === 'OPEN' && c.countedBy === userId && isWaitingCount(c));
}

/** My CLOSE count of this shift that waits for a manager; none without a shift. */
export function myWaitingCloseCount(
  rows: readonly CashCountRow[],
  userId: string,
  shiftId: string | null | undefined,
): CashCountRow | undefined {
  if (!shiftId) return undefined;
  return rows.find(
    (c) => c.kind === 'CLOSE' && c.countedBy === userId && isWaitingCount(c) && c.shiftId === shiftId,
  );
}

/** Other people's OPEN and CLOSE counts that wait, for a certifier. */
export function countsToVerify(rows: readonly CashCountRow[], userId: string): CashCountRow[] {
  return rows.filter(
    (c) => isWaitingCount(c) && (c.kind === 'OPEN' || c.kind === 'CLOSE') && c.countedBy !== userId,
  );
}
