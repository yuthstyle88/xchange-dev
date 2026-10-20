// Issue #2654: Staff Transfers — Cancel keeps the new-transfer form's values (and a refused Save's error)

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import {
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
  type StaffMeView,
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
const ELSEWHERE = 'B09';
const OTHER_BRANCH = 'B07';
const ME = '00000000-0000-4000-8000-0000000000b1';

function me(permissions: string[]) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

const DENOMS = [
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '50.0000', groupCode: 'NOTE' }),
];

const DESTINATIONS = [
  makeTransferBranchRow({ branchId: ELSEWHERE, name: 'Ayutthaya' }),
  makeTransferBranchRow({ branchId: OTHER_BRANCH, name: 'Phuket' }),
];

const ID_1 = 'f0000000-0000-4000-8000-0000000000a1';
const ID_2 = 'f0000000-0000-4000-8000-0000000000a2';

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};
type Answer = { status: number; body: unknown } | 'network';

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

function stub(views: Record<string, unknown> = {}) {
  const all: Record<string, unknown> = {
    cash_transfers: [],
    denominations: { rows: DENOMS },
    transfer_branches: { rows: DESTINATIONS },
    ...views,
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const name = path.startsWith('/api/v3/views/')
          ? path.slice('/api/v3/views/'.length)
          : '';
        if (name in all)
          return json(200, { version: 1, changed: true, data: all[name] });
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      const headers = new Headers(init?.headers);
      sent.push({
        url: path,
        method,
        key: headers.get('idempotency-key'),
        body: init?.body as string | undefined,
      });
      const next = answers[path]?.shift();
      if (next === 'network') throw new TypeError('Failed to fetch');
      if (next) return json(next.status, next.body);
      if (path === '/api/cash-transfers')
        return json(200, { id: ID_1, status: 'DRAFT' });
      return json(200, { id: path.split('/')[3], status: 'DRAFT' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView) {
  const card = await loadCard();
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: floor(), refresh };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

function tid(id: string): string {
  return `[data-testid="${id}"]`;
}

function typeInto(
  input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
  value: string,
) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function qtyInput(form: Element, id: number): HTMLInputElement {
  const i = form.querySelector<HTMLInputElement>(
    `input[data-denomination-id="${id}"]`,
  );
  expect(i, `a quantity input for denomination ${id}`).not.toBeNull();
  return i!;
}

async function openCreate(): Promise<HTMLFormElement> {
  must(tid('transfer-new')).click();
  await settle();
  await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());
  const form = must<HTMLFormElement>(tid('transfer-form'));
  await vi.waitFor(() =>
    expect(form.querySelector('input[data-denomination-id]')).not.toBeNull(),
  );
  return form;
}

async function save() {
  must<HTMLButtonElement>(tid('transfer-save')).click();
  await settle();
}

function clickCancel(form: Element) {
  const cancelBtn = [...form.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => b.type === 'button',
  );
  expect(cancelBtn).toBeDefined();
  cancelBtn!.click();
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

describe('108-Plaza/xchange#2654 Staff Transfers: Cancel resets form and clears error', () => {
  // C1: New transfer → kind EXTERNAL_OUT, note "x", a quantity "5" → Cancel → New transfer
  // Required: kind BRANCH, note empty, every quantity empty, no destination branch
  it('C1: New transfer -> kind EXTERNAL_OUT, note "x", quantity "5" -> Cancel -> New transfer -> empty form', async () => {
    stub();
    await mount(me(['CashManage']));

    let form = await openCreate();
    // With kind BRANCH (the default), pick a destination branch, Cancel, and verify reset
    typeInto(must<HTMLSelectElement>(tid('transfer-to-branch')), ELSEWHERE);
    await settle();
    clickCancel(form);
    await settle();
    expect(q(tid('transfer-form'))).toBeNull();

    form = await openCreate();
    expect(must<HTMLSelectElement>(tid('transfer-to-branch')).value).toBe('');

    typeInto(must<HTMLSelectElement>(tid('transfer-to-branch')), ELSEWHERE);
    await settle();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'x');
    typeInto(qtyInput(form, 21), '5');
    await settle();

    clickCancel(form);
    await settle();
    expect(q(tid('transfer-form'))).toBeNull();

    // Reopen form
    form = await openCreate();
    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const toBranchSelect = must<HTMLSelectElement>(tid('transfer-to-branch'));
    const noteInput = must<HTMLInputElement>(tid('transfer-note'));
    const qty21 = qtyInput(form, 21);
    const qty11 = qtyInput(form, 11);
    const qty12 = qtyInput(form, 12);

    expect(kindSelect.value).toBe('BRANCH');
    expect(toBranchSelect.value).toBe('');
    expect(noteInput.value).toBe('');
    expect(qty21.value).toBe('');
    expect(qty11.value).toBe('');
    expect(qty12.value).toBe('');
  });

  // C2: Save is refused (422 overLimit, error banner shown) → Cancel
  // Required: no transfer-error banner
  it('C2: Save is refused (422 overLimit, error banner shown) -> Cancel -> no transfer-error banner', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];

    await mount(me(['CashManage']));

    const form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'attempt');
    typeInto(qtyInput(form, 21), '5');
    await settle();

    await save();
    expect(q(tid('transfer-error'))).not.toBeNull();

    clickCancel(form);
    await settle();

    expect(q(tid('transfer-error'))).toBeNull();
  });

  // C3: after C1, type values and Save (create 200, submit 200)
  // Required: one create + one submit with the new values only
  it('C3: after C1, type values and Save (create 200, submit 200) -> one create + one submit with new values', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 200, body: { id: ID_1, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));

    let form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'x');
    typeInto(qtyInput(form, 21), '5');
    await settle();

    clickCancel(form);
    await settle();

    // Re-open and enter new values
    form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'new transfer note');
    typeInto(qtyInput(form, 21), '2');
    await settle();

    await save();

    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);
    const body = JSON.parse(sent[0]!.body!);
    expect(body.note).toBe('new transfer note');
    expect(body.lines).toEqual([
      { currency: 'USD', denominationId: 21, qty: '2' },
    ]);
  });

  // C4: successful Save, then New transfer
  // Required: empty form
  it('C4: successful Save, then New transfer -> empty form', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 200, body: { id: ID_1, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));

    let form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'success transfer');
    typeInto(qtyInput(form, 21), '3');
    await settle();

    await save();
    expect(q(tid('transfer-form'))).toBeNull();

    // Reopen form
    form = await openCreate();
    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const noteInput = must<HTMLInputElement>(tid('transfer-note'));
    const qty21 = qtyInput(form, 21);

    expect(kindSelect.value).toBe('BRANCH');
    expect(noteInput.value).toBe('');
    expect(qty21.value).toBe('');
  });

  // C5: refused Save → Cancel → New transfer → type the same values → Save
  // Required: a new create, not the old draft id
  it('C5: refused Save -> Cancel -> New transfer -> type the same values -> Save -> creates a new draft', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
      { status: 200, body: { id: ID_2, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_2}/submit`] = [
      { status: 200, body: { id: ID_2, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));

    let form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'same note');
    typeInto(qtyInput(form, 21), '10');
    await settle();

    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);

    clickCancel(form);
    await settle();

    // Re-open and type the same values again
    form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'same note');
    typeInto(qtyInput(form, 21), '10');
    await settle();

    await save();

    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
    ]);
  });
});
