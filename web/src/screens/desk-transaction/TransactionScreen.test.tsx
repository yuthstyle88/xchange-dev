// TD-3 #1291: the B04 screen `/desk/transactions/{id}` (design `docs/design/blueprint/desk-transaction-detail.md`
// §4.5, D1–D11). Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE
// lane controller claude/5edf7bb5's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// What the screen reads (D2): `staff_me`, `transaction?id=` (TX-10 #678 + RV-4 #685) and `transaction_audit?id=`
// (TD-2), once on open, once after its own command succeeds, and on the refresh button. No timer.
// What it sends: the reversal request `POST /api/transactions/<id>/reversal-requests` `{ reason }` through S-3's
// `command.ts` (one `Idempotency-Key` kept until success); reprint is a GET of `receipt_document?number=` (TD-2) printed
// through `ReceiptFrame.print(html)`. There is no post button: an APPROVED reversal shows *บันทึกคืนเงินที่หน้าเดสก์*
// (owner Q-1 B, D6).
//
// Answer types: the generated `web/src/shared/api/generated/{transaction,reversal,receipt_document,transaction_audit}.ts`
// of TX-10 #678, RV-4 #685 and TD-2 #1294 (merged); the fixtures are typed by them.
//
// The test's handle on the screen ([proposed], stated so it can be argued; the design names no selectors):
//   - `TransactionScreen({ pathname, search })` from `./TransactionScreen` (the stub's signature);
//   - its root carries `data-state` = `loading` | `ready` | `not-found` | `error`;
//   - its buttons carry `data-action` = `refresh` | `retry` | `reprint` (with `data-number`) | `request-reversal`;
//   - the reason dialog is S-3's `ReasonDialog` (its <input> and its last button, the confirm);
//   - `ReceiptFrame.print` is called through the `ReceiptFrame` object, so a spy sees it.
// Every text is checked against what the server sent, the S-3/i18n tables or the owner's verbatim line.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../shared/i18n';
import {
  makeMe,
  makeReceiptDocumentDto,
  makeReceiptDocumentViewData,
  makeReversalSummaryDto,
  makeTransactionAuditEventDto,
  makeTransactionAuditViewData,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeTxApprovalDto,
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
import { th } from './i18n';
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

function me(permissions: string[], roles: MeRole[] = []) {
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

/** Opens S-3's ReasonDialog through the request button, types `reason`, confirms. */
async function requestReversal(reason: string) {
  await click(btn('request-reversal')[0]);
  const input = host.querySelector<HTMLInputElement>('dialog input');
  expect(input, 'S-3 ReasonDialog is open').not.toBeNull();
  input!.value = reason;
  input!.dispatchEvent(new Event('input', { bubbles: true }));
  const buttons = input!.closest('dialog')!.querySelectorAll('button');
  await click(buttons[buttons.length - 1]);
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

// ── reads (D2) ──────────────────────────────────────────────────────────────────────────────────────────────────

describe('reads: three GETs on open, none on a timer, three again on refresh (D2)', () => {
  it('opens with exactly me, transaction {id} and transaction_audit {id}, and nothing else', async () => {
    await mount();
    expect(gets('me')).toHaveLength(1);
    expect(gets('staff_me')).toHaveLength(0);
    expect(gets('transaction')).toHaveLength(1);
    expect(gets('transaction')[0]!.query.get('id')).toBe(TX_ID);
    expect(gets('transaction_audit')).toHaveLength(1);
    expect(gets('transaction_audit')[0]!.query.get('id')).toBe(TX_ID);
    expect(calls).toHaveLength(3);
    expect(state()).toBe('ready');
  });

  it("forwards an owner's ?branch= to the reads", async () => {
    await mount({}, '?branch=B2');
    expect(gets('transaction')).toHaveLength(1);
    expect(gets('transaction_audit')).toHaveLength(1);
    expect(gets('transaction')[0]!.query.get('branch')).toBe('B2');
    expect(gets('transaction_audit')[0]!.query.get('branch')).toBe('B2');
  });

  it('owner (EXECUTIVE+ADMIN) with ReversalRequest and ReceiptReprint on me sees both actions and makes no staff_me request (#3963)', async () => {
    await mount({
      me: me(
        ['TransactionRead', 'ReceiptReprint', 'ReversalRequest'],
        [
          { role: 'EXECUTIVE', branchId: null },
          { role: 'ADMIN', branchId: null },
        ],
      ),
    });
    expect(state()).toBe('ready');
    expect(btn('reprint')).toHaveLength(1);
    expect(btn('request-reversal')).toHaveLength(1);
    expect(gets('me')).toHaveLength(1);
    expect(gets('staff_me')).toEqual([]);
  });

  it('makes no request in 10 minutes without a click (no polling)', async () => {
    vi.useFakeTimers();
    await mount();
    const opened = calls.length;
    expect(opened).toBe(3);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(calls.length, 'no read on a timer').toBe(opened);
  });

  it('the refresh button reads the same three views once more', async () => {
    await mount();
    await click(btn('refresh')[0]);
    expect(gets('me')).toHaveLength(2);
    expect(gets('transaction')).toHaveLength(2);
    expect(gets('transaction_audit')).toHaveLength(2);
    expect(calls).toHaveLength(6);
  });
});

// ── states (D11) ────────────────────────────────────────────────────────────────────────────────────────────────

describe('states (D11)', () => {
  it('transaction: null is the not-found state, with no action button', async () => {
    await mount({ transaction: { transaction: null } });
    expect(state()).toBe('not-found');
    expect(btn('reprint')).toHaveLength(0);
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('a view error shows error + retry, and retry repeats the read', async () => {
    await mount({ transaction: { status: 500, json: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } } });
    expect(state()).toBe('error');
    expect(btn('retry')).toHaveLength(1);
    world.transaction = makeTransactionViewData({ transaction: completed() });
    await click(btn('retry')[0]);
    expect(gets('transaction')).toHaveLength(2);
    expect(state()).toBe('ready');
  });

  it('me failing still shows the data, with no action button', async () => {
    await mount({ me: { status: 500, json: { error: { code: 'serverError', message: 'x' } } } });
    expect(state()).toBe('ready');
    expect(text()).toContain('35120.00');
    expect(btn('reprint')).toHaveLength(0);
    expect(btn('request-reversal')).toHaveLength(0);
  });
});

// ── content (D3) ────────────────────────────────────────────────────────────────────────────────────────────────

describe('content: every figure is the server string, unchanged (D3)', () => {
  it('a COMPLETED USD BUY shows quantity, rate, amount, fee and settle with their digits unchanged', async () => {
    await mount();
    for (const s of ['1000.0000', '35.12000000', '35120.00', '0.00', 'USD', 'XC-0001']) {
      expect(text(), s).toContain(s);
    }
  });

  it('DRAFT and CONFIRMED show their status and no receipts, reprint or request button', async () => {
    for (const status of ['DRAFT', 'CONFIRMED'] as const) {
      await mount({ transaction: makeTransactionViewData({ transaction: completed({ status, documentRef: null, receipts: [], completedAt: null }) }) });
      expect(state(), status).toBe('ready');
      expect(btn('reprint'), status).toHaveLength(0);
      expect(btn('request-reversal'), status).toHaveLength(0);
      expect(text(), status).not.toContain('XC-0001');
      dispose?.();
      host.innerHTML = '';
    }
  });

  it("PENDING_APPROVAL shows the approval request's status and no request button", async () => {
    await mount({
      transaction: makeTransactionViewData({
        transaction: completed({
          status: 'PENDING_APPROVAL',
          documentRef: null,
          receipts: [],
          completedAt: null,
          approvalRequest: makeTxApprovalDto({ id: 'ap-1', status: 'PENDING' }),
        }),
      }),
    });
    expect(text()).toContain('PENDING');
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('CANCELLED shows cancelledAt and cancelReason, and no receipts', async () => {
    await mount({
      transaction: makeTransactionViewData({
        transaction: completed({
          status: 'CANCELLED',
          documentRef: null,
          receipts: [],
          completedAt: null,
          cancelledAt: '2026-10-11T05:00:00Z',
          cancelReason: 'ลูกค้าเปลี่ยนใจ',
        }),
      }),
    });
    expect(text()).toContain('ลูกค้าเปลี่ยนใจ');
    expect(btn('reprint')).toHaveLength(0);
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('shows the audit events in the order sent', async () => {
    await mount();
    const t0 = text();
    expect(t0).toContain('approval.approved');
    expect(t0).toContain('reversal.approved');
    expect(t0.indexOf('approval.approved')).toBeLessThan(t0.indexOf('reversal.approved'));
  });
});

// ── reprint (D5) ────────────────────────────────────────────────────────────────────────────────────────────────

describe('reprint: a GET of receipt_document, printed through ReceiptFrame (D5)', () => {
  it('with ReceiptReprint: one button per receipt; click → one GET {number} → ReceiptFrame.print(html); no write', async () => {
    const print = vi.spyOn(ReceiptFrame, 'print').mockResolvedValue();
    await mount();
    expect(btn('reprint')).toHaveLength(1);
    expect(btn('reprint')[0]!.getAttribute('data-number')).toBe('XC-0001');
    await click(btn('reprint')[0]);
    expect(gets('receipt_document')).toHaveLength(1);
    expect(gets('receipt_document')[0]!.query.get('number')).toBe('XC-0001');
    expect(print).toHaveBeenCalledTimes(1);
    expect(print).toHaveBeenCalledWith(RECEIPT_HTML);
    expect(nonGets()).toEqual([]);
  });

  it('without ReceiptReprint: no reprint button', async () => {
    await mount({ me: me(['TransactionRead', 'ReversalRequest']) });
    expect(state()).toBe('ready');
    expect(btn('reprint')).toHaveLength(0);
  });

  it('receipt: null on reprint shows "document not found", prints nothing, and the button stays', async () => {
    const print = vi.spyOn(ReceiptFrame, 'print').mockResolvedValue();
    await mount({ receipt: { receipt: null } });
    await click(btn('reprint')[0]);
    expect(text()).toContain(th.docNotFound);
    expect(print).not.toHaveBeenCalled();
    expect(btn('reprint')).toHaveLength(1);
  });
});

// ── reversal (D6) ───────────────────────────────────────────────────────────────────────────────────────────────

describe('reversal request (D6)', () => {
  it('without ReversalRequest: no request-reversal button', async () => {
    await mount({ me: me(['TransactionRead', 'ReceiptReprint']) });
    expect(state()).toBe('ready');
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('ReversalRequest + COMPLETED + no reversal: request → ReasonDialog → POST {reason} with an Idempotency-Key', async () => {
    await mount();
    expect(btn('request-reversal')).toHaveLength(1);
    await requestReversal('ลูกค้าขอยกเลิก');
    const posts = nonGets();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.method).toBe('POST');
    expect(posts[0]!.path).toBe(`/api/transactions/${TX_ID}/reversal-requests`);
    expect(posts[0]!.body).toBe(JSON.stringify({ reason: 'ลูกค้าขอยกเลิก' }));
    expect(posts[0]!.key, 'an Idempotency-Key').toBeTruthy();
  });

  it('a network failure, then retry: the same key and a byte-equal body', async () => {
    await mount({ posts: [{ status: 0, network: true }, { status: 200, json: { id: 'rv-0001', status: 'REQUESTED' } }] });
    await requestReversal('ลูกค้าขอยกเลิก');
    await requestReversal('ลูกค้าขอยกเลิก');
    const posts = nonGets();
    expect(posts).toHaveLength(2);
    expect(posts[1]!.key).toBe(posts[0]!.key);
    expect(posts[1]!.body).toBe(posts[0]!.body);
  });

  // D6's `t('error.' + code)`, through the repo's `errorText(code, message)`: the shared table's text for the code
  // when it has one, else the server's message, never the bare key.
  it('a refusal (reversalWindowClosed / staleVersion / outOfScope) shows errorText(code, message) and re-reads nothing', async () => {
    for (const code of ['reversalWindowClosed', 'staleVersion', 'outOfScope']) {
      const message = `server says ${code}`;
      await mount({ posts: [{ status: 409, json: { error: { code, message } } }] });
      const readsBefore = calls.filter((c) => c.method === 'GET').length;
      await requestReversal('ลูกค้าขอยกเลิก');
      expect(text(), code).toContain(errorText(code, message));
      expect(text(), `${code}: never the bare key`).not.toContain(`error.${code}`);
      expect(calls.filter((c) => c.method === 'GET').length, `${code}: nothing re-read`).toBe(readsBefore);
      dispose?.();
      host.innerHTML = '';
    }
  });

  it('success re-reads transaction and transaction_audit once each', async () => {
    await mount({ afterPost: makeTransactionViewData({ transaction: completed({ reversal: reversal('REQUESTED') }) }) });
    await requestReversal('ลูกค้าขอยกเลิก');
    expect(gets('transaction')).toHaveLength(2);
    expect(gets('transaction_audit')).toHaveLength(2);
    expect(btn('request-reversal'), 'REQUESTED: no second request').toHaveLength(0);
  });

  it('REQUESTED, APPROVED, POSTED, EXPIRED: no request button; the section shows status, reason and amount', async () => {
    for (const s of ['REQUESTED', 'APPROVED', 'POSTED', 'EXPIRED'] as const) {
      await mount({ transaction: makeTransactionViewData({ transaction: completed({ reversal: reversal(s) }) }) });
      expect(btn('request-reversal'), s).toHaveLength(0);
      expect(text(), s).toContain(s);
      expect(text(), s).toContain('ลูกค้าขอยกเลิก');
      dispose?.();
      host.innerHTML = '';
    }
  });

  it('REJECTED: shown rejected, and the request button is shown again', async () => {
    await mount({ transaction: makeTransactionViewData({ transaction: completed({ reversal: reversal('REJECTED') }) }) });
    expect(text()).toContain('REJECTED');
    expect(btn('request-reversal')).toHaveLength(1);
  });

  it('POSTED: the original figures are unchanged, and receipts list revision 1 and 2 with the document number', async () => {
    await mount({
      transaction: makeTransactionViewData({
        transaction: completed({
          reversal: reversal('POSTED'),
          receipts: [
            makeTxReceiptDto({ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z' }),
            makeTxReceiptDto({ number: 'XC-0001-R2', revision: 2, issuedAt: '2026-10-11T04:20:00Z' }),
          ],
        }),
      }),
    });
    for (const s of ['1000.0000', '35.12000000', '35120.00', 'XC-0001-R2']) expect(text(), s).toContain(s);
    expect(btn('reprint')).toHaveLength(2);
  });

  it('APPROVED: the owner line บันทึกคืนเงินที่หน้าเดสก์, and no post button or post request in any case (Q-1 B)', async () => {
    await mount({ transaction: makeTransactionViewData({ transaction: completed({ reversal: reversal('APPROVED') }) }) });
    expect(th.approvedAtDesk).toBe('บันทึกคืนเงินที่หน้าเดสก์');
    expect(text()).toContain('บันทึกคืนเงินที่หน้าเดสก์');
    expect(btn('post-reversal')).toHaveLength(0);
    for (const b of Array.from(host.querySelectorAll('button'))) {
      expect(b.getAttribute('data-action') ?? '', 'no button posts the refund').not.toMatch(/post/i);
    }
    for (const b of Array.from(host.querySelectorAll('button'))) b.click();
    await settle();
    expect(calls.filter((c) => c.path.startsWith('/api/reversals/')), 'never POST /api/reversals/:id/post').toEqual([]);
  });
});
