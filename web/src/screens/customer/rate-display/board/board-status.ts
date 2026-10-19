// P01 v2 board status (rate-display.md §9.5, §9.6): synced / delayed / offline from the rates poller.
import type { PublicPollState } from '../../../../shared/live/public-poller';
import { BOARD_TEXT } from './board-texts';

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

export function boardStatus(input: BoardStatusInput): BoardStatusView {
  if (input.lastOkAt === undefined) {
    return {
      status: 'offline',
      ratesVisible: false,
      emptyMessage: input.pollerState === 'loading' ? BOARD_TEXT.loading : BOARD_TEXT.askCounter,
    };
  }

  if (input.now - input.lastOkAt > HIDE_AFTER_MS) {
    return {
      status: 'offline',
      ratesVisible: false,
      emptyMessage: BOARD_TEXT.askCounter,
    };
  }

  return {
    status: input.pollerState === 'error' ? 'delayed' : 'synced',
    ratesVisible: true,
    emptyMessage: input.lineCount === 0 ? BOARD_TEXT.noCurrencies : null,
  };
}

