// CA-9 #625, stage 1: the admin cash menu moves from the three retired direct actions (`adminOpenShift`,
// `adminMoveStock`, `adminCloseShift`, design `docs/design/blueprint/cash-shift-transfer.md` §6 `:554`) to CA-8's counts
// and transfers. Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane
// controller claude/5edf7bb5's order; contract approved on #625, issuecomment-6111994145). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// What the page gains, each control shown only when the caller's `me.permissions` holds the permission its server row
// checks (`api/crates/xc-auth/src/domain/policy/areas/cash.rs`), so the source names no role:
//   - "Counts waiting" (`ShiftCloseCertify`): `cash_counts?status=WAITING`, and Verify → `POST /api/cash-counts/{id}/verify`.
//     A 200 that answers `AWAITING_FINANCE` shows the approval request, not a success.
//   - "Transfers" (`CashManage`): `cash_transfers`, one action per status — DRAFT submit, APPROVED dispatch, DISPATCHED
//     receive, RECEIVED reconcile (also `FinanceReconcile`), RECONCILED none.
//   - "New transfer" (`CashManage`): `POST /api/cash-transfers` with CA-8's `TransferCreateInput`; lines from
//     `denominations`; BRANCH from `transfer_branches`; the drawer kinds from the OPEN rows of `admin_shifts`.
// Every write carries one `Idempotency-Key` per user action, kept until it succeeds. Money and quantities are the
// server's strings or what was typed. The screen reads `me` itself: the shell does not pass permissions down
// (`AdminMenuProps`, shared and out of scope).
//
// The test's handles ([proposed], as TD-3's; the design names no selectors):
//   - sections `[data-section="counts-waiting" | "transfers" | "new-transfer"]`;
//   - buttons `[data-action="verify" | "submit" | "dispatch" | "receive" | "reconcile" | "create"]` with `data-id`
//     (the count's or the transfer's id; "create" has none);
//   - the forms' controls: `select[name="kind"]`, `select[name="toBranch"]`, `select[name="fromShift"]`,
//     `select[name="toShift"]`, `input[name="note"]`, and one `input[data-denomination="<id>"]` per line;
//   - the receive and reconcile forms open in the page's `Dialog`, and confirm with `[data-action="confirm"]`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { Me } from '../../../shared/api/generated/common';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { TransferBranchRow } from '../../../shared/api/generated/transfer_branches';
import { errorText, setLang } from '../../../shared/i18n';
import { AdminShell } from '../AdminShell';
import { menus } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import { en, th } from './i18n';

// ── fixtures ────────────────────────────────────────────────────────────────────────────────────────────────────

const VERIFY = 'ShiftCloseCertify';
const MANAGE = 'CashManage';
const RECONCILE = 'FinanceReconcile';

function meWith(permissions: string[]): Me {
  return {
    id: 'u-admin',
    name: 'ผู้ดูแลทดสอบ',
    roles: [],
    permissions,
    branch: 'MAIN',
    branchName: 'สำนักงานใหญ่',
    branches: [{ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' }],
  };
}

/** Figures that do not add up, so a screen that computed them would show something else. */
const COUNTS: CashCountRow[] = [
  {
    id: 'cc-1',
    branchId: 'MAIN',
    shiftId: 'S-0001',
    kind: 'CLOSE',
    countedBy: 'EMP-007',
    countedAt: '2026-10-06T10:00:00Z',
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [
      { currency: 'THB', counted: '1000.50', expected: '1000.75', difference: '-0.25' },
      { currency: 'USD', counted: '120.00', expected: '100.00', difference: '+7.00' },
    ],
  },
];

function transfer(id: string, status: string, over: Partial<CashTransferRow> = {}): CashTransferRow {
  return {
    id,
    kind: 'EXTERNAL_OUT',
    status,
    fromBranchId: 'MAIN',
    toBranchId: null,
    fromShiftId: null,
    toShiftId: null,
    createdBy: 'EMP-002',
    createdAt: '2026-10-06T09:00:00Z',
    note: `note ${id}`,
    lines: [
      { currency: 'USD', denominationId: 11, qtySent: '3', qtyReceived: null },
      { currency: 'USD', denominationId: 12, qtySent: '7', qtyReceived: null },
    ],
    ...over,
  };
}

const TRANSFERS: CashTransferRow[] = [
  transfer('tr-draft', 'DRAFT'),
  transfer('tr-approved', 'APPROVED'),
  transfer('tr-dispatched', 'DISPATCHED'),
  transfer('tr-received', 'RECEIVED'),
  transfer('tr-reconciled', 'RECONCILED'),
];

const DENOMINATIONS: DenominationRow[] = [
  { id: 11, currency: 'USD', faceValue: '100', groupCode: 'G' },
  { id: 12, currency: 'USD', faceValue: '50', groupCode: 'G' },
  { id: 21, currency: 'EUR', faceValue: '20', groupCode: 'G' },
];

/** The server's order, deliberately not alphabetical. */
const BRANCHES: TransferBranchRow[] = [
  { branchId: 'B9', name: 'สาขาเก้า' },
  { branchId: 'B2', name: 'สาขาสอง' },
];

const ADMIN_CASH = {
  shift: { id: 'S-0001', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-007' },
  stock: [{ code: 'THB', opening: '100.00', net: '0.00', expected: '100.00' }],
  stats: { opening: '100.00', incoming: '0.00', outgoing: '0.00', remaining: '100.00' },
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD'],
  lastCounted: null,
};

/** One OPEN and one CLOSED shift: only the OPEN one may be a drawer of a move. */
const SHIFTS = [
  { id: 'S-0001', status: 'OPEN', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-007', note: '', difference: null, balances: [] },
  { id: 'S-0000', status: 'CLOSED', openedAt: '2026-10-05T02:00:00.000Z', staff: 'EMP-002', note: '', difference: [], balances: [] },
];

// ── the fake server: the global fetch ───────────────────────────────────────────────────────────────────────────

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
  body: string | null;
  key: string | null;
}
type Reply = { status: number; body?: unknown } | 'network';

class Server {
  me: Me = meWith([VERIFY, MANAGE, RECONCILE]);
  counts: CashCountRow[] = COUNTS;
  transfers: CashTransferRow[] = TRANSFERS;
  sent: Sent[] = [];
  /** How a command answers, by path; unset = 200 `{ id, status: 'OK' }`. A function answers each call in turn. */
  replies: Record<string, Reply | ((n: number) => Reply)> = {};
  private calls: Record<string, number> = {};

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers);
    const body = typeof init?.body === 'string' ? init.body : null;
    this.sent.push({ method, path: u.pathname, query: u.searchParams, body, key: headers.get('idempotency-key') });
    const ok = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method === 'GET') {
      switch (u.pathname) {
        case '/api/v3/views/me':
          return ok(this.me);
        case '/api/v3/views/kiosk_notifications':
          return json(200, { version: 1, changed: false });
        case '/api/v3/views/admin_branches':
          return ok({ branches: [{ id: 'MAIN', name: 'สำนักงานใหญ่' }] });
        case '/api/v3/views/admin_cash':
          return ok(ADMIN_CASH);
        case '/api/v3/views/admin_cash_movements':
          return ok([]);
        case '/api/v3/views/admin_shifts':
          return ok(SHIFTS);
        case '/api/v3/views/cash_counts':
          return ok(this.counts);
        case '/api/v3/views/cash_transfers':
          return ok(this.transfers);
        case '/api/v3/views/denominations':
          return ok({ rows: DENOMINATIONS });
        case '/api/v3/views/transfer_branches':
          return ok({ rows: BRANCHES });
        default:
          return json(200, { version: 1, changed: true, data: null });
      }
    }
    const n = (this.calls[u.pathname] = (this.calls[u.pathname] ?? 0) + 1);
    const how = this.replies[u.pathname];
    const reply = typeof how === 'function' ? how(n) : how;
    if (reply === 'network') throw new TypeError('Failed to fetch');
    if (reply) return json(reply.status, reply.body ?? {});
    return json(200, { id: 'x', status: 'OK' });
  };

  writes(): Sent[] {
    return this.sent.filter((s) => s.method !== 'GET');
  }

  reads(view: string): Sent[] {
    return this.sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${view}`);
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

/** The menu as the shell mounts it, with the page's one `Dialog` and `Toast`. */
async function mount(permissions: string[] = [VERIFY, MANAGE, RECONCILE]): Promise<HTMLElement> {
  server.me = meWith(permissions);
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  return root;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  vi.stubGlobal('matchMedia', (query: string) => ({
    media: query,
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
  }));
  location.hash = '#cash';
  setLang('th');
});

afterEach(async () => {
  await settle();
  await pendingWrite.abandon();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

// ── DOM helpers ─────────────────────────────────────────────────────────────────────────────────────────────────

const section = (el: ParentNode, name: string) => el.querySelector<HTMLElement>(`[data-section="${name}"]`);
const actions = (el: ParentNode, action: string) =>
  [...el.querySelectorAll<HTMLButtonElement>(`button[data-action="${action}"]`)];
const action = (el: ParentNode, act: string, id?: string) => {
  const found = actions(el, act).filter((b) => id === undefined || b.dataset.id === id);
  expect(found.length, `one ${act} button${id ? ` for ${id}` : ''}`).toBe(1);
  return found[0]!;
};
const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

function type(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function control<T extends HTMLElement>(within: ParentNode, selector: string): T {
  const el = within.querySelector<T>(selector);
  expect(el, `the control ${selector}`).not.toBeNull();
  return el!;
}

async function click(el: HTMLElement): Promise<void> {
  el.click();
  await settle();
}

function dialog(): HTMLDialogElement {
  const d = document.querySelector<HTMLDialogElement>('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

const bodyOf = (s: Sent) => JSON.parse(s.body ?? 'null') as Record<string, unknown>;

// ── the admin entry ─────────────────────────────────────────────────────────────────────────────────────────────

describe('the real admin entry (AdminShell with the menus glob)', () => {
  it('the cash menu is in the shell for any session (needs stays "all") and #cash draws the page with its sections', async () => {
    server.me = meWith([VERIFY, MANAGE, RECONCILE]);
    expect(menu.needs, "cash keeps needs: 'all' (stage 1, #625)").toBe('all');
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(() => <AdminShell api={new ApiClient()} menus={menus} />, root);
    await settle();
    await settle();
    expect(text(root)).toContain(th.label);
    expect(section(root, 'counts-waiting'), 'the counts section through the shell').not.toBeNull();
    expect(section(root, 'transfers'), 'the transfers section through the shell').not.toBeNull();
    expect(section(root, 'new-transfer'), 'the new-transfer section through the shell').not.toBeNull();
  });

  it('a session holding none of the three permissions still has the cash menu, with balances only', async () => {
    server.me = meWith([]);
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(() => <AdminShell api={new ApiClient()} menus={menus} />, root);
    await settle();
    await settle();
    expect(text(root)).toContain(th.label);
    expect(text(root)).toContain('กะกำลังเปิด · S-0001');
    for (const s of ['counts-waiting', 'transfers', 'new-transfer']) expect(section(root, s), s).toBeNull();
  });
});

// ── who sees what (each control by the permission its server row checks) ────────────────────────────────────────

describe('who sees what', () => {
  it('no permission: no section, no new button, and none of the CA-8 views is read', async () => {
    const el = await mount([]);
    for (const s of ['counts-waiting', 'transfers', 'new-transfer']) expect(section(el, s), s).toBeNull();
    for (const a of ['verify', 'submit', 'dispatch', 'receive', 'reconcile', 'create']) expect(actions(el, a), a).toEqual([]);
    for (const v of ['cash_counts', 'cash_transfers', 'denominations', 'transfer_branches']) expect(server.reads(v), v).toEqual([]);
  });

  it('ShiftCloseCertify alone: counts waiting only, read with status=WAITING', async () => {
    const el = await mount([VERIFY]);
    expect(section(el, 'counts-waiting')).not.toBeNull();
    expect(section(el, 'transfers')).toBeNull();
    expect(section(el, 'new-transfer')).toBeNull();
    expect(server.reads('cash_counts')).toHaveLength(1);
    expect(server.reads('cash_counts')[0]!.query.get('status')).toBe('WAITING');
    expect(server.reads('cash_transfers')).toEqual([]);
  });

  it('CashManage alone: transfers and new transfer, no counts, and no reconcile button', async () => {
    const el = await mount([MANAGE]);
    expect(section(el, 'counts-waiting')).toBeNull();
    expect(section(el, 'transfers')).not.toBeNull();
    expect(section(el, 'new-transfer')).not.toBeNull();
    expect(actions(el, 'reconcile')).toEqual([]);
    expect(server.reads('cash_counts')).toEqual([]);
  });

  it('one action per transfer status: DRAFT submit, APPROVED dispatch, DISPATCHED receive, RECEIVED reconcile, RECONCILED none', async () => {
    const el = await mount([MANAGE, RECONCILE]);
    const by = (act: string) => actions(el, act).map((b) => b.dataset.id);
    expect(by('submit')).toEqual(['tr-draft']);
    expect(by('dispatch')).toEqual(['tr-approved']);
    expect(by('receive')).toEqual(['tr-dispatched']);
    expect(by('reconcile')).toEqual(['tr-received']);
    for (const a of ['submit', 'dispatch', 'receive', 'reconcile']) {
      expect(actions(el, a).some((b) => b.dataset.id === 'tr-reconciled'), `${a} on RECONCILED`).toBe(false);
    }
  });
});

// ── counts waiting ──────────────────────────────────────────────────────────────────────────────────────────────

describe('counts waiting (countVerify)', () => {
  it("shows each waiting count's figures as the server sent them", async () => {
    const el = await mount([VERIFY]);
    const s = text(section(el, 'counts-waiting'));
    for (const figure of ['1000.50', '1000.75', '-0.25', '120.00', '100.00', '+7.00', 'EMP-007']) expect(s).toContain(figure);
  });

  it('Verify sends POST /api/cash-counts/{id}/verify {branch, id} with an Idempotency-Key, then reads the counts again', async () => {
    const el = await mount([VERIFY]);
    server.replies['/api/cash-counts/cc-1/verify'] = { status: 200, body: { id: 'cc-1', status: 'VERIFIED', approvalRequestId: null } };
    await click(action(el, 'verify', 'cc-1'));
    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(w[0]!.method).toBe('POST');
    expect(w[0]!.path).toBe('/api/cash-counts/cc-1/verify');
    expect(bodyOf(w[0]!)).toEqual({ branch: 'MAIN', id: 'cc-1' });
    expect(w[0]!.key, 'an Idempotency-Key').toBeTruthy();
    expect(server.reads('cash_counts').length, 'read again after the verify').toBe(2);
  });

  it('a 200 with AWAITING_FINANCE shows the approval request and its own sentence, not success', async () => {
    const el = await mount([VERIFY]);
    server.replies['/api/cash-counts/cc-1/verify'] = {
      status: 200,
      body: { id: 'cc-1', status: 'AWAITING_FINANCE', approvalRequestId: 'apr-77' },
    };
    await click(action(el, 'verify', 'cc-1'));
    const all = text(document.body);
    const sentence = (th as Record<string, string>).countAwaitingFinance;
    expect(sentence, 'the i18n key countAwaitingFinance').toBeTruthy();
    expect(all).toContain(sentence!);
    expect(all).toContain('apr-77');
  });

  it('a refusal shows errorText(code, message)', async () => {
    const el = await mount([VERIFY]);
    server.replies['/api/cash-counts/cc-1/verify'] = {
      status: 409,
      body: { error: { code: 'staleVersion', message: 'already verified' } },
    };
    await click(action(el, 'verify', 'cc-1'));
    expect(text(document.body)).toContain(errorText('staleVersion', 'already verified'));
  });

  it('a network failure, then Verify again: the same Idempotency-Key and a byte-equal body', async () => {
    const el = await mount([VERIFY]);
    server.replies['/api/cash-counts/cc-1/verify'] = (n) => (n === 1 ? 'network' : { status: 200, body: { id: 'cc-1', status: 'VERIFIED', approvalRequestId: null } });
    await click(action(el, 'verify', 'cc-1'));
    await pendingWrite.abandon().catch(() => undefined);
    closeDialog();
    await settle();
    await click(action(el, 'verify', 'cc-1'));
    const w = server.writes().filter((s) => s.path === '/api/cash-counts/cc-1/verify');
    expect(w.length).toBe(2);
    expect(w[1]!.key, 'the same key on the retry').toBe(w[0]!.key);
    expect(w[1]!.body, 'the same body on the retry').toBe(w[0]!.body);
  });
});

// ── the transfer actions ────────────────────────────────────────────────────────────────────────────────────────

describe('transfer actions', () => {
  it('submit and dispatch send {branch, id} to their routes, each with its own key', async () => {
    const el = await mount([MANAGE]);
    await click(action(el, 'submit', 'tr-draft'));
    await click(action(el, 'dispatch', 'tr-approved'));
    const w = server.writes();
    expect(w.map((s) => [s.method, s.path])).toEqual([
      ['POST', '/api/cash-transfers/tr-draft/submit'],
      ['POST', '/api/cash-transfers/tr-approved/dispatch'],
    ]);
    expect(bodyOf(w[0]!)).toEqual({ branch: 'MAIN', id: 'tr-draft' });
    expect(bodyOf(w[1]!)).toEqual({ branch: 'MAIN', id: 'tr-approved' });
    expect(w[0]!.key).toBeTruthy();
    expect(w[1]!.key).toBeTruthy();
    expect(w[1]!.key, 'two actions, two keys').not.toBe(w[0]!.key);
  });

  it('receive asks one quantity per sent line and sends them as typed (qtyReceived strings)', async () => {
    const el = await mount([MANAGE]);
    await click(action(el, 'receive', 'tr-dispatched'));
    const d = dialog();
    type(control<HTMLInputElement>(d, 'input[data-denomination="11"]'), '3');
    type(control<HTMLInputElement>(d, 'input[data-denomination="12"]'), '6.50');
    await click(action(d, 'confirm'));
    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(w[0]!.path).toBe('/api/cash-transfers/tr-dispatched/receive');
    expect(bodyOf(w[0]!)).toEqual({
      branch: 'MAIN',
      id: 'tr-dispatched',
      lines: [
        { currency: 'USD', denominationId: 11, qtyReceived: '3' },
        { currency: 'USD', denominationId: 12, qtyReceived: '6.50' },
      ],
    });
    expect(w[0]!.key).toBeTruthy();
  });

  it('reconcile (with FinanceReconcile) sends {branch, id, note}', async () => {
    const el = await mount([MANAGE, RECONCILE]);
    await click(action(el, 'reconcile', 'tr-received'));
    const d = dialog();
    type(control<HTMLInputElement>(d, 'input[name="note"]'), 'ตรงกับใบนำส่ง');
    await click(action(d, 'confirm'));
    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(w[0]!.path).toBe('/api/cash-transfers/tr-received/reconcile');
    expect(bodyOf(w[0]!)).toEqual({ branch: 'MAIN', id: 'tr-received', note: 'ตรงกับใบนำส่ง' });
    expect(w[0]!.key).toBeTruthy();
  });
});

// ── new transfer (transferCreate) ───────────────────────────────────────────────────────────────────────────────

describe('new transfer (transferCreate)', () => {
  it('offers the five kinds and one quantity per denomination row of the server', async () => {
    const el = await mount([MANAGE]);
    const s = section(el, 'new-transfer')!;
    const kinds = [...control<HTMLSelectElement>(s, 'select[name="kind"]').options].map((o) => o.value).filter(Boolean);
    expect(kinds.sort()).toEqual(['BRANCH', 'DRAWER_TO_SAFE', 'EXTERNAL_IN', 'EXTERNAL_OUT', 'SAFE_TO_DRAWER']);
    expect([...s.querySelectorAll<HTMLInputElement>('input[data-denomination]')].map((i) => i.dataset.denomination)).toEqual(['11', '12', '21']);
    expect(server.reads('denominations')).toHaveLength(1);
  });

  it('EXTERNAL_OUT sends no branch or shift, the typed lines only, and the note', async () => {
    const el = await mount([MANAGE]);
    const s = section(el, 'new-transfer')!;
    choose(control<HTMLSelectElement>(s, 'select[name="kind"]'), 'EXTERNAL_OUT');
    await settle();
    type(control<HTMLInputElement>(s, 'input[data-denomination="12"]'), '4');
    type(control<HTMLInputElement>(s, 'input[data-denomination="21"]'), '10.5');
    type(control<HTMLInputElement>(s, 'input[name="note"]'), 'ส่งธนาคาร');
    await click(action(s, 'create'));
    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(w[0]!.method).toBe('POST');
    expect(w[0]!.path).toBe('/api/cash-transfers');
    expect(bodyOf(w[0]!)).toEqual({
      branch: 'MAIN',
      kind: 'EXTERNAL_OUT',
      toBranchId: null,
      fromShiftId: null,
      toShiftId: null,
      lines: [
        { currency: 'USD', denominationId: 12, qty: '4' },
        { currency: 'EUR', denominationId: 21, qty: '10.5' },
      ],
      note: 'ส่งธนาคาร',
    });
    expect(w[0]!.key).toBeTruthy();
  });

  it("BRANCH picks the destination from transfer_branches, in the server's order, and sends toBranchId", async () => {
    const el = await mount([MANAGE]);
    const s = section(el, 'new-transfer')!;
    choose(control<HTMLSelectElement>(s, 'select[name="kind"]'), 'BRANCH');
    await settle();
    const to = control<HTMLSelectElement>(s, 'select[name="toBranch"]');
    expect([...to.options].map((o) => o.value).filter(Boolean)).toEqual(['B9', 'B2']);
    choose(to, 'B2');
    type(control<HTMLInputElement>(s, 'input[data-denomination="11"]'), '1');
    type(control<HTMLInputElement>(s, 'input[name="note"]'), 'n');
    await click(action(s, 'create'));
    const b = bodyOf(server.writes()[0]!);
    expect([b.kind, b.toBranchId, b.fromShiftId, b.toShiftId]).toEqual(['BRANCH', 'B2', null, null]);
  });

  it('DRAWER_TO_SAFE takes fromShiftId and SAFE_TO_DRAWER takes toShiftId, from the OPEN shifts only', async () => {
    const el = await mount([MANAGE]);
    const s = section(el, 'new-transfer')!;
    choose(control<HTMLSelectElement>(s, 'select[name="kind"]'), 'DRAWER_TO_SAFE');
    await settle();
    const from = control<HTMLSelectElement>(s, 'select[name="fromShift"]');
    expect([...from.options].map((o) => o.value).filter(Boolean), 'only the OPEN shift').toEqual(['S-0001']);
    choose(from, 'S-0001');
    type(control<HTMLInputElement>(s, 'input[data-denomination="11"]'), '2');
    type(control<HTMLInputElement>(s, 'input[name="note"]'), 'n');
    await click(action(s, 'create'));
    let b = bodyOf(server.writes()[0]!);
    expect([b.kind, b.toBranchId, b.fromShiftId, b.toShiftId]).toEqual(['DRAWER_TO_SAFE', null, 'S-0001', null]);

    choose(control<HTMLSelectElement>(s, 'select[name="kind"]'), 'SAFE_TO_DRAWER');
    await settle();
    choose(control<HTMLSelectElement>(s, 'select[name="toShift"]'), 'S-0001');
    type(control<HTMLInputElement>(s, 'input[data-denomination="11"]'), '2');
    type(control<HTMLInputElement>(s, 'input[name="note"]'), 'n');
    await click(action(s, 'create'));
    b = bodyOf(server.writes()[1]!);
    expect([b.kind, b.toBranchId, b.fromShiftId, b.toShiftId]).toEqual(['SAFE_TO_DRAWER', null, null, 'S-0001']);
  });
});

// ── texts ───────────────────────────────────────────────────────────────────────────────────────────────────────

describe('texts', () => {
  it('every new key has TH and EN (countAwaitingFinance, and the section titles)', () => {
    for (const key of ['countAwaitingFinance', 'countsWaitingTitle', 'transfersTitle', 'newTransferTitle']) {
      expect((th as Record<string, string>)[key], `th.${key}`).toBeTruthy();
      expect((en as Record<string, string>)[key], `en.${key}`).toBeTruthy();
    }
  });
});

// ── the source ──────────────────────────────────────────────────────────────────────────────────────────────────

const sources = import.meta.glob<string>('./**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });
const own = Object.entries(sources).filter(([path]) => !path.includes('.test.'));

describe('the source', () => {
  it('calls none of the retired direct actions (design §6)', () => {
    expect(own.length).toBeGreaterThan(0);
    for (const [path, src] of own) {
      for (const retired of ['adminOpenShift', 'adminCloseShift', 'adminMoveStock']) {
        expect(src.includes(retired), `${path}: ${retired}`).toBe(false);
      }
    }
  });

  it('names no role: access is the permission its server row checks', () => {
    const role = /['"`](EXECUTIVE|ADMIN|RATE_MANAGER|BRANCH_MANAGER|TELLER|FINANCE|MARKETING|VERIFICATION_REVIEWER|OWNER|MANAGER|FRONT|BACKEND)['"`]/;
    for (const [path, src] of own) {
      src.split('\n').forEach((line, i) => {
        expect(role.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });
});
