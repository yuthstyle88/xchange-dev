// The admin transaction dialog and table say when an exchange was reversed and name who served it.
//
// Seen 2026-11-20 while writing docs/USER_MANUAL.md: a reversed SELL showed `สำเร็จ`, and `พนักงาน` showed a
// user UUID. The original row stays `COMPLETED` (docs/design/blueprint/reversal.md V-2); the server now
// sends `reversedAt` (the POSTED reversal's time) and `staffName` (users.name for `staff`).
//
// | Row | สถานะ | พนักงาน | refund-check button |
// |---|---|---|---|
// | COMPLETED, reversedAt set, staffName set | สำเร็จ · กลับรายการแล้ว | the name | none |
// | COMPLETED, not reversed, staffName null | สำเร็จ | the staff id | shown, disabled |
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import type { ApiClient, ViewAnswer } from '../../../shared/api/client';
import { makeTransactionRow, type TransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { TransactionDetail } from './TransactionDetail';
import { TransactionTable } from './TransactionTable';

const USER_ID = 'fa7742d3-5a24-442e-b64e-098b92d7acfb';
const REFUND_CHECK = 'รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล';

function row(over: Partial<TransactionRow> = {}) {
  return makeTransactionRow({
    id: 'XC-20261120-000002',
    date: '2026-11-20',
    created: '2026-11-20T02:59:00.000Z',
    type: 'SELL',
    currency: 'USD',
    amount: '50.00',
    rate: '34.20',
    unitSize: '1',
    principal: '1710.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '1710.00',
    method: 'Cash',
    customer: 'WALK-IN',
    staff: USER_ID,
    staffName: null,
    status: 'COMPLETED',
    reversedAt: null,
    ...over,
  });
}

const api = {
  view: () => new Promise<ViewAnswer<unknown>>(() => {}),
  page: async () => {
    throw new Error('no page');
  },
  post: async () => {
    throw new Error('no write');
  },
} as unknown as ApiClient;

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

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
  setLang('th');
});

function field(el: HTMLElement, label: string): string {
  const dt = [...el.querySelectorAll('dt')].find((d) => d.textContent === label);
  expect(dt, `${label} is a field`).toBeDefined();
  return (dt!.nextElementSibling?.textContent ?? '').trim();
}

function refundButtons(el: HTMLElement): HTMLButtonElement[] {
  return [...el.querySelectorAll('button')].filter(
    (b) => (b.textContent ?? '').trim() === REFUND_CHECK,
  ) as HTMLButtonElement[];
}

describe('TransactionDetail — reversed and staff name', () => {
  it('a reversed sale reads สำเร็จ · กลับรายการแล้ว, names the staff member, and offers no refund check', () => {
    const el = mount(() => (
      <TransactionDetail
        api={api}
        branch="MAIN"
        row={row({ reversedAt: '2026-11-20T03:10:00.000Z', staffName: 'Teller Demo' })}
        onClose={() => {}}
      />
    ));
    expect(field(el, 'สถานะ')).toBe('สำเร็จ · กลับรายการแล้ว');
    expect(field(el, 'พนักงาน')).toBe('Teller Demo');
    expect(refundButtons(el)).toEqual([]);
  });

  it('a sale that was not reversed reads สำเร็จ; with no name the staff id is shown', () => {
    const el = mount(() => <TransactionDetail api={api} branch="MAIN" row={row()} onClose={() => {}} />);
    expect(field(el, 'สถานะ')).toBe('สำเร็จ');
    expect(field(el, 'พนักงาน')).toBe(USER_ID);
    const refund = refundButtons(el);
    expect(refund.length).toBe(1);
    expect(refund[0]!.disabled).toBe(true);
  });

  it('English: Completed · Reversed', () => {
    setLang('en');
    const el = mount(() => (
      <TransactionDetail
        api={api}
        branch="MAIN"
        row={row({ reversedAt: '2026-11-20T03:10:00.000Z' })}
        onClose={() => {}}
      />
    ));
    expect(field(el, 'Status')).toBe('Completed · Reversed');
  });
});

describe('TransactionTable — reversed', () => {
  it('the status pill of a reversed sale says so', () => {
    const el = mount(() => (
      <TransactionTable rows={[row({ reversedAt: '2026-11-20T03:10:00.000Z' })]} onOpen={() => {}} />
    ));
    expect(el.textContent).toContain('สำเร็จ · กลับรายการแล้ว');
  });
});
