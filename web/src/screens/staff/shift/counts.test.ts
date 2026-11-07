// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// docs/design/desk-shift-open.md §2: the count rules /staff and /desk share.
import { describe, expect, it } from 'vitest';
import { makeCashCountRow } from '../../../shared/testing/fixtures';
import {
  countsToVerify,
  formatCountTime,
  isWaitingCount,
  myWaitingCloseCount,
  myWaitingOpenCount,
} from './counts';

const ME = 'u-me';
const row = (over: Parameters<typeof makeCashCountRow>[0]) =>
  makeCashCountRow({ kind: 'OPEN', countedBy: ME, status: 'WAITING', verifiedBy: null, shiftId: null, ...over });

describe('counts', () => {
  it('C1 waiting means WAITING and not verified', () => {
    expect(isWaitingCount(row({}))).toBe(true);
    expect(isWaitingCount(row({ verifiedBy: 'u-mgr' }))).toBe(false);
    expect(isWaitingCount(row({ status: 'VERIFIED' }))).toBe(false);
  });

  it('C2 my waiting OPEN count: mine and OPEN only', () => {
    const rows = [row({ id: 'a', countedBy: 'u-other' }), row({ id: 'b', kind: 'CLOSE' }), row({ id: 'c' })];
    expect(myWaitingOpenCount(rows, ME)?.id).toBe('c');
    expect(myWaitingOpenCount([rows[0]!, rows[1]!], ME)).toBeUndefined();
  });

  it('C3 my waiting CLOSE count: this shift only, none without a shift', () => {
    const rows = [row({ id: 'a', kind: 'CLOSE', shiftId: 's-old' }), row({ id: 'b', kind: 'CLOSE', shiftId: 's-1' })];
    expect(myWaitingCloseCount(rows, ME, 's-1')?.id).toBe('b');
    expect(myWaitingCloseCount(rows, ME, null)).toBeUndefined();
  });

  it('C4 counts to verify: other people OPEN/CLOSE that wait', () => {
    const rows = [
      row({ id: 'mine' }),
      row({ id: 'theirs-open', countedBy: 'u-t' }),
      row({ id: 'theirs-close', countedBy: 'u-t', kind: 'CLOSE' }),
      row({ id: 'theirs-handover', countedBy: 'u-t', kind: 'HANDOVER_OUT' }),
      row({ id: 'theirs-done', countedBy: 'u-t', verifiedBy: 'u-m' }),
    ];
    expect(countsToVerify(rows, ME).map((c) => c.id)).toEqual(['theirs-open', 'theirs-close']);
  });

  it('C5 count time is dd/mm/yyyy HH:mm in Bangkok; a non-date comes back as is', () => {
    expect(formatCountTime('2026-10-11T04:30:00Z')).toBe('11/10/2026 11:30');
    expect(formatCountTime('soon')).toBe('soon');
  });
});
