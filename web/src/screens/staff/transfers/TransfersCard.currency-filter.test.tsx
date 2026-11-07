import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import {
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeMyShift,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { warmModules } from '../../../shared/testing/warm-modules';
import { card } from './card';

const BRANCH = 'MAIN';
const ELSEWHERE = 'B09';
const SHIFT = 'SH-0b765d57-0000-4000-8000-000000000001';
const T_IN = 'f0000000-0000-4000-8000-000000000001';
const NEW_ID = 'f0000000-0000-4000-8000-0000000000aa';

const DENOMS = [
  makeDenominationRow({ id: 31, currency: 'EUR', faceValue: '20.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '0.25', groupCode: 'COIN' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 22, currency: 'USD', faceValue: '50.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 23, currency: 'USD', faceValue: '0.10', groupCode: 'COIN' }),
];

type Sent = { url: string; method: string; body: string | undefined };

let sent: Sent[];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stub(
  customDenoms = DENOMS,
  incomingLines = [] as ReturnType<typeof makeCashTransferLineRow>[],
) {
  const incoming = makeCashTransferRow({
    id: T_IN,
    kind: 'SAFE_TO_DRAWER',
    status: 'DISPATCHED',
    fromBranchId: BRANCH,
    toBranchId: BRANCH,
    fromShiftId: null,
    toShiftId: SHIFT,
    createdBy: '00000000-0000-4000-8000-0000000000f1',
    createdAt: '2026-11-05T08:45:00Z',
    note: 'Top up',
    lines: incomingLines,
  });

  const myShiftData = {
    ...makeMyShift({ id: SHIFT, status: 'OPEN', openedAt: '2026-11-05T01:05:00Z', balances: [] }),
    incomingTransfers: [incoming],
  };

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const name = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        if (name === 'my_shift') return json(200, { version: 1, changed: true, data: myShiftData });
        if (name === 'denominations') return json(200, { version: 1, changed: true, data: { rows: customDenoms } });
        if (name === 'cash_transfers') return json(200, { version: 1, changed: true, data: [incoming] });
        if (name === 'transfer_branches') {
          return json(200, {
            version: 1,
            changed: true,
            data: { rows: [makeTransferBranchRow({ branchId: ELSEWHERE, name: 'Ayutthaya' })] },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      sent.push({ url: path, method, body: init?.body as string | undefined });
      if (path === '/api/cash-transfers') return json(200, { id: NEW_ID, status: 'DRAFT' });
      return json(200, { id: NEW_ID, status: 'DRAFT' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(isCashManage = true) {
  const ctx: StaffCtx = {
    me: makeStaffMeView({
      userId: '00000000-0000-4000-8000-0000000000c1',
      name: 'Ploy',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions: isCashManage ? ['CashManage'] : ['ShiftOperate'],
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-11-05T01:00:00Z',
      myShift: makeStaffFloorMyShift({ id: SHIFT, openedAt: '2026-11-05T01:05:00Z' }),
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { ctx };
}

async function openCreate(): Promise<HTMLFormElement> {
  const btn = host.querySelector<HTMLButtonElement>('[data-testid="transfer-new"]');
  expect(btn).not.toBeNull();
  btn!.click();
  await settle();
  const form = host.querySelector<HTMLFormElement>('[data-testid="transfer-form"]');
  expect(form).not.toBeNull();
  return form!;
}

function typeInto(input: HTMLInputElement | HTMLSelectElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

beforeAll(() => warmModules(() => import('./card')), 60_000);

beforeEach(() => {
  sent = [];
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  host.remove();
  vi.restoreAllMocks();
});

describe('TransfersCard currency filter & totals (F49 / #6712)', () => {
  it('select[name="transferCurrency"] lists currencies present in denominations() with THB first when present', async () => {
    stub();
    await mount();
    const form = await openCreate();
    const select = form.querySelector<HTMLSelectElement>('select[name="transferCurrency"]');
    expect(select).not.toBeNull();
    const options = Array.from(select!.options).map((o) => o.value);
    // DENOMS has EUR, THB, USD. With THB first:
    expect(options).toEqual(['THB', 'EUR', 'USD']);
    expect(select!.value).toBe('THB');
  });

  it("only the chosen currency's rows are shown", async () => {
    stub();
    await mount();
    const form = await openCreate();
    const select = form.querySelector<HTMLSelectElement>('select[name="transferCurrency"]')!;

    const thbRow = form.querySelector<HTMLDivElement>('input[data-denomination-id="11"]')!.closest<HTMLElement>('[data-denomination-row]')!;
    const usdRow = form.querySelector<HTMLDivElement>('input[data-denomination-id="21"]')!.closest<HTMLElement>('[data-denomination-row]')!;

    // Initially THB is selected: THB shown, USD hidden
    expect(thbRow.style.display).toBe('');
    expect(thbRow.hasAttribute('hidden')).toBe(false);
    expect(usdRow.style.display).toBe('none');
    expect(usdRow.hasAttribute('hidden')).toBe(true);

    // Switch to USD
    typeInto(select, 'USD');
    await settle();

    expect(thbRow.style.display).toBe('none');
    expect(thbRow.hasAttribute('hidden')).toBe(true);
    expect(usdRow.style.display).toBe('');
    expect(usdRow.hasAttribute('hidden')).toBe(false);
  });

  it('row 1 of item 9: Switching currency keeps the other currency’s quantities in the POST', async () => {
    stub();
    await mount();
    const form = await openCreate();
    const select = form.querySelector<HTMLSelectElement>('select[name="transferCurrency"]')!;

    // Select USD and type quantity 2
    typeInto(select, 'USD');
    await settle();
    const usdInput = form.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!;
    typeInto(usdInput, '2');

    // Switch back to THB and type quantity 3
    typeInto(select, 'THB');
    await settle();
    const thbInput = form.querySelector<HTMLInputElement>('input[data-denomination-id="11"]')!;
    typeInto(thbInput, '3');

    // Select destination branch for BRANCH kind
    const branchSelect = form.querySelector<HTMLSelectElement>('select[data-testid="transfer-to-branch"]')!;
    typeInto(branchSelect, ELSEWHERE);
    await settle();

    // Save
    const saveBtn = form.querySelector<HTMLButtonElement>('[data-testid="transfer-save"]')!;
    saveBtn.click();
    await settle();

    const createCall = sent.find((s) => s.url === '/api/cash-transfers');
    expect(createCall).toBeDefined();
    const body = JSON.parse(createCall!.body!) as { lines: Array<{ currency: string; denominationId: number; qty: string }> };

    // Both USD and THB quantities are preserved and sent
    expect(body.lines).toEqual(
      expect.arrayContaining([
        { currency: 'USD', denominationId: 21, qty: '2' },
        { currency: 'THB', denominationId: 11, qty: '3' },
      ]),
    );
  });

  it('row 2 of item 9: A blank or invalid quantity counts as 0 in the total; the existing validation is unchanged', async () => {
    stub();
    await mount();
    const form = await openCreate();
    const totalEl = form.querySelector<HTMLElement>('[data-testid="transfer-total"]')!;

    const thb1000 = form.querySelector<HTMLInputElement>('input[data-denomination-id="11"]')!;

    // Blank counts as 0
    typeInto(thb1000, '');
    await settle();
    expect(totalEl.textContent).toBe('');

    // Invalid quantity ('abc') counts as 0 in running total
    typeInto(thb1000, 'abc');
    await settle();
    expect(totalEl.textContent).toBe('');

    // Invalid quantity ('-5') counts as 0 in running total
    typeInto(thb1000, '-5');
    await settle();
    expect(totalEl.textContent).toBe('');

    // Valid quantity shows total
    typeInto(thb1000, '2');
    await settle();
    expect(totalEl.textContent).toBe('THB 2,000');

    // Type invalid back into input and submit: existing validation triggers
    typeInto(thb1000, 'abc');
    const branchSelect = form.querySelector<HTMLSelectElement>('select[data-testid="transfer-to-branch"]')!;
    typeInto(branchSelect, ELSEWHERE);
    await settle();

    const saveBtn = form.querySelector<HTMLButtonElement>('[data-testid="transfer-save"]')!;
    saveBtn.click();
    await settle();

    // Field error is shown by existing validation
    const err = form.querySelector('[data-field-error="lines.11.qty"]');
    expect(err).not.toBeNull();
    // No POST sent
    expect(sent.find((s) => s.url === '/api/cash-transfers')).toBeUndefined();
  });

  it('row 3 of item 9: No denominations → no select', async () => {
    stub([]);
    await mount();
    const form = await openCreate();
    const select = form.querySelector('select[name="transferCurrency"]');
    expect(select).toBeNull();
  });

  it('row 4 of item 9: Totals use exact decimals, not floats: 0.25 × 3 is 0.75', async () => {
    stub();
    await mount();
    const form = await openCreate();
    const totalEl = form.querySelector<HTMLElement>('[data-testid="transfer-total"]')!;

    const coin025 = form.querySelector<HTMLInputElement>('input[data-denomination-id="12"]')!;
    typeInto(coin025, '3');
    await settle();

    // Exactly 0.75, not a float approximation
    expect(totalEl.textContent).toBe('THB 0.75');

    // 0.10 × 3 is 0.30000000000000004 in binary float; exact decimals give 0.3
    const select = form.querySelector<HTMLSelectElement>('select[name="transferCurrency"]')!;
    typeInto(select, 'USD');
    await settle();
    typeInto(form.querySelector<HTMLInputElement>('input[data-denomination-id="23"]')!, '3');
    await settle();
    expect(totalEl.textContent).toBe('THB 0.75 · USD 0.3');
  });

  it('running total shows each currency with non-zero quantity joined by ·', async () => {
    stub();
    await mount();
    const form = await openCreate();
    const select = form.querySelector<HTMLSelectElement>('select[name="transferCurrency"]')!;
    const totalEl = form.querySelector<HTMLElement>('[data-testid="transfer-total"]')!;

    // Type 2 x 1000 THB
    const thb1000 = form.querySelector<HTMLInputElement>('input[data-denomination-id="11"]')!;
    typeInto(thb1000, '2');
    await settle();

    // Switch to USD and type 3 x 50 USD
    typeInto(select, 'USD');
    await settle();
    const usd50 = form.querySelector<HTMLInputElement>('input[data-denomination-id="22"]')!;
    typeInto(usd50, '3');
    await settle();

    // Running total shows both currencies joined by ·
    expect(totalEl.textContent).toBe('THB 2,000 · USD 150');
  });

  it('receive form shows running total for received quantities', async () => {
    const lines = [
      makeCashTransferLineRow({ currency: 'THB', denominationId: 11, qtySent: '5' }),
      makeCashTransferLineRow({ currency: 'USD', denominationId: 21, qtySent: '2' }),
    ];
    stub(DENOMS, lines);
    await mount(false);

    // Click receive button
    const rxBtn = host.querySelector<HTMLButtonElement>(`[data-transfer-id="${T_IN}"] [data-testid="transfer-receive"]`);
    expect(rxBtn).not.toBeNull();
    rxBtn!.click();
    await settle();

    const rxForm = host.querySelector<HTMLFormElement>('[data-testid="transfer-receive-form"]');
    expect(rxForm).not.toBeNull();

    const totalEl = rxForm!.querySelector<HTMLElement>('[data-testid="transfer-total"]');
    expect(totalEl).not.toBeNull();
    expect(totalEl!.textContent).toBe('');

    // Type 3 into THB 1000
    const thbInput = rxForm!.querySelector<HTMLInputElement>('input[data-denomination-id="11"]')!;
    typeInto(thbInput, '3');
    await settle();
    expect(totalEl!.textContent).toBe('THB 3,000');

    // Type 1 into USD 100
    const usdInput = rxForm!.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!;
    typeInto(usdInput, '1');
    await settle();
    expect(totalEl!.textContent).toBe('THB 3,000 · USD 100');
  });
});
