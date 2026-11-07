// F20b (docs/design/backdated-keying.md §15.4): the LIVE CSV that Transactions, Overview and Reports export marks a
// backdated sale in a last column `Backdated`: `YES` for a backdated row, empty for a live one. Written before the
// implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a file of your own.

import { describe, expect, it } from 'vitest';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { LIVE_CSV_HEADERS, liveCsvRow } from './live-csv';

describe('F20b: the LIVE CSV marks a backdated row', () => {
  it('has 22 headers, the last one Backdated, and the 21 before it unchanged', () => {
    expect(LIVE_CSV_HEADERS).toHaveLength(22);
    expect(LIVE_CSV_HEADERS[21]).toBe('Backdated');
    expect(LIVE_CSV_HEADERS[20]).toBe('Reference Spread %');
    expect(LIVE_CSV_HEADERS.indexOf('Backdated')).toBe(21);
  });

  it('a backdated row ends with YES', () => {
    const cells = liveCsvRow('MAIN', 'M', makeTransactionRow({ backdated: true }));
    expect(cells).toHaveLength(22);
    expect(cells[21]).toBe('YES');
  });

  it('a live row ends with an empty cell', () => {
    const cells = liveCsvRow('MAIN', 'M', makeTransactionRow({ backdated: false }));
    expect(cells).toHaveLength(22);
    expect(cells[21]).toBe('');
  });
});
