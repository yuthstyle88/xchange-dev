// #781: the transaction parts show a row's rate and reference rate through `formatRate` itself,
// with the row's own rate, unit and currency, and print its answer as is.
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design `docs/design/blueprint/rates.md` §3.3 (one web formatter, `formatRate` in
// `web/src/shared/ui/rate.ts`; no screen formats a rate itself) and §4.9 R1 (v3 views speak per
// unit). `TransactionRate.test.tsx` pins the literal strings with the real formatter; this file
// replaces the formatter with a marker, so a part that formats a rate by itself, passes another
// unit or currency, or reworks the formatter's answer, shows something other than the marker.
//
// No server is involved. Nothing here skips.
//
// Signatures and fields assumed:
// - [required] `TransactionRow.unitSize: string` (generated `transaction_row.ts`; the `unitSize`
//   below is an `npm run typecheck` error until it is regenerated);
// - `formatRate(rate: string, unitSize: number | string, code: string): string`, a named export of
//   `web/src/shared/ui/rate.ts`, imported by the parts from `../../../shared/ui/rate`;
// - `TransactionTable({ rows, onOpen })`, `TransactionDetail({ api, branch, row, onClose })` as
//   merged, the rate under the amount in `.rate`, the detail as `<dt>`/`<dd>` pairs.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import type { ApiClient, ViewAnswer } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { TransactionDetail } from './TransactionDetail';
import { TransactionTable } from './TransactionTable';

const calls = vi.hoisted((): Array<[string, string, string]> => []);

vi.mock('../../../shared/ui/rate', () => ({
  formatRate: (rate: string, unitSize: number | string, code: string): string => {
    calls.push([rate, `${unitSize}`, code]);
    return `<${rate}|${unitSize}|${code}>`;
  },
}));

function row(over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: 'RE4-KRW',
    date: '2026-10-06',
    created: '2026-10-06T03:04:05.000Z',
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
    customerId: 'CU-RE4-KRW',
    staff: 'owner',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: '25.00000000',
    referenceValue: '2500.00',
    referenceSpread: '-12.35',
    referencePercent: '-0.494000',
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
}

function quietApi(): ApiClient {
  return {
    view: () => new Promise<ViewAnswer<unknown>>(() => {}),
    page: async () => {
      throw new Error('the detail reads no page');
    },
    post: async () => {
      throw new Error('the detail writes nothing');
    },
  } as unknown as ApiClient;
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(fn, root);
  return root;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  calls.length = 0;
  setLang('th');
});

function field(el: HTMLElement, label: string): string | undefined {
  const dt = [...el.querySelectorAll('dt')].find((d) => (d.textContent ?? '').trim() === label);
  if (!dt) return undefined;
  const dd = dt.nextElementSibling;
  expect(dd?.tagName, `${label}: a dd follows`).toBe('DD');
  return (dd!.textContent ?? '').trim();
}

describe('#781 the parts print what formatRate answers, for the row they show', () => {
  it('TransactionTable: `@ ` then formatRate(rate, unitSize, currency) of each row', () => {
    const usd = row({ id: 'RE4-USD', currency: 'USD', rate: '32.10000000', unitSize: '1' });
    const el = mount(() => <TransactionTable rows={[row(), usd]} onOpen={() => {}} />);
    const rates = [...el.querySelectorAll('tbody tr .rate')].map((n) => (n.textContent ?? '').trim());
    expect(rates).toEqual(['@ <25.12345678|1000|KRW>', '@ <32.10000000|1|USD>']);
    expect(calls).toContainEqual(['25.12345678', '1000', 'KRW']);
    expect(calls).toContainEqual(['32.10000000', '1', 'USD']);
  });

  it('TransactionDetail: เรท and เรทอ้างอิง are formatRate of the row rate and reference rate, with its unit', () => {
    const el = mount(() => <TransactionDetail api={quietApi()} branch="MAIN" row={row()} onClose={() => {}} />);
    expect(field(el, th.detailRate)).toBe('<25.12345678|1000|KRW>');
    expect(field(el, th.detailRefRate)).toBe('<25.00000000|1000|KRW>');
    expect(calls).toContainEqual(['25.12345678', '1000', 'KRW']);
    expect(calls).toContainEqual(['25.00000000', '1000', 'KRW']);
  });

  it('TransactionDetail with no reference rate: the formatter is never asked about null', () => {
    const mid = row({
      id: 'RE4-MID',
      rate: '25.12500000',
      referenceRate: null,
      referenceValue: null,
      referenceSpread: null,
      referencePercent: null,
    });
    const el = mount(() => <TransactionDetail api={quietApi()} branch="MAIN" row={mid} onClose={() => {}} />);
    expect(field(el, th.detailRate)).toBe('<25.12500000|1000|KRW>');
    expect(field(el, th.detailRefRate)).toBeUndefined();
    for (const [rate] of calls) {
      expect(rate).not.toBe('null');
    }
  });
});
