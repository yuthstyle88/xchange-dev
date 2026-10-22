// G-ME, GME-T (`docs/design/web-silent-failures.md` §5): B04 `/desk/transactions/{id}` when its `me` read fails.
// Written before the implementation by the ticket author (claude/e8020026's sub-agent). Do not edit, delete or weaken;
// add tests in a file of your own. The harness below is copied from TransactionScreen.test.tsx (TD-3 #1291).
//
// What changes (the #1719 standard): a failed `me` read is no longer a silent hide. The screen keeps D2's own `me`
// read (open, refresh), and when it fails:
//   - the data still shows (state `ready`), with the shared line `[data-permissions-unavailable]` (role="alert");
//   - each reprint button and the request-reversal button are on screen, **disabled**, with
//     `title = permissionsUnavailableText(error)`; the non-permission conditions still decide whether the request
//     button exists at all (COMPLETED, no pending or active reversal);
//   - the line's retry (`button[data-action="permissions-retry"]`) reads `me` once more; a good answer removes the
//     line and the actions follow the permissions as today.
// What stays (guards): with `me` loaded, an action the person lacks the permission for stays **hidden**
// (`ReceiptsSection.tsx:34`, `TransactionScreen.tsx:208`), and no line shows.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import {
  makeMe,
  makeReceiptDocumentDto,
  makeReceiptDocumentViewData,
  makeReversalSummaryDto,
  makeTransactionAuditEventDto,
  makeTransactionAuditViewData,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeTxLineDto,
  makeTxPaymentDto,
  makeTxReceiptDto,
  type MeRole,
  type MeView,
  type ReceiptDocumentViewData,
  type ReversalSummaryDto,
  type TransactionAuditViewData,
  type TransactionDetailDto,
  type TransactionViewData,
} from '../../shared/testing/fixtures';
import { ReceiptFrame } from '../../shared/receipt/ReceiptFrame';
import { ApiError } from '../../shared/api/client';
import { permissionsUnavailableText } from '../../shared/ui/PermissionsUnavailable';
import { th as shared } from '../../shared/i18n/th';
import { TransactionScreen } from './TransactionScreen';

// ── answer types: the generated types of TX-10 #678, RV-4 #685 and TD-2 #1294 (merged) ──────────────────────────────

/** The statuses these fixtures use; the wire field is a `string` (`ReversalSummaryDto.status`). */
type ReversalStatus = 'REQUESTED' | 'APPROVED' | 'POSTED' | 'REJECTED' | 'EXPIRED';
type TransactionData = TransactionViewData;
type TransactionAuditData = TransactionAuditViewData;
type ReceiptDocumentData = ReceiptDocumentViewData;

// ── fixtures ────────────────────────────────────────────────────────────────────────────────────────────────────

const TX_ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';
const PATH = `/desk/transactions/${TX_ID}`;
const RECEIPT_HTML = '<!doctype html><html><body>ใบเสร็จ XC-0001 35,120.00</body></html>';

function me(permissions: string[], roles?: MeRole[]) {
  return makeMe({
    id: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    roles,
    branch: 'MAIN',
    branchName: 'สาขาทดสอบ',
    branches: [],
    permissions,
  });
}
const ALL = ['TransactionRead', 'ReceiptReprint', 'ReversalRequest'];

function completed(over: Partial<TransactionDetailDto> = {}) {
  return makeTransactionDetailDto({
    id: TX_ID,
    status: 'COMPLETED',
    documentRef: 'XC-0001',
    branchId: 'MAIN',
    customerId: 'cust-1',
    quoteId: null,
    quoteExpiresAt: null,
    lines: [
      makeTxLineDto({
        lineNo: 1,
        currency: 'USD',
        side: 'BUY',
        quantity: '1000.0000',
        unitSize: '1.0000',
        rateSnapshot: '35.12000000',
        thbAmount: '35120.00',
        feeThb: '0.00',
        settleThb: '35120.00',
      }),
    ],
    payment: makeTxPaymentDto({
      method: 'CASH',
      reference: '',
      account: '',
      slipFileKey: null,
      amountThb: '35120.00',
      bankFeeThb: '0.00',
      paidAt: '2026-10-11T03:00:00Z',
    }),
    receipts: [makeTxReceiptDto({ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z' })],
    approvalRequest: null,
    completedAt: '2026-10-11T03:00:05Z',
    cancelledAt: null,
    cancelReason: null,
    reversal: null,
    benefits: [],
    booking: null,
    ...over,
  });
}

function reversal(status: ReversalStatus, over: Partial<ReversalSummaryDto> = {}) {
  return makeReversalSummaryDto({
    id: 'rv-0001',
    originalTransactionId: TX_ID,
    status,
    reason: 'ลูกค้าขอยกเลิก',
    amount: '35120.00',
    requestedBy: 'u-teller',
    requestedAt: '2026-10-11T04:00:00Z',
    approvedAt: status === 'REQUESTED' || status === 'REJECTED' ? null : '2026-10-11T04:10:00Z',
    postedAt: status === 'POSTED' ? '2026-10-11T04:20:00Z' : null,
    documentNumber: status === 'POSTED' ? 'XC-0001-R2' : null,
    ...over,
  });
}

/**
 * Only decision events: `ApprovalService::request` writes no audit event, only a decision does (measured by the BE lane
 * on #1294). Here: the over-limit approval of the exchange decided, then the reversal's FINANCE decision.
 */
const AUDIT: TransactionAuditData = makeTransactionAuditViewData({
  events: [
    makeTransactionAuditEventDto({ at: '2026-10-11T02:59:30Z', actorId: 'u-mgr', actorRole: 'ROLE_A', action: 'approval.approved' }),
    makeTransactionAuditEventDto({ at: '2026-10-11T04:10:00Z', actorId: 'u-fin', actorRole: 'ROLE_B', action: 'reversal.approved' }),
  ],
});

// ── the fake server ─────────────────────────────────────────────────────────────────────────────────────────────

type Call = { method: string; url: string; path: string; query: URLSearchParams; body: string | null; key: string | null };
type Reply = { status: number; json?: unknown; network?: true; text?: string };

interface World {
  me: MeView | Reply;
  transaction: TransactionData | Reply;
  audit: TransactionAuditData | Reply;
  receipt: ReceiptDocumentData;
  /** Answers to the reversal POST, in order; the last one repeats. */
  posts: Reply[];
  /** After a successful POST, the transaction answer becomes this. */
  afterPost?: TransactionData;
}

let world: World;
let calls: Call[] = [];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let postCount = 0;

function isReply(v: unknown): v is Reply {
  return typeof v === 'object' && v !== null && 'status' in v && typeof (v as Reply).status === 'number';
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

function answer(r: Reply): Response {
  if (r.network) throw new TypeError('Failed to fetch');
  if (r.text !== undefined) return new Response(r.text, { status: r.status });
  return json(r.status, r.json ?? {});
}

function view<T>(data: T | Reply): Response {
  if (isReply(data)) return answer(data);
  return json(200, { version: 1, changed: true, data });
}

function installFetch() {
  calls = [];
  postCount = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : null;
      calls.push({ method, url, path: u.pathname, query: u.searchParams, body, key: headers.get('Idempotency-Key') });
      if (method === 'GET' && u.pathname === '/api/v3/views/me') return view(world.me);
      if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
        return json(401, { error: { code: 'employeeAuthRequired', message: 'employee auth required' } });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction') return view(world.transaction);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction_audit') return view(world.audit);
      if (method === 'GET' && u.pathname === '/api/v3/views/receipt_document') return view(world.receipt);
      if (method === 'POST' && u.pathname === `/api/transactions/${TX_ID}/reversal-requests`) {
        const r = world.posts[Math.min(postCount, world.posts.length - 1)]!;
        postCount++;
        const res = answer(r);
        if (res.ok && world.afterPost) world.transaction = world.afterPost;
        return res;
      }
      return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
    }),
  );
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

async function mount(over: Partial<World> = {}, search = '') {
  world = {
    me: me(ALL),
    transaction: makeTransactionViewData({ transaction: completed() }),
    audit: AUDIT,
    receipt: makeReceiptDocumentViewData({
      receipt: makeReceiptDocumentDto({ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z', html: RECEIPT_HTML }),
    }),
    posts: [{ status: 200, json: { id: 'rv-0001', status: 'REQUESTED' } }],
    ...over,
  };
  installFetch();
  dispose = render(() => <TransactionScreen pathname={PATH} search={search} />, host);
  await settle();
  return host;
}

const gets = (name: string) => calls.filter((c) => c.method === 'GET' && c.path === `/api/v3/views/${name}`);
const nonGets = () => calls.filter((c) => c.method !== 'GET');
const btn = (action: string) => host.querySelectorAll<HTMLButtonElement>(`button[data-action="${action}"]`);
const state = () => host.querySelector('[data-state]')?.getAttribute('data-state');
const text = () => host.textContent ?? '';

async function click(el: Element | null | undefined) {
  expect(el, 'the button is there').toBeTruthy();
  (el as HTMLElement).click();
  await settle();
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

// ── G-ME ────────────────────────────────────────────────────────────────────────────────────────────────────────

const ME_DOWN: Reply = { status: 500, json: { error: { code: 'serverError', message: 'x' } } };
const ME_OFFLINE: Reply = { status: 0, network: true };

const line = () => host.querySelector<HTMLElement>('[data-permissions-unavailable]');
const retryPermissions = () => host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');

describe('G-ME: me fails ⇒ worded, actions shown disabled with the reason', () => {
  for (const [label, failure] of [
    ['a 500', ME_DOWN],
    ['a network failure', ME_OFFLINE],
  ] as const) {
    it(`${label}: the data shows, with the shared role="alert" line`, async () => {
      await mount({ me: failure });
      expect(state()).toBe('ready');
      expect(text()).toContain('35120.00');
      const p = line();
      expect(p, 'the shared line').not.toBeNull();
      expect(p!.getAttribute('role')).toBe('alert');
      expect(p!.textContent).toContain(shared['shell.permissions.unavailable']);
    });

    it(`${label}: reprint and request-reversal are present, disabled, with the reason as title`, async () => {
      await mount({ me: failure });
      const reprint = btn('reprint');
      expect(reprint).toHaveLength(1);
      expect(reprint[0]!.disabled).toBe(true);
      expect(reprint[0]!.title).toContain(shared['shell.permissions.unavailable']);
      const request = btn('request-reversal');
      expect(request).toHaveLength(1);
      expect(request[0]!.disabled).toBe(true);
      expect(request[0]!.title).toContain(shared['shell.permissions.unavailable']);
      expect(request[0]!.title).toBe(reprint[0]!.title);
    });
  }

  it('the title is the shared worded reason: unavailable + the worded error, never the raw message', async () => {
    await mount({ me: ME_DOWN });
    expect(btn('reprint')[0]!.title).toBe(`${shared['shell.permissions.unavailable']} ${shared['error.serverError']}`);
    expect(btn('reprint')[0]!.title).toBe(permissionsUnavailableText(new ApiError({ status: 500, code: 'serverError', message: 'x' })));
    dispose?.();
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    await mount({ me: ME_OFFLINE });
    expect(btn('reprint')[0]!.title).toBe(`${shared['shell.permissions.unavailable']} ${shared['error.network']}`);
  });

  it('a click on a disabled action sends and prints nothing', async () => {
    const print = vi.spyOn(ReceiptFrame, 'print').mockResolvedValue();
    await mount({ me: ME_DOWN });
    btn('reprint')[0]!.click();
    btn('request-reversal')[0]!.click();
    await settle();
    expect(gets('receipt_document')).toEqual([]);
    expect(print).not.toHaveBeenCalled();
    expect(nonGets()).toEqual([]);
    expect(host.querySelector('dialog[open]'), 'no reason dialog').toBeNull();
  });

  it('the request button still follows the non-permission rules: an active reversal ⇒ no request button', async () => {
    await mount({
      me: ME_DOWN,
      transaction: makeTransactionViewData({ transaction: completed({ reversal: reversal('REQUESTED') }) }),
    });
    expect(line()).not.toBeNull();
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('CANCELLED with me failing: no receipts, no request button, the line shows', async () => {
    await mount({
      me: ME_DOWN,
      transaction: makeTransactionViewData({
        transaction: completed({ status: 'CANCELLED', receipts: [], cancelledAt: '2026-10-11T03:10:00Z', cancelReason: 'x' }),
      }),
    });
    expect(btn('reprint')).toHaveLength(0);
    expect(btn('request-reversal')).toHaveLength(0);
    expect(line()).not.toBeNull();
  });

  it('retry reads me once more; a good answer removes the line and enables the actions', async () => {
    await mount({ me: ME_DOWN });
    expect(gets('me')).toHaveLength(1);
    world.me = me(ALL);
    await click(retryPermissions());
    expect(gets('me')).toHaveLength(2);
    expect(line()).toBeNull();
    expect(btn('reprint')[0]!.disabled).toBe(false);
    expect(btn('request-reversal')[0]!.disabled).toBe(false);
  });

  it('retry that brings a me without the permissions: the line goes and the actions are hidden as today', async () => {
    await mount({ me: ME_DOWN });
    world.me = me(['TransactionRead']);
    await click(retryPermissions());
    expect(line()).toBeNull();
    expect(btn('reprint')).toHaveLength(0);
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('the refresh button (D2) re-reads me too: a good answer removes the line', async () => {
    await mount({ me: ME_DOWN });
    world.me = me(ALL);
    await click(btn('refresh')[0]);
    expect(line()).toBeNull();
    expect(btn('reprint')[0]!.disabled).toBe(false);
  });
});

describe('G-ME guards: me loaded ⇒ today’s behaviour, no line', () => {
  it('all permissions: both actions enabled, no title, no line', async () => {
    await mount();
    expect(line()).toBeNull();
    expect(retryPermissions()).toBeNull();
    expect(btn('reprint')[0]!.disabled).toBe(false);
    expect(btn('reprint')[0]!.title).toBe('');
    expect(btn('request-reversal')[0]!.disabled).toBe(false);
  });

  it('without ReceiptReprint: the reprint button is hidden (not disabled), no line', async () => {
    await mount({ me: me(['TransactionRead', 'ReversalRequest']) });
    expect(btn('reprint')).toHaveLength(0);
    expect(line()).toBeNull();
  });

  it('without ReversalRequest: the request button is hidden (not disabled), no line', async () => {
    await mount({ me: me(['TransactionRead', 'ReceiptReprint']) });
    expect(btn('request-reversal')).toHaveLength(0);
    expect(line()).toBeNull();
  });

  it('a transaction read failure is still the error state with its own retry, not the permissions line', async () => {
    await mount({ transaction: { status: 500, json: { error: { code: 'serverError', message: 'x' } } } });
    expect(state()).toBe('error');
    expect(btn('retry')).toHaveLength(1);
    expect(line()).toBeNull();
  });
});
