// Cash forms reuse CurrencyCount (owner screenshot 2026-11-04, xchange#564): the staff new-transfer and
// receive forms list only the currencies they need, and the user adds any other currency through the search box.
//
// Pre-added (locked, no remove button):
//   new transfer  THB only (no transfer kind implies another currency);
//   receive       THB, plus every currency in the transfer's lines, in line order.
// A currency that is not on the form sends no lines, as a blank field does today. Request bodies are unchanged.
//
// DOM contract (#5114 §4): rows keep `[data-denomination-row]` / `input[data-denomination-id]` and
// `[data-field-error="lines.<id>.qty"]`; CurrencyCount adds `currency-search`, `currency-option[data-currency]`,
// `currency-group[data-currency]`, `currency-remove`.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import {
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
} from '../../../shared/testing/fixtures';
import type { StaffCard, StaffCtx } from '../cards';
import { warmModules } from '../../../shared/testing/warm-modules';

const cardMods = import.meta.glob<{ card: StaffCard }>('./card.ts');

async function loadCard(): Promise<StaffCard> {
  const l = cardMods['./card.ts'];
  expect(l, 'web/src/screens/staff/transfers/card.ts exists').toBeDefined();
  return (await l!()).card;
}

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000b1';
const TR_ID = 'f0000000-0000-4000-8000-0000000000c1';

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '50.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 22, currency: 'USD', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 31, currency: 'EUR', faceValue: '20.0000', groupCode: 'NOTE' }),
];

// A dispatched transfer into this branch that carries USD only.
const DISPATCHED = makeCashTransferRow({
  id: TR_ID,
  kind: 'BRANCH',
  status: 'DISPATCHED',
  fromBranchId: 'B07',
  toBranchId: BRANCH,
  fromShiftId: null,
  toShiftId: null,
  createdBy: 'u-x',
  createdAt: '2026-11-04T03:00:00Z',
  note: 'usd-in',
  lines: [makeCashTransferLineRow({ currency: 'USD', denominationId: 22, qtySent: '3', qtyReceived: null })],
});

type Sent = { url: string; method: string; body: unknown };
let sent: Sent[];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stub(rows: unknown[] = []) {
  const all: Record<string, unknown> = {
    cash_transfers: rows,
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [makeTransferBranchRow({ branchId: 'B07', name: 'Phuket' })] },
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const name = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        if (name in all) return json(200, { version: 1, changed: true, data: all[name] });
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      sent.push({ url: path, method, body: JSON.parse(typeof init?.body === 'string' ? init.body : '{}') });
      if (path === '/api/cash-transfers') return json(200, { id: 'f0000000-0000-4000-8000-0000000000a1', status: 'DRAFT' });
      return json(200, { id: path.split('/')[3], status: 'DRAFT' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount() {
  const card = await loadCard();
  const m = makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions: ['CashManage'],
  });
  const floor = makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
  const ctx: StaffCtx = { me: m, floor, refresh: vi.fn(async () => {}) };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
}

const tid = (id: string) => `[data-testid="${id}"]`;
const q = <T extends Element = HTMLElement>(sel: string, root: ParentNode = host) => root.querySelector<T>(sel);
function must<T extends Element = HTMLElement>(sel: string, root: ParentNode = host): T {
  const el = q<T>(sel, root);
  expect(el, sel).not.toBeNull();
  return el!;
}
const group = (code: string) => `${tid('currency-group')}[data-currency="${code}"]`;
const option = (code: string) => `${tid('currency-option')}[data-currency="${code}"]`;
const qty = (id: number) => `input[data-denomination-id="${id}"]`;
const groupsOf = (form: Element) =>
  Array.from(form.querySelectorAll(tid('currency-group'))).map((g) => g.getAttribute('data-currency'));

function type(input: HTMLInputElement | HTMLSelectElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function openCreate(): Promise<HTMLFormElement> {
  must(tid('transfer-new')).click();
  await vi.waitFor(() => expect(q(`${tid('transfer-form')} ${qty(11)}`), 'THB inputs drawn').not.toBeNull());
  const form = must<HTMLFormElement>(tid('transfer-form'));
  type(must<HTMLSelectElement>(tid('transfer-kind'), form), 'EXTERNAL_IN');
  await settle();
  return form;
}

async function openReceive(): Promise<HTMLFormElement> {
  await vi.waitFor(() => expect(q(`[data-transfer-id="${TR_ID}"] ${tid('transfer-receive')}`)).not.toBeNull());
  must<HTMLButtonElement>(`[data-transfer-id="${TR_ID}"] ${tid('transfer-receive')}`).click();
  await vi.waitFor(() => expect(q(`${tid('transfer-receive-form')} ${qty(22)}`), 'USD inputs drawn').not.toBeNull());
  return must<HTMLFormElement>(tid('transfer-receive-form'));
}

async function addBySearch(form: Element, code: string) {
  type(must<HTMLInputElement>(tid('currency-search'), form), code.toLowerCase());
  await vi.waitFor(() => expect(q(option(code), form), option(code)).not.toBeNull());
  must<HTMLButtonElement>(option(code), form).click();
  await vi.waitFor(() => expect(q(group(code), form), group(code)).not.toBeNull());
}

beforeAll(
  () =>
    warmModules(
      () => import('./card'),
      () => import('../main'),
    ),
  60_000,
);

beforeEach(() => {
  sent = [];
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('cash forms reuse CurrencyCount: staff new-transfer and receive forms', () => {
  it('T1: the new-transfer form pre-adds THB only, locked, with a search box', async () => {
    stub();
    await mount();
    const form = await openCreate();

    expect(groupsOf(form)).toEqual(['THB']);
    expect(q(qty(21), form), 'USD not on the form until added').toBeNull();
    expect(q(qty(31), form), 'EUR not on the form until added').toBeNull();
    must<HTMLInputElement>(tid('currency-search'), form);
    expect(must(group('THB'), form).querySelector(tid('currency-remove')), 'THB is locked').toBeNull();
  });

  it('T2: adding USD by search, then saving, sends exactly the typed notes', async () => {
    stub();
    await mount();
    const form = await openCreate();
    await addBySearch(form, 'USD');

    type(must<HTMLInputElement>(qty(11), form), '2');
    type(must<HTMLInputElement>(qty(21), form), '4');
    must<HTMLButtonElement>(tid('transfer-save')).click();
    await vi.waitFor(() => expect(sent.some((s) => s.url === '/api/cash-transfers')).toBe(true));

    const created = sent.find((s) => s.url === '/api/cash-transfers')!.body as { kind: string; lines: unknown };
    expect(created.kind).toBe('EXTERNAL_IN');
    expect(created.lines).toEqual([
      { currency: 'THB', denominationId: 11, qty: '2' },
      { currency: 'USD', denominationId: 21, qty: '4' },
    ]);
  });

  it('T3: removing an added currency clears its notes and sends no line for it', async () => {
    stub();
    await mount();
    const form = await openCreate();
    await addBySearch(form, 'USD');

    type(must<HTMLInputElement>(qty(21), form), '4');
    must<HTMLButtonElement>(`${group('USD')} ${tid('currency-remove')}`, form).click();
    await vi.waitFor(() => expect(q(group('USD'), form)).toBeNull());
    type(must<HTMLInputElement>(qty(12), form), '7');
    must<HTMLButtonElement>(tid('transfer-save')).click();
    await vi.waitFor(() => expect(sent.some((s) => s.url === '/api/cash-transfers')).toBe(true));

    const created = sent.find((s) => s.url === '/api/cash-transfers')!.body as { lines: unknown };
    expect(created.lines).toEqual([{ currency: 'THB', denominationId: 12, qty: '7' }]);
  });

  it('T4: a quantity of 0 is refused on that note row and nothing is sent', async () => {
    stub();
    await mount();
    const form = await openCreate();
    await addBySearch(form, 'USD');

    type(must<HTMLInputElement>(qty(22), form), '0');
    must<HTMLButtonElement>(tid('transfer-save')).click();
    await settle();

    expect(sent.filter((s) => s.url === '/api/cash-transfers')).toHaveLength(0);
    const row = must(qty(22), form).closest('[data-denomination-row]');
    expect(row?.querySelector('[data-field-error="lines.22.qty"]'), 'error on USD 100').not.toBeNull();
  });

  it("T5: the receive form pre-adds THB and the transfer's currencies, locked", async () => {
    stub([DISPATCHED]);
    await mount();
    const form = await openReceive();

    expect(groupsOf(form)).toEqual(['THB', 'USD']);
    expect(q(qty(31), form), 'EUR not on the form until added').toBeNull();
    expect(must(group('USD'), form).querySelector(tid('currency-remove')), 'USD is locked').toBeNull();
    must<HTMLInputElement>(tid('currency-search'), form);
  });

  it('T6: the receive body lists exactly the typed notes, as before', async () => {
    stub([DISPATCHED]);
    await mount();
    const form = await openReceive();

    type(must<HTMLInputElement>(qty(22), form), '3');
    must<HTMLButtonElement>(tid('transfer-receive-save')).click();
    const path = `/api/cash-transfers/${TR_ID}/receive`;
    await vi.waitFor(() => expect(sent.some((s) => s.url === path)).toBe(true));

    const body = sent.find((s) => s.url === path)!.body as { lines: unknown };
    expect(body.lines).toEqual([{ currency: 'USD', denominationId: 22, qtyReceived: '3' }]);
  });

  it('T7: English shows the same groups and search box', async () => {
    setLang('en');
    stub([DISPATCHED]);
    await mount();
    const create = await openCreate();
    expect(groupsOf(create)).toEqual(['THB']);
    await addBySearch(create, 'EUR');
    expect(q(qty(31), create)).not.toBeNull();
  });
});
