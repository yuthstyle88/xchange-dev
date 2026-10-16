// Issue #2244: Desk transaction — after a successful reversal request, a failed reload
// preserves "requested" and the transaction on screen, shows the reload error, and
// does not switch the screen to the error state or allow a second request.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../shared/i18n';
import type { ReceiptDocumentViewData } from '../../shared/api/generated/receipt_document';
import type { ReversalSummaryDto } from '../../shared/api/generated/reversal';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import type { TransactionDetailDto, TransactionViewData } from '../../shared/api/generated/transaction';
import type { TransactionAuditViewData } from '../../shared/api/generated/transaction_audit';
import { th } from './i18n';
import { TransactionScreen } from './TransactionScreen';

type ReversalStatus = 'REQUESTED' | 'APPROVED' | 'POSTED' | 'REJECTED' | 'EXPIRED';

const TX_ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';
const PATH = `/desk/transactions/${TX_ID}`;
const RECEIPT_HTML = '<!doctype html><html><body>ใบเสร็จ XC-0001 35,120.00</body></html>';

function me(permissions: string[]): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    roles: [],
    branchId: 'MAIN',
    branchName: 'สาขาทดสอบ',
    permissions,
  };
}
const ALL = ['TransactionRead', 'ReceiptReprint', 'ReversalRequest'];

function completed(over: Partial<TransactionDetailDto> = {}): TransactionDetailDto {
  return {
    id: TX_ID,
    status: 'COMPLETED',
    documentRef: 'XC-0001',
    branchId: 'MAIN',
    customerId: 'cust-1',
    quoteId: null,
    quoteExpiresAt: null,
    lines: [
      {
        lineNo: 1,
        currency: 'USD',
        side: 'BUY',
        quantity: '1000.0000',
        rateSnapshot: '35.12000000',
        unitSize: '1.0000',
        thbAmount: '35120.00',
        feeThb: '0.00',
        settleThb: '35120.00',
      },
    ],
    payment: {
      method: 'CASH',
      reference: '',
      account: '',
      slipFileKey: null,
      amountThb: '35120.00',
      bankFeeThb: '0.00',
      paidAt: '2026-10-11T03:00:00Z',
    },
    receipts: [{ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z' }],
    approvalRequest: null,
    completedAt: '2026-10-11T03:00:05Z',
    cancelledAt: null,
    cancelReason: null,
    reversal: null,
    ...over,
  };
}

function reversal(status: ReversalStatus, over: Partial<ReversalSummaryDto> = {}): ReversalSummaryDto {
  return {
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
  };
}

const AUDIT: TransactionAuditViewData = {
  events: [
    { at: '2026-10-11T02:59:30Z', actorId: 'u-mgr', actorRole: 'ROLE_A', action: 'approval.approved' },
  ],
};

type Call = { method: string; url: string; path: string; query: URLSearchParams; body: string | null; key: string | null };
type Reply = { status: number; json?: unknown; network?: true; text?: string };

interface World {
  me: StaffMeView | Reply;
  transaction: TransactionViewData | Reply;
  audit: TransactionAuditViewData | Reply;
  receipt: ReceiptDocumentViewData;
  posts: Reply[];
  afterPost?: TransactionViewData | Reply;
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
      if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') return view(world.me);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction') return view(world.transaction);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction_audit') return view(world.audit);
      if (method === 'GET' && u.pathname === '/api/v3/views/receipt_document') return view(world.receipt);
      if (method === 'POST' && u.pathname === `/api/transactions/${TX_ID}/reversal-requests`) {
        const r = world.posts[Math.min(postCount, world.posts.length - 1)]!;
        postCount++;
        const res = answer(r);
        if (res.ok && world.afterPost !== undefined) world.transaction = world.afterPost;
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
    transaction: { transaction: completed() },
    audit: AUDIT,
    receipt: { receipt: { number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z', html: RECEIPT_HTML } },
    posts: [{ status: 200, json: { id: 'rv-0001', status: 'REQUESTED' } }],
    ...over,
  };
  installFetch();
  dispose = render(() => <TransactionScreen pathname={PATH} search={search} />, host);
  await settle();
  return host;
}

const gets = (name: string) => calls.filter((c) => c.method === 'GET' && c.path === `/api/v3/views/${name}`);
const btn = (action: string) => host.querySelectorAll<HTMLButtonElement>(`button[data-action="${action}"]`);
const state = () => host.querySelector('[data-state]')?.getAttribute('data-state');
const text = () => host.textContent ?? '';
const messageText = () => (host.querySelector('.xc-dtx-message')?.textContent ?? '').trim();

async function click(el: Element | null | undefined) {
  expect(el, 'the button is there').toBeTruthy();
  (el as HTMLElement).click();
  await settle();
}

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

describe('Desk transaction reversal request reload handling (#2244)', () => {
  it('Request succeeds, reload succeeds: "requested" shown, transaction refreshed (and if subsequent refresh after rejected reversal fails reload, keeps requested)', async () => {
    // 1. Request succeeds and reload succeeds with REJECTED so we can also verify reload-fail on a second request
    await mount({ afterPost: { transaction: completed({ reversal: reversal('REJECTED') }) } });
    await requestReversal('ลูกค้าขอยกเลิก');

    expect(state()).toBe('ready');
    expect(messageText()).toBe(th.requested);
    expect(text()).toContain('REJECTED');
    expect(gets('transaction')).toHaveLength(2);

    // 2. Second request succeeds, reload fails -> "requested" still shown, screen stays ready
    world.afterPost = { status: 500, json: { error: { code: 'serverError', message: 'reload failed' } } };
    await requestReversal('ขออีกครั้ง');

    expect(state()).toBe('ready');
    expect(messageText()).toContain(th.requested);
    expect(messageText()).toContain(errorText('serverError', 'reload failed'));
  });

  it('Request succeeds, reload fails: "requested" still shown, transaction still shown, the reload error also visible; screen not in the error state, and request button is hidden', async () => {
    await mount({
      afterPost: { status: 500, json: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } },
    });

    expect(state()).toBe('ready');
    expect(btn('request-reversal')).toHaveLength(1);

    await requestReversal('ลูกค้าขอยกเลิก');

    expect(state()).toBe('ready');
    expect(state()).not.toBe('error');
    expect(btn('retry')).toHaveLength(0);
    expect(messageText()).toContain(th.requested);
    expect(messageText()).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(text()).toContain('XC-0001');
    expect(text()).toContain('35120.00');
    expect(btn('request-reversal'), 'must not offer a second request while reversal is pending').toHaveLength(0);
  });

  it('Request refused: wordedError shown and no reload; when next request succeeds with reload failure, keeps "requested" and transaction', async () => {
    await mount({
      posts: [
        { status: 409, json: { error: { code: 'reversalWindowClosed', message: 'เกินเวลาที่กำหนด' } } },
        { status: 200, json: { id: 'rv-0001', status: 'REQUESTED' } },
      ],
      afterPost: { status: 0, network: true },
    });

    const readsBefore = gets('transaction').length;
    await requestReversal('ลูกค้าขอยกเลิก');

    expect(messageText()).toBe(errorText('reversalWindowClosed', 'เกินเวลาที่กำหนด'));
    expect(gets('transaction').length).toBe(readsBefore);
    expect(state()).toBe('ready');
    expect(btn('request-reversal')).toHaveLength(1);

    // Retry succeeds on POST, reload fails on network -> keeps requested and transaction
    await requestReversal('ลูกค้าขอยกเลิก');
    expect(state()).toBe('ready');
    expect(messageText()).toContain(th.requested);
    expect(messageText()).toContain(errorText('network', ''));
    expect(text()).toContain('XC-0001');
    expect(btn('request-reversal')).toHaveLength(0);
  });

  it('First load fails: error state as today; after retry succeeds, a reversal request whose reload fails stays ready with "requested"', async () => {
    await mount({
      transaction: { status: 500, json: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } },
      afterPost: { status: 500, json: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } },
    });

    // First load fails -> error state
    expect(state()).toBe('error');
    expect(btn('retry')).toHaveLength(1);
    expect(messageText()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));

    // Retry succeeds -> ready
    world.transaction = { transaction: completed() };
    await click(btn('retry')[0]);
    expect(state()).toBe('ready');

    // Request succeeds, reload fails -> stays ready with "requested" + reload error
    await requestReversal('ลูกค้าขอยกเลิก');
    expect(state()).toBe('ready');
    expect(messageText()).toContain(th.requested);
    expect(messageText()).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(text()).toContain('XC-0001');
  });

  it('Post-request reload held in flight disables refresh; when a newer load answers OK (APPROVED) before the older reload answers OK (REQUESTED), the newest answer stays on screen', async () => {
    await mount();

    let txCallCount = 0;
    let resolveOlderReload!: (res: Response) => void;
    const olderReloadPromise = new Promise<Response>((resolve) => {
      resolveOlderReload = resolve;
    });

    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(url, 'http://xc.test');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && u.pathname === '/api/v3/views/transaction') {
          txCallCount += 1;
          if (txCallCount === 1) {
            return olderReloadPromise;
          }
          return view({ transaction: completed({ reversal: reversal('APPROVED') }) });
        }
        return baseFetch(input, init);
      }),
    );

    await requestReversal('ลูกค้าขอยกเลิก');
    expect(txCallCount).toBe(1);
    const refreshBtn = btn('refresh')[0]!;
    const wasRefreshDisabledDuringReload = refreshBtn.disabled;

    // Trigger a newer load while the post-request reload is still held in flight
    refreshBtn.disabled = false;
    await click(refreshBtn);

    expect(txCallCount).toBe(2);
    expect(state()).toBe('ready');
    expect(text()).toContain('APPROVED');

    // Now let the older post-request reload finish last with stale REQUESTED state
    resolveOlderReload(view({ transaction: completed({ reversal: reversal('REQUESTED') }) }));
    await settle();

    expect(state()).toBe('ready');
    expect(text()).toContain('APPROVED');
    expect(text()).not.toContain('REQUESTED');
    expect(messageText()).toBe('');
    expect(wasRefreshDisabledDuringReload, 'refresh button is disabled while post-request reload runs').toBe(true);
  });

  it('Post-request reload held in flight: when a newer load answers OK (APPROVED) before the older reload answers 500, the newest answer stays on screen without error', async () => {
    await mount();

    let txCallCount = 0;
    let resolveOlderReload!: (res: Response) => void;
    const olderReloadPromise = new Promise<Response>((resolve) => {
      resolveOlderReload = resolve;
    });

    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(url, 'http://xc.test');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && u.pathname === '/api/v3/views/transaction') {
          txCallCount += 1;
          if (txCallCount === 1) {
            return olderReloadPromise;
          }
          return view({ transaction: completed({ reversal: reversal('APPROVED') }) });
        }
        return baseFetch(input, init);
      }),
    );

    await requestReversal('ลูกค้าขอยกเลิก');
    expect(txCallCount).toBe(1);
    const refreshBtn = btn('refresh')[0]!;
    const wasRefreshDisabledDuringReload = refreshBtn.disabled;

    // Trigger a newer load while the post-request reload is still held in flight
    refreshBtn.disabled = false;
    await click(refreshBtn);

    expect(txCallCount).toBe(2);
    expect(state()).toBe('ready');
    expect(text()).toContain('APPROVED');

    // Now let the older post-request reload fail last with 500
    resolveOlderReload(json(500, { error: { code: 'serverError', message: 'stale reload 500' } }));
    await settle();

    expect(state()).toBe('ready');
    expect(text()).toContain('APPROVED');
    expect(messageText()).toBe('');
    expect(wasRefreshDisabledDuringReload, 'refresh button is disabled while post-request reload runs').toBe(true);
  });
});
