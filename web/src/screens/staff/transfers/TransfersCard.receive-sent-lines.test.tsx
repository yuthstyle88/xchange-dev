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
  type CashTransferLineRow,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { warmModules } from '../../../shared/testing/warm-modules';
import { card } from './card';
import { th as transfersTh } from './i18n';

const BRANCH = 'MAIN';
const SHIFT = 'SH-0b765d57-0000-4000-8000-000000000001';
const T_IN = 'f0000000-0000-4000-8000-000000000001';

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '500', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '100', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 22, currency: 'USD', faceValue: '50', groupCode: 'NOTE' }),
];

type Sent = { url: string; method: string; body: string | undefined };
type Answer = { status: number; body: unknown };

let sent: Sent[];
let answers: Record<string, Answer[]>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function makeIncoming(lines: CashTransferLineRow[]) {
  return makeCashTransferRow({
    id: T_IN,
    kind: 'SAFE_TO_DRAWER',
    status: 'DISPATCHED',
    fromBranchId: BRANCH,
    toBranchId: BRANCH,
    fromShiftId: null,
    toShiftId: SHIFT,
    createdBy: '00000000-0000-4000-8000-0000000000f1',
    createdAt: '2026-11-05T08:45:00Z',
    note: 'Top up drawer',
    lines,
  });
}

function stub(lines: CashTransferLineRow[]) {
  const incoming = makeIncoming(lines);
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
        if (name === 'denominations') return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      sent.push({ url: path, method, body: init?.body as string | undefined });
      const next = answers[path]?.shift();
      if (next) return json(next.status, next.body);
      return json(200, { id: T_IN, status: 'RECEIVED' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mountAndOpenReceive(lines: CashTransferLineRow[]): Promise<HTMLFormElement> {
  stub(lines);
  const ctx: StaffCtx = {
    me: makeStaffMeView({
      userId: '00000000-0000-4000-8000-0000000000c1',
      name: 'Teller 1',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions: ['ShiftOperate', 'ExchangeTransact'],
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
  await vi.waitFor(() =>
    expect(host.querySelector(`[data-transfer-id="${T_IN}"] [data-testid="transfer-receive"]`)).not.toBeNull(),
  );
  host.querySelector<HTMLButtonElement>(`[data-transfer-id="${T_IN}"] [data-testid="transfer-receive"]`)!.click();
  await settle();
  const form = host.querySelector<HTMLFormElement>('[data-testid="transfer-receive-form"]');
  expect(form).not.toBeNull();
  return form!;
}

function typeInto(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

beforeAll(() => warmModules(() => import('./card')), 60_000);

beforeEach(() => {
  sent = [];
  answers = {};
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

describe('F26b (#6320): staff receive form lists sent lines and sends every sent line', () => {
  const usd100Line = makeCashTransferLineRow({
    currency: 'USD',
    denominationId: 21,
    qtySent: '6',
    qtyReceived: null,
  });

  it('1. Transfer sent USD 100 ×6 -> form has exactly one row, "USD 100", input empty, no "6" anywhere in the form', async () => {
    const form = await mountAndOpenReceive([usd100Line]);
    const rows = form.querySelectorAll('[data-denomination-row]');
    expect(rows).toHaveLength(1);
    expect(rows[0]!.textContent).toBe('USD 100');
    const input = rows[0]!.querySelector<HTMLInputElement>('input[data-denomination-id="21"]');
    expect(input).not.toBeNull();
    expect(input!.value).toBe('');
    expect(form.textContent).not.toContain('6');
  });

  it('2. Save untouched -> no request; that row shows countEveryLine', async () => {
    const form = await mountAndOpenReceive([usd100Line]);
    host.querySelector<HTMLButtonElement>('[data-testid="transfer-receive-save"]')!.click();
    await settle();
    expect(sent).toHaveLength(0);
    const errEl = form.querySelector('[data-field-error="lines.21.qty"]');
    expect(errEl).not.toBeNull();
    expect(errEl!.textContent).toBe(transfersTh.countEveryLine);
  });

  it('3. Teller types 6 -> body lines = [{currency:"USD", denominationId:21, qtyReceived:"6"}]', async () => {
    const form = await mountAndOpenReceive([usd100Line]);
    expect(form.querySelectorAll('[data-denomination-row]')).toHaveLength(1);
    const input = form.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!;
    typeInto(input, '6');
    host.querySelector<HTMLButtonElement>('[data-testid="transfer-receive-save"]')!.click();
    await settle();
    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0]!.body!).lines).toEqual([
      { currency: 'USD', denominationId: 21, qtyReceived: '6' },
    ]);
  });

  it('4. Teller types 5 (or 0) -> body carries qtyReceived:"5" (or "0")', async () => {
    const form = await mountAndOpenReceive([usd100Line]);
    expect(form.querySelectorAll('[data-denomination-row]')).toHaveLength(1);
    const input = form.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!;
    typeInto(input, '5');
    host.querySelector<HTMLButtonElement>('[data-testid="transfer-receive-save"]')!.click();
    await settle();
    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0]!.body!).lines).toEqual([
      { currency: 'USD', denominationId: 21, qtyReceived: '5' },
    ]);

    dispose?.();
    host.innerHTML = '';
    sent = [];
    const formZero = await mountAndOpenReceive([usd100Line]);
    expect(formZero.querySelectorAll('[data-denomination-row]')).toHaveLength(1);
    typeInto(formZero.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!, '0');
    host.querySelector<HTMLButtonElement>('[data-testid="transfer-receive-save"]')!.click();
    await settle();
    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0]!.body!).lines).toEqual([
      { currency: 'USD', denominationId: 21, qtyReceived: '0' },
    ]);
  });

  it('5. Two sent lines (THB 1000 ×3, USD 100 ×6) -> two rows, two body lines, in the transfer line order (and server field error maps to i-th sent line)', async () => {
    const twoLines = [
      makeCashTransferLineRow({ currency: 'THB', denominationId: 11, qtySent: '3', qtyReceived: null }),
      usd100Line,
    ];
    answers[`/api/cash-transfers/${T_IN}/receive`] = [
      {
        status: 422,
        body: {
          error: {
            code: 'validationFailed',
            field: 'lines[1].qtyReceived',
            message: 'invalid quantity',
          },
        },
      },
      { status: 200, body: { id: T_IN, status: 'RECEIVED' } },
    ];
    const form = await mountAndOpenReceive(twoLines);
    const rows = form.querySelectorAll('[data-denomination-row]');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.textContent).toBe('THB 1000');
    expect(rows[1]!.textContent).toBe('USD 100');

    typeInto(form.querySelector<HTMLInputElement>('input[data-denomination-id="11"]')!, '3');
    typeInto(form.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!, '6');
    host.querySelector<HTMLButtonElement>('[data-testid="transfer-receive-save"]')!.click();
    await settle();
    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0]!.body!).lines).toEqual([
      { currency: 'THB', denominationId: 11, qtyReceived: '3' },
      { currency: 'USD', denominationId: 21, qtyReceived: '6' },
    ]);
    const errEl = form.querySelector('[data-field-error="lines.21.qty"]');
    expect(errEl).not.toBeNull();
  });
});
