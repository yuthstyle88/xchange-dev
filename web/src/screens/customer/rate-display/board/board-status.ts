// P01 v2 board status (rate-display.md §9.5, §9.6): synced / delayed / offline from the rates poller.
// SKELETON STUB (feat/fe-rateboard): RB-2 replaces `boardStatus`; the types and signature stay.
import type { PublicPollState } from '../../../../shared/live/public-poller';

/** Rates are hidden when the last successful update (a 200 or a 304) is older than this. */
export const HIDE_AFTER_MS = 30_000;

export interface BoardStatusInput {
  pollerState: PublicPollState;
  /** `PublicPoller.lastOkAt()`: the last successful update, ms; undefined before the first. */
  lastOkAt: number | undefined;
  now: number;
  /** The number of lines in the last answer; undefined before the first answer. */
  lineCount: number | undefined;
}

export interface BoardStatusView {
  status: 'synced' | 'delayed' | 'offline';
  /** False only when offline: the table is hidden. */
  ratesVisible: boolean;
  /** The text of the empty panel (one of BOARD_TEXT's loading / askCounter / noCurrencies), or null. */
  emptyMessage: string | null;
}

export function boardStatus(_input: BoardStatusInput): BoardStatusView {
  return { status: 'synced', ratesVisible: true, emptyMessage: null };
}
