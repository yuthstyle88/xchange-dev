// #6010: #cash-policy lists the branch's traded currencies, with an add-currency select; the blank-row note says what
// the code does (finding #564 issuecomment-6580287046; ruling on issuecomment-6580564909: "the rule does not change").
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - Rows: the branch's `admin_cash.stock[].code` in answer order, then currencies added on the form, then any currency
//   the policy names that is not already listed. Each row keeps `input[data-currency="<CUR>"]`.
// - `select[name="addCurrency"]`: first option value '' (`addCurrencyPick`), then the register currencies
//   (`admin_cash.currencies`) not shown as rows, in register order. Choosing one adds an empty row, resets the select
//   to '' and drops the code from the options. No select when nothing is left to offer.
// - Saving sends only rows with a value, as today. A branch switch rebuilds the rows and drops unsaved added rows.
// - `emptyNote` th 'ช่องที่เว้นว่าง: ทุกการโอนสกุลนี้ต้องให้ผู้บริหารอนุมัติ', en 'A blank row: every transfer in that currency
//   needs the executive' (transfer_policy.rs:141: a blank currency is never within the limit).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminCashData, makeAdminCashStockRow, makeCashTransferPolicyData } from '../../../shared/testing/fixtures';
import { Toast } from '../parts/toast';
import { CashPolicy } from './CashPolicy';

const NOTE_TH = 'ช่องที่เว้นว่าง: ทุกการโอนสกุลนี้ต้องให้ผู้บริหารอนุมัติ';
const NOTE_EN = 'A blank row: every transfer in that currency needs the executive';
const OLD_NOTE_TH = 'ช่องที่เว้นว่างไม่มีวงเงิน';

/** A register as long as the real one: THB first, then many codes. */
const REGISTER = ['THB', 'AED', 'AUD', 'CHF', 'CNY', 'EUR', 'GBP', 'HKD', 'JPY', 'KRW', 'SGD', 'USD'];

function cash(stock: string[], currencies: string[] = REGISTER) {
  return makeAdminCashData({
    openShifts: [],
    stock: stock.map((code) => makeAdminCashStockRow({ code })),
    stats: null,
    currencies,
    lastCounted: null,
  });
}

type Reply = { status: number; body: unknown };

class Server {
  cashByBranch: Record<string, Reply> = { MAIN: { status: 200, body: cash(['THB', 'USD', 'EUR']) } };
  policy: unknown = null;
  puts: { body: string }[] = [];

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const u = new URL(String(input), 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method === 'GET' && u.pathname === '/api/v3/views/admin_cash') {
      const r = this.cashByBranch[u.searchParams.get('branch') ?? ''] ?? { status: 404, body: {} };
      return r.status === 200 ? view(r.body) : json(r.status, r.body);
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/cash_transfer_policy') {
      return json(200, { changed: true, versions: {}, data: this.policy });
    }
    if (method === 'PUT' && u.pathname === '/api/cash-transfer-policy') {
      this.puts.push({ body: String(init?.body ?? '') });
      return json(200, makeCashTransferPolicyData({ version: 'v2', limits: JSON.parse(String(init?.body)).limits }));
    }
    return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let server: Server;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

async function mount() {
  const [branch, setBranch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
  const [shown] = createSignal(true);
  dispose = render(
    () => (
      <>
        <CashPolicy branch={branch} period={period} shown={shown} />
        <Toast />
      </>
    ),
    host,
  );
  await vi.waitFor(() => expect(rows().length).toBeGreaterThan(0));
  return { setBranch };
}

const rows = () => [...host.querySelectorAll<HTMLInputElement>('input[data-currency]')].map((i) => i.dataset.currency);
const input = (cur: string) => host.querySelector<HTMLInputElement>(`input[data-currency="${cur}"]`);
const addSelect = () => host.querySelector<HTMLSelectElement>('select[name="addCurrency"]');
const offered = () => [...addSelect()!.options].map((o) => o.value);

function choose(cur: string) {
  const s = addSelect()!;
  s.value = cur;
  s.dispatchEvent(new Event('change', { bubbles: true }));
  s.dispatchEvent(new Event('input', { bubbles: true }));
}

function type(cur: string, value: string) {
  const i = input(cur)!;
  i.value = value;
  i.dispatchEvent(new Event('input', { bubbles: true }));
}

async function save() {
  host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();
  await vi.waitFor(() => expect(server.puts.length).toBeGreaterThan(0));
}

describe('#6010 #cash-policy rows are the branch traded currencies', () => {
  it('T1 the rows are the stock codes in answer order, not one per register code', async () => {
    await mount();
    expect(rows()).toEqual(['THB', 'USD', 'EUR']);
  });

  it('T2 a currency the policy names but the branch does not hold comes after the stock rows, with its limit', async () => {
    server.policy = makeCashTransferPolicyData({ version: 'v1', limits: { JPY: '90000.00', USD: '500.00' } });
    await mount();
    await vi.waitFor(() => expect(rows()).toEqual(['THB', 'USD', 'EUR', 'JPY']));
    expect(input('JPY')!.value).toBe('90000.00');
    expect(input('USD')!.value).toBe('500.00');
  });

  it('T3 the select offers the rest of the register in order; adding GBP gives an empty row and drops it from the options', async () => {
    await mount();
    expect(offered()).toEqual(['', ...REGISTER.filter((c) => !['THB', 'USD', 'EUR'].includes(c))]);
    choose('GBP');
    await vi.waitFor(() => expect(rows()).toEqual(['THB', 'USD', 'EUR', 'GBP']));
    expect(input('GBP')!.value).toBe('');
    expect(addSelect()!.value).toBe('');
    expect(offered()).not.toContain('GBP');
    expect(server.puts, 'nothing is sent until save').toHaveLength(0);
  });

  it('T4 a value typed in an added row is saved', async () => {
    await mount();
    choose('GBP');
    await vi.waitFor(() => expect(input('GBP')).not.toBeNull());
    type('GBP', '500');
    type('THB', '20000.00');
    await save();
    expect(JSON.parse(server.puts[0]!.body)).toEqual({ limits: { THB: '20000.00', GBP: '500' } });
  });

  it('T5 an added row left blank is not sent', async () => {
    await mount();
    choose('CNY');
    await vi.waitFor(() => expect(input('CNY')).not.toBeNull());
    type('THB', '20000.00');
    await save();
    expect(JSON.parse(server.puts[0]!.body)).toEqual({ limits: { THB: '20000.00' } });
  });

  it('T6 a branch switch rebuilds the rows from the new branch stock and drops unsaved added rows', async () => {
    server.cashByBranch.B02 = { status: 200, body: cash(['THB', 'JPY']) };
    const { setBranch } = await mount();
    choose('GBP');
    await vi.waitFor(() => expect(rows()).toContain('GBP'));
    setBranch('B02');
    await vi.waitFor(() => expect(rows()).toEqual(['THB', 'JPY']));
    setBranch('MAIN');
    await vi.waitFor(() => expect(rows()).toEqual(['THB', 'USD', 'EUR']));
  });

  it('T7 the admin_cash read fails: the policy currencies only, and no add select', async () => {
    server.cashByBranch.MAIN = { status: 500, body: { error: { code: 'serverError', message: 'x' } } };
    server.policy = makeCashTransferPolicyData({ version: 'v1', limits: { THB: '20000.00' } });
    dispose = undefined;
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
    dispose = render(() => <CashPolicy branch={branch} period={period} shown={() => true} />, host);
    await vi.waitFor(() => expect(rows()).toEqual(['THB']));
    expect(addSelect()).toBeNull();
  });

  it('T8 every register currency is already a row: no add select', async () => {
    server.cashByBranch.MAIN = { status: 200, body: cash(['THB', 'USD'], ['THB', 'USD']) };
    await mount();
    expect(rows()).toEqual(['THB', 'USD']);
    expect(addSelect()).toBeNull();
  });

  it('a policy currency that the branch holds shows once, in its stock position', async () => {
    server.policy = makeCashTransferPolicyData({ version: 'v1', limits: { EUR: '100.00' } });
    await mount();
    await vi.waitFor(() => expect(input('EUR')!.value).toBe('100.00'));
    expect(rows()).toEqual(['THB', 'USD', 'EUR']);
  });

  it('T9 the note says a blank row sends every transfer to the executive (TH and EN)', async () => {
    await mount();
    expect(host.textContent).toContain(NOTE_TH);
    expect(host.textContent).not.toContain(OLD_NOTE_TH);
    setLang('en');
    await vi.waitFor(() => expect(host.textContent).toContain(NOTE_EN));
  });
});
