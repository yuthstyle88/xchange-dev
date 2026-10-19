// Issue #2188: Staff transfers — after a refused submit, editing the form and pressing
// Save again creates a new draft and submits the new id instead of resubmitting the old draft.

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
const ID_3 = 'f0000000-0000-4000-8000-0000000000a3';
const ID_4 = 'f0000000-0000-4000-8000-0000000000a4';
const ID_5 = 'f0000000-0000-4000-8000-0000000000a5';

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

describe('Staff transfers resubmit after refused submit (#2188)', () => {
  it('Create OK → submit fails on the network → Save with the form unchanged: only submit is resent with same key and body, while editing creates a new draft', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
      { status: 200, body: { id: ID_2, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      'network',
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_2}/submit`] = [
      { status: 200, body: { id: ID_2, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));
    const form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'to bank');
    typeInto(qtyInput(form, 21), '10');
    await settle();

    // First Save: create OK (ID_1), submit fails on network
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);

    // Second Save with form unchanged: only submit is resent with same key and body
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      `/api/cash-transfers/${ID_1}/submit`,
    ]);
    expect(sent[2]!.key).toBe(sent[1]!.key);
    expect(sent[2]!.body).toBe(sent[1]!.body);

    // Now edit quantity after the 422 refusal and Save: creates ID_2 and submits ID_2
    typeInto(qtyInput(form, 21), '2');
    await settle();
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      `/api/cash-transfers/${ID_1}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
    ]);
  });

  it('Create fails on the network → Save: create is resent with same key and byte-equal body, and subsequent edit after submit 4xx creates anew', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      'network',
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
    const form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'to bank');
    typeInto(qtyInput(form, 21), '5');
    await settle();

    // First Save: create fails on network
    await save();
    expect(sent.map((s) => s.url)).toEqual(['/api/cash-transfers']);

    // Second Save unchanged: create is resent with same key and byte-equal body, then submit ID_1 (which 422s)
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);

    // Edit note and Save: new create -> submit ID_2
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'corrected note');
    await settle();
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
    ]);
  });

  it('Create OK → submit 4xx → user edits a quantity, kind, branch or note → Save: creates new transfer with edited fields and submits new id', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
      { status: 200, body: { id: ID_2, status: 'DRAFT' } },
      { status: 200, body: { id: ID_3, status: 'DRAFT' } },
      { status: 200, body: { id: ID_4, status: 'DRAFT' } },
      { status: 200, body: { id: ID_5, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_2}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_3}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_4}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_5}/submit`] = [
      { status: 200, body: { id: ID_5, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));
    const form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'BRANCH');
    await settle();
    typeInto(must<HTMLSelectElement>(tid('transfer-to-branch')), ELSEWHERE);
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'initial note');
    typeInto(qtyInput(form, 21), '100');
    await settle();

    // 1. Initial Save -> creates ID_1, submit ID_1 returns 422
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);

    // 2. Edit quantity -> Save -> creates ID_2 with new qty, submits ID_2 (422)
    typeInto(qtyInput(form, 21), '50');
    await settle();
    await save();
    expect(sent.slice(2).map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
    ]);
    expect(JSON.parse(sent[2]!.body!).lines).toEqual([
      { currency: 'USD', denominationId: 21, qty: '50' },
    ]);
    expect(JSON.parse(sent[3]!.body!).id).toBe(ID_2);

    // 3. Edit branch -> Save -> creates ID_3 with new branch, submits ID_3 (422)
    typeInto(must<HTMLSelectElement>(tid('transfer-to-branch')), OTHER_BRANCH);
    await settle();
    await save();
    expect(sent.slice(4).map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_3}/submit`,
    ]);
    expect(JSON.parse(sent[4]!.body!).toBranchId).toBe(OTHER_BRANCH);
    expect(JSON.parse(sent[5]!.body!).id).toBe(ID_3);

    // 4. Edit note -> Save -> creates ID_4 with new note, submits ID_4 (422)
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'updated note');
    await settle();
    await save();
    expect(sent.slice(6).map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_4}/submit`,
    ]);
    expect(JSON.parse(sent[6]!.body!).note).toBe('updated note');
    expect(JSON.parse(sent[7]!.body!).id).toBe(ID_4);

    // 5. Edit kind -> Save -> creates ID_5 with new kind, submits ID_5 (200 OK)
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    await save();
    expect(sent.slice(8).map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_5}/submit`,
    ]);
    expect(JSON.parse(sent[8]!.body!).kind).toBe('EXTERNAL_OUT');
    expect(JSON.parse(sent[9]!.body!).id).toBe(ID_5);
  });

  it('Create OK → submit 4xx → Save with nothing changed: no new create; submit of the same id, until edited', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
      { status: 200, body: { id: ID_2, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 400, body: { error: { code: 'invalidState', message: 'Refused' } } },
      { status: 400, body: { error: { code: 'invalidState', message: 'Refused again' } } },
    ];
    answers[`/api/cash-transfers/${ID_2}/submit`] = [
      { status: 200, body: { id: ID_2, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));
    const form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'same note');
    typeInto(qtyInput(form, 21), '3');
    await settle();

    // First Save -> create ID_1, submit ID_1 (400)
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);

    // Second Save with nothing changed -> no new create, submits same ID_1
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      `/api/cash-transfers/${ID_1}/submit`,
    ]);
    expect(JSON.parse(sent[2]!.body!).id).toBe(ID_1);

    // Third Save after editing -> creates ID_2 and submits ID_2
    typeInto(qtyInput(form, 21), '1');
    await settle();
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      `/api/cash-transfers/${ID_1}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
    ]);
  });

  it('Cancel, then a new transfer: creates from scratch (and editing after 4xx before cancel also creates anew)', async () => {
    stub();
    answers['/api/cash-transfers'] = [
      { status: 200, body: { id: ID_1, status: 'DRAFT' } },
      { status: 200, body: { id: ID_2, status: 'DRAFT' } },
      { status: 200, body: { id: ID_3, status: 'DRAFT' } },
    ];
    answers[`/api/cash-transfers/${ID_1}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_2}/submit`] = [
      { status: 422, body: { error: { code: 'overLimit', message: 'Still over limit' } } },
    ];
    answers[`/api/cash-transfers/${ID_3}/submit`] = [
      { status: 200, body: { id: ID_3, status: 'SUBMITTED' } },
    ];

    await mount(me(['CashManage']));
    let form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'first attempt');
    typeInto(qtyInput(form, 21), '10');
    await settle();

    // First Save -> create ID_1, submit ID_1 (422)
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
    ]);

    // Edit note -> Save -> create ID_2, submit ID_2 (422)
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'edited before cancel');
    await settle();
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
    ]);

    // Cancel
    const cancelBtn = [...form.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.type === 'button',
    )!;
    cancelBtn.click();
    await settle();
    expect(q(tid('transfer-form'))).toBeNull();

    // Re-open with exact same values as ID_2 -> must create from scratch (ID_3), not reuse ID_2
    form = await openCreate();
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'edited before cancel');
    typeInto(qtyInput(form, 21), '10');
    await settle();
    await save();
    expect(sent.map((s) => s.url)).toEqual([
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_1}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_2}/submit`,
      '/api/cash-transfers',
      `/api/cash-transfers/${ID_3}/submit`,
    ]);
  });
});
