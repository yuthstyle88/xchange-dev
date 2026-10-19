// Ticket #868: the LIVE transaction CSV has one builder, and it carries the row's unit.
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Decided: the owner, 2026-10-10 ("เพิ่มหน่วยด้วย"). Design: rates.md §4.9 R1 (a row's rate is per
// its unit since #781). v2's 20 headers (`admin-html:170`) stay as they are, with `Unit Size`
// inserted after `Rate`.
//
// Signatures this test assumes ([required], #868 §7):
// - `web/src/screens/admin/parts/live-csv.ts` exports `LIVE_CSV_HEADERS: string[]` and
//   `liveCsvRow(branchId: string, branchName: string, r: TransactionRow): string[]`;
// - `Transactions.tsx`, `Overview.tsx` and `Reports.tsx` declare no CSV headers of their own and
//   build their rows with `liveCsvRow`.

import { describe, expect, it } from 'vitest';
import { makeTransactionRow, type TransactionRow } from '../../../shared/testing/fixtures';

type LiveCsv = {
  LIVE_CSV_HEADERS: string[];
  liveCsvRow: (branchId: string, branchName: string, r: TransactionRow) => string[];
};

const modules = import.meta.glob<LiveCsv>('./live-csv.ts', { eager: true });
const screens = import.meta.glob<string>(
  ['../transactions/Transactions.tsx', '../overview/Overview.tsx', '../reports/Reports.tsx'],
  { query: '?raw', import: 'default', eager: true },
);

function liveCsv(): LiveCsv {
  const m = modules['./live-csv.ts'];
  expect(m, 'web/src/screens/admin/parts/live-csv.ts exists (#868 §7)').toBeDefined();
  return m!;
}

const HEADERS = [
  'Branch ID',
  'Branch',
  'Mode',
  'Document Ref',
  'Date',
  'Type',
  'Currency',
  'Amount',
  'Rate',
  'Unit Size',
  'Principal THB',
  'Fee THB',
  'Bank Fee THB',
  'Settlement THB',
  'Method',
  'Status',
  'Shift ID',
  'Reference Rate',
  'Reference Value THB',
  'Reference Spread THB',
  'Reference Spread %',
];

function row(over: Partial<TransactionRow> = {}) {
  return makeTransactionRow({
    id: 'XC-20261010-000001',
    date: '2026-10-10',
    created: '2026-10-10T03:04:05.000Z',
    type: 'BUY',
    currency: 'KRW',
    amount: '100000.00',
    rate: '25.12345678',
    unitSize: '1000',
    principal: '2512.35',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '2512.35',
    method: 'Cash',
    customer: 'Somchai',
    customerId: 'CU-1',
    staff: 'front',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: '25.00000000',
    referenceValue: '2500.00',
    referenceSpread: '-12.35',
    referencePercent: '-0.494',
    paymentRef: null,
    slipUrl: null,
    ...over,
  });
}

describe('#868 the LIVE transaction CSV', () => {
  it("has v2's 20 headers with Unit Size right after Rate", () => {
    expect(liveCsv().LIVE_CSV_HEADERS).toEqual(HEADERS);
  });

  it('a KRW row per 1000 carries its 8-place rates and its unit, cell for cell', () => {
    expect(liveCsv().liveCsvRow('MAIN', 'Silom #2', row())).toEqual([
      'MAIN', 'Silom #2', 'LIVE', 'XC-20261010-000001', '2026-10-10', 'BUY', 'KRW', '100000.00',
      '25.12345678', '1000', '2512.35', '0.00', '0.00', '2512.35', 'Cash', 'COMPLETED', 'SH-1',
      '25.00000000', '2500.00', '-12.35', '-0.494',
    ]);
  });

  it('the unit is the row\'s own, not a constant: JPY per 100 and USD per 1', () => {
    const { liveCsvRow } = liveCsv();
    const at = HEADERS.indexOf('Unit Size');
    expect(liveCsvRow('MAIN', 'M', row({ currency: 'JPY', unitSize: '100' }))[at]).toBe('100');
    expect(liveCsvRow('MAIN', 'M', row({ currency: 'USD', unitSize: '1' }))[at]).toBe('1');
  });

  it('a row with no reference and no shift leaves those cells empty, as v2 did', () => {
    const cells = liveCsv().liveCsvRow(
      'B1',
      'Ari',
      row({ shiftId: null, referenceRate: null, referenceValue: null, referenceSpread: null, referencePercent: null }),
    );
    expect(cells).toHaveLength(HEADERS.length);
    expect(cells.slice(HEADERS.indexOf('Shift ID'))).toEqual(['', '', '', '', '']);
  });

  it('the three screens use the one builder and keep no headers of their own', () => {
    for (const [path, src] of Object.entries(screens)) {
      expect(src, `${path} imports the shared builder`).toMatch(/from '\.\.\/parts\/live-csv'/);
      expect(src, `${path} calls liveCsvRow`).toContain('liveCsvRow(');
      expect(src, `${path} declares no LIVE headers of its own`).not.toContain("'Reference Spread %'");
    }
    expect(Object.keys(screens)).toHaveLength(3);
  });
});
