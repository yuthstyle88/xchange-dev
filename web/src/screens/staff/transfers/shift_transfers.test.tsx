import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { TransferBranchRow } from '../../../shared/api/generated/transfer_branches';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th, en } from './i18n';
import { th as staffTh } from '../i18n';
import type { StaffCard, StaffCtx } from '../cards';

const cardMods = import.meta.glob<{ card: StaffCard }>('./card.ts');

async function loadCard(): Promise<StaffCard> {
  const l = cardMods['./card.ts'];
  expect(l).toBeDefined();
  return (await l!()).card;
}

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000b1';
const SHIFT_ID = 'shift-open-001';

function me(permissions: string[]): StaffMeView {
  return {
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  };
}

function floorWithShift(shiftId: string | null): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: shiftId ? { id: shiftId, openedAt: '2026-10-11T01:00:00Z' } : null,
    cutoff: null,
    target: null,
    safe: null,
  };
}

const DENOMS: DenominationRow[] = [
  { id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' },
  { id: 21, currency: 'USD', faceValue: '50.0000', groupCode: 'NOTE' },
];

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};

type Answer = { status: number; body: unknown } | 'network' | 'network-empty';

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
    transfer_branches: { rows: [] },
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
        if (name in all) {
          return json(200, { version: 1, changed: true, data: all[name] });
        }
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
      if (next === 'network-empty') throw new TypeError('');
      if (next) return json(next.status, next.body);

      if (path === '/api/cash-transfers') {
        return json(200, { id: 'created-shift-tx', status: 'DRAFT' });
      }
      return json(200, { id: path.split('/')[3], status: 'DRAFT' });
    })
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(shiftId: string | null, permissions: string[] = ['CashManage', 'FinanceReconcile']) {
  const card = await loadCard();
  const ctx: StaffCtx = {
    me: me(permissions),
    floor: floorWithShift(shiftId),
    refresh: vi.fn(async () => {}),
  };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
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

async function click(el: Element) {
  (el as HTMLElement).click();
  await settle();
}

function typeInto(
  input: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement,
  value: string
) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function bodyOf(s: Sent): Record<string, unknown> {
  const b = JSON.parse(s.body!) as Record<string, unknown>;
  delete b['branch'];
  for (const k of Object.keys(b)) {
    if (b[k] === null) delete b[k];
  }
  return b;
}

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

describe('Lane controller rulings for drawer moves and card robustness', () => {
  it('1. DRAWER_TO_SAFE sends fromShiftId = my_shift.id and no toShiftId', async () => {
    stub();
    await mount(SHIFT_ID);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const opt = [...kindSelect.options].find((o) => o.value === 'DRAWER_TO_SAFE');
    expect(opt).toBeDefined();
    expect(opt!.disabled).toBe(false);

    typeInto(kindSelect, 'DRAWER_TO_SAFE');
    await settle();

    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'end of day safe move');
    const qtyInput = must<HTMLInputElement>('input[data-denomination-id="11"]');
    typeInto(qtyInput, '5');
    await settle();

    await click(must(tid('transfer-save')));

    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      'POST /api/cash-transfers',
      'POST /api/cash-transfers/created-shift-tx/submit',
    ]);

    const createBody = bodyOf(sent[0]!);
    expect(createBody['kind']).toBe('DRAWER_TO_SAFE');
    expect(createBody['fromShiftId']).toBe(SHIFT_ID);
    expect('toShiftId' in createBody).toBe(false);
  });

  it('2. SAFE_TO_DRAWER sends toShiftId = my_shift.id and no fromShiftId', async () => {
    stub();
    await mount(SHIFT_ID);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const opt = [...kindSelect.options].find((o) => o.value === 'SAFE_TO_DRAWER');
    expect(opt).toBeDefined();
    expect(opt!.disabled).toBe(false);

    typeInto(kindSelect, 'SAFE_TO_DRAWER');
    await settle();

    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'morning float from safe');
    const qtyInput = must<HTMLInputElement>('input[data-denomination-id="21"]');
    typeInto(qtyInput, '10');
    await settle();

    await click(must(tid('transfer-save')));

    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      'POST /api/cash-transfers',
      'POST /api/cash-transfers/created-shift-tx/submit',
    ]);

    const createBody = bodyOf(sent[0]!);
    expect(createBody['kind']).toBe('SAFE_TO_DRAWER');
    expect(createBody['toShiftId']).toBe(SHIFT_ID);
    expect('fromShiftId' in createBody).toBe(false);
  });

  it('3. When there is no open shift, drawer kinds are disabled and show noOpenShift reason text', async () => {
    stub();
    await mount(null);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const d2sOpt = [...kindSelect.options].find((o) => o.value === 'DRAWER_TO_SAFE');
    const s2dOpt = [...kindSelect.options].find((o) => o.value === 'SAFE_TO_DRAWER');

    expect(d2sOpt).toBeDefined();
    expect(d2sOpt!.disabled).toBe(true);
    expect(d2sOpt!.textContent).toContain(th.noOpenShift);

    expect(s2dOpt).toBeDefined();
    expect(s2dOpt!.disabled).toBe(true);
    expect(s2dOpt!.textContent).toContain(th.noOpenShift);

    // Reason text is also shown beside the kind select
    expect(must(tid('transfer-form')).textContent).toContain(th.noOpenShift);
  });

  it('4. Without an open shift, non-drawer kinds (BRANCH, EXTERNAL_IN, EXTERNAL_OUT) are still enabled and submit', async () => {
    stub();
    await mount(null);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    for (const k of ['BRANCH', 'EXTERNAL_IN', 'EXTERNAL_OUT']) {
      const opt = [...kindSelect.options].find((o) => o.value === k);
      expect(opt, `${k} exists`).toBeDefined();
      expect(opt!.disabled, `${k} is enabled`).toBe(false);
    }

    typeInto(kindSelect, 'EXTERNAL_IN');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'external deposit without shift');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="11"]'), '3');
    await settle();

    await click(must(tid('transfer-save')));

    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      'POST /api/cash-transfers',
      'POST /api/cash-transfers/created-shift-tx/submit',
    ]);
    const createBody = bodyOf(sent[0]!);
    expect(createBody['kind']).toBe('EXTERNAL_IN');
  });

  it('5. Shows localized kind_* labels in kind select and list rows (TH and EN), not raw codes', async () => {
    const row: CashTransferRow = {
      id: 'tx-lang-001',
      kind: 'DRAWER_TO_SAFE',
      fromBranchId: BRANCH,
      toBranchId: null,
      fromShiftId: SHIFT_ID,
      toShiftId: null,
      status: 'APPROVED',
      createdBy: ME,
      note: 'lang test',
      createdAt: '2026-10-11T00:00:00Z',
      lines: [],
    };

    // Thai check
    setLang('th');
    stub({ cash_transfers: [row] });
    await mount(SHIFT_ID);

    const trTh = must('tr[data-transfer-id="tx-lang-001"]');
    expect(trTh.textContent).toContain(th.kind_DRAWER_TO_SAFE);
    expect(trTh.textContent).not.toContain('DRAWER_TO_SAFE');

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());
    const kindSelectTh = must<HTMLSelectElement>(tid('transfer-kind'));
    const branchOptTh = [...kindSelectTh.options].find((o) => o.value === 'BRANCH')!;
    expect(branchOptTh.textContent).toContain(th.kind_BRANCH);
    expect(branchOptTh.textContent).not.toBe('BRANCH');

    // English check
    dispose?.();
    setLang('en');
    stub({ cash_transfers: [row] });
    await mount(SHIFT_ID);

    const trEn = must('tr[data-transfer-id="tx-lang-001"]');
    expect(trEn.textContent).toContain(en.kind_DRAWER_TO_SAFE);
    expect(trEn.textContent).not.toContain('DRAWER_TO_SAFE');

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());
    const kindSelectEn = must<HTMLSelectElement>(tid('transfer-kind'));
    const branchOptEn = [...kindSelectEn.options].find((o) => o.value === 'BRANCH')!;
    expect(branchOptEn.textContent).toContain(en.kind_BRANCH);
    expect(branchOptEn.textContent).not.toBe('BRANCH');
  });

  it('6. Notice uses t("saved") so English users see English Saved notice instead of Thai', async () => {
    setLang('en');
    stub();
    await mount(null);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'note');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="11"]'), '1');
    await settle();

    await click(must(tid('transfer-save')));

    const noticeEl = must(tid('transfer-notice'));
    expect(noticeEl.textContent).toBe(en.saved);
    expect(noticeEl.textContent).not.toBe(th.saved);
  });

  it('7. Reconcile keeps ONE command (one Idempotency-Key) on retry after network error', async () => {
    const row: CashTransferRow = {
      id: 'tx-rec-001',
      kind: 'BRANCH',
      fromBranchId: 'B99',
      toBranchId: BRANCH,
      fromShiftId: null,
      toShiftId: null,
      status: 'RECEIVED',
      createdBy: ME,
      note: 'reconcile target',
      createdAt: '2026-10-11T00:00:00Z',
      lines: [],
    };
    stub({ cash_transfers: [row] });
    answers['/api/cash-transfers/tx-rec-001/reconcile'] = [
      'network',
      { status: 200, body: { id: 'tx-rec-001', status: 'RECONCILED' } },
    ];

    await mount(null);
    await click(must(tid('transfer-reconcile')));

    // Reason dialog appears with input
    await vi.waitFor(() => expect(q('dialog')).not.toBeNull());
    const dialog = must('dialog');
    typeInto(must<HTMLInputElement>('dialog input'), 'verified counts match');

    // Confirm button in ReasonDialog
    const confirmBtn = [...dialog.querySelectorAll('button')].find(
      (b) => b.textContent === staffTh.confirm
    );
    expect(confirmBtn).toBeDefined();

    // First attempt -> fails on network
    await click(confirmBtn!);

    expect(sent.length).toBe(1);
    expect(sent[0]!.url).toBe('/api/cash-transfers/tx-rec-001/reconcile');
    const firstKey = sent[0]!.key;
    const firstBody = sent[0]!.body;
    expect(firstKey).toBeTruthy();

    // Dialog stays open; click confirm again (retry)
    await click(confirmBtn!);

    expect(sent.length).toBe(2);
    expect(sent[1]!.url).toBe('/api/cash-transfers/tx-rec-001/reconcile');
    expect(sent[1]!.key).toBe(firstKey);
    expect(sent[1]!.body).toBe(firstBody);
  });

  it('8. The BRANCH destination list keeps transfer_branches server order, with no client re-sort', async () => {
    const serverBranches: TransferBranchRow[] = [
      { branchId: 'Z99', name: 'Zebra Branch' },
      { branchId: 'A01', name: 'Alpha Branch' },
      { branchId: 'M50', name: 'Middle Branch' },
    ];
    stub({ transfer_branches: { rows: serverBranches } });
    await mount(null);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'BRANCH');
    await settle();

    const branchSelect = must<HTMLSelectElement>(tid('transfer-to-branch'));
    const options = [...branchSelect.options]
      .map((o) => o.value)
      .filter((v) => v !== '');

    expect(options).toEqual(['Z99', 'A01', 'M50']);
  });

  it('9. Receive starts EMPTY, with no prefill from row lines (blind count)', async () => {
    const row: CashTransferRow = {
      id: 'tx-recv-001',
      kind: 'BRANCH',
      fromBranchId: 'B99',
      toBranchId: BRANCH,
      fromShiftId: null,
      toShiftId: null,
      status: 'DISPATCHED',
      createdBy: ME,
      note: 'incoming cash',
      createdAt: '2026-10-11T00:00:00Z',
      lines: [
        { currency: 'THB', denominationId: 11, qtySent: '50' } as any,
        { currency: 'USD', denominationId: 21, qtySent: '100' } as any,
      ],
    };
    stub({ cash_transfers: [row] });
    await mount(null);

    await click(must(tid('transfer-receive')));
    await vi.waitFor(() => expect(q(tid('transfer-receive-form'))).not.toBeNull());

    const input11 = must<HTMLInputElement>('input[data-denomination-id="11"]');
    const input21 = must<HTMLInputElement>('input[data-denomination-id="21"]');

    expect(input11.value).toBe('');
    expect(input21.value).toBe('');
  });

  it('10. On a network TypeError, shows a generic connection-error message, never a blank one', async () => {
    stub();
    answers['/api/cash-transfers'] = ['network'];
    await mount(null);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'bank note');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="11"]'), '2');
    await settle();

    await click(must(tid('transfer-save')));

    const errEl = must(tid('transfer-error'));
    expect(errEl.textContent?.trim()).toBe(sharedTh['error.network']);
    expect(errEl.textContent).not.toContain('Failed to fetch');

    // Also test network with empty message
    answers['/api/cash-transfers'] = ['network-empty'];
    await click(must(tid('transfer-save')));
    const errEl2 = must(tid('transfer-error'));
    expect(errEl2.textContent?.trim()).toBe(sharedTh['error.network']);

    // English check
    dispose?.();
    setLang('en');
    stub();
    answers['/api/cash-transfers'] = ['network'];
    await mount(null);
    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'EXTERNAL_OUT');
    await settle();
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'bank note');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="11"]'), '2');
    await settle();

    await click(must(tid('transfer-save')));

    const errElEn = must(tid('transfer-error'));
    expect(errElEn.textContent?.trim()).toBe(sharedEn['error.network']);
    expect(errElEn.textContent).not.toContain('Failed to fetch');
  });
});
