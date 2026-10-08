// V2-parts (#476): the exchange table (ticket §7 `TransactionTable.tsx`, design §12.2, P1, P6).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// v2 `admin-html:63`, with the LIVE header of `:100` (`ลูกค้าทดลอง` → `ลูกค้า`): six columns —
// the id as a button that opens the detail, under it `date · HH:MM` (Bangkok time of `created`);
// the customer, under it the method's label; the type pill (`buy` / `sell`); the amount with its
// currency, under it `@ <rate>`; the settlement in THB; the status pill (`cancel` when
// CANCELLED). Columns 4 and 5 are numeric (class `num`).
//
// Figures (W7, P6): every figure is the server's string from `TransactionRow`. The rows below are
// chosen so that nothing the screen could compute matches: the settlement is not amount × rate ±
// fee, and one is a figure a JavaScript number cannot hold. The rate is shown as the server
// wrote it (v2's `Number(rate).toFixed(4)` is arithmetic the screen no longer does).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { TransactionTable } from './TransactionTable';

const THAI = /[฀-๿]/;
const HEADERS = ['รายการ / เวลา', 'ลูกค้า', 'ประเภท', 'จำนวนเงิน', 'ยอดชำระ THB', 'สถานะ'];

function row(over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: 'XC-B-2026-000001',
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '90071992547409.93',
    method: 'Cash',
    customer: 'Somchai Jaidee',
    customerId: 'CUS-1',
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
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
  setLang('th');
});

function cells(tr: Element): HTMLTableCellElement[] {
  return [...tr.querySelectorAll('td')];
}

describe('TransactionTable (#476 P1, P6)', () => {
  it("v2's six LIVE headers in order, the amount and settlement columns numeric", () => {
    const el = mount(() => <TransactionTable rows={[row()]} onOpen={() => {}} />);
    const ths = [...el.querySelectorAll('thead th')];
    expect(ths.map((th) => th.textContent)).toEqual(HEADERS);
    expect(ths.map((th) => th.classList.contains('num'))).toEqual([false, false, false, true, true, false]);
    expect(el.textContent).not.toContain('ลูกค้าทดลอง');
  });

  it('one body row per exchange, in the order given', () => {
    const rows = [row({ id: 'TX-3' }), row({ id: 'TX-1' }), row({ id: 'TX-2' })];
    const el = mount(() => <TransactionTable rows={rows} onOpen={() => {}} />);
    const ids = [...el.querySelectorAll('tbody tr')].map((tr) => tr.querySelector('button')?.textContent);
    expect(ids).toEqual(['TX-3', 'TX-1', 'TX-2']);
  });

  it('the first cell: the id as a button opening that exchange, and date · Bangkok time', () => {
    const onOpen = vi.fn();
    const el = mount(() => (
      <TransactionTable rows={[row({ id: 'TX-A' }), row({ id: 'TX-B', created: '2026-10-07T17:30:00.000Z', date: '2026-10-08' })]} onOpen={onOpen} />
    ));
    const [a, b] = [...el.querySelectorAll('tbody tr')];
    const first = cells(a!)[0]!;
    expect(first.textContent).toContain('2026-10-08 · 10:04');
    expect(cells(b!)[0]!.textContent).toContain('2026-10-08 · 00:30');
    (b!.querySelector('button') as HTMLButtonElement).click();
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen).toHaveBeenCalledWith('TX-B');
  });

  it("the customer with the method's label, and the type pill", () => {
    const el = mount(() => (
      <TransactionTable
        rows={[row({ type: 'BUY', method: 'PromptPay' }), row({ id: 'TX-2', type: 'SELL', method: 'Crypto', customer: 'Jane Roe' })]}
        onOpen={() => {}}
      />
    ));
    const [buy, sell] = [...el.querySelectorAll('tbody tr')];
    expect(cells(buy!)[1]!.textContent).toContain('Somchai Jaidee');
    expect(cells(buy!)[1]!.textContent).toContain('พร้อมเพย์');
    expect(cells(sell!)[1]!.textContent).toContain('Jane Roe');
    expect(cells(sell!)[1]!.textContent).toContain('อื่น ๆ');
    const buyPill = cells(buy!)[2]!.querySelector('.pill')!;
    const sellPill = cells(sell!)[2]!.querySelector('.pill')!;
    expect(buyPill.textContent).toBe('ร้านรับซื้อ');
    expect(buyPill.classList.contains('buy')).toBe(true);
    expect(sellPill.textContent).toBe('ร้านขาย');
    expect(sellPill.classList.contains('sell')).toBe(true);
  });

  it("figures are the server's strings: amount with currency, @ rate as written, settlement in the numeric cells", () => {
    const el = mount(() => (
      <TransactionTable rows={[row({ amount: '1500.5', currency: 'EUR', rate: '38.25', settlement: '90071992547409.93' })]} onOpen={() => {}} />
    ));
    const tds = cells(el.querySelector('tbody tr')!);
    expect(tds[3]!.classList.contains('num')).toBe(true);
    expect(tds[4]!.classList.contains('num')).toBe(true);
    expect(tds[3]!.textContent).toContain(`${formatMoney('1500.5')} EUR`);
    expect(tds[3]!.textContent).toContain('@ 38.25');
    expect(tds[3]!.textContent).not.toContain('38.2500');
    expect(tds[4]!.textContent).toContain('90,071,992,547,409.93');
  });

  it('the status pill: สำเร็จ, ยกเลิก (class cancel), รอบันทึกเอกสาร', () => {
    const el = mount(() => (
      <TransactionTable
        rows={[row({ id: 'A', status: 'COMPLETED' }), row({ id: 'B', status: 'CANCELLED' }), row({ id: 'C', status: 'PENDING' })]}
        onOpen={() => {}}
      />
    ));
    const pills = [...el.querySelectorAll('tbody tr')].map((tr) => cells(tr)[5]!.querySelector('.pill')!);
    expect(pills.map((p) => p.textContent)).toEqual(['สำเร็จ', 'ยกเลิก', 'รอบันทึกเอกสาร']);
    expect(pills.map((p) => p.classList.contains('cancel'))).toEqual([false, true, false]);
  });

  it("no rows: the table's empty state", () => {
    const el = mount(() => <TransactionTable rows={[]} onOpen={() => {}} />);
    expect(el.querySelector('table')).toBeNull();
    expect(el.textContent).toContain('ไม่พบรายการในช่วงที่เลือก');
  });

  it('in English the headers and labels switch; the figures keep their digits (W10)', () => {
    setLang('en');
    const el = mount(() => <TransactionTable rows={[row({ status: 'CANCELLED' })]} onOpen={() => {}} />);
    expect(THAI.test(el.textContent ?? '')).toBe(false);
    expect(el.textContent).toContain('90,071,992,547,409.93');
    expect(el.textContent).toContain('10:04');
  });
});
