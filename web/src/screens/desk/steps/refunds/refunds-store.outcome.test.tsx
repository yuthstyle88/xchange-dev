// #2189: Desk refunds outcome handling — separate post from readRefunds(), and treat definite 4xx refusals as non-retryable wordedError.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DeskRefundRow, DeskRefunds } from '../../../../shared/api/generated/desk_refunds';
import type { ReversalSummaryDto } from '../../../../shared/api/generated/reversal';
import type { StaffMeView } from '../../../../shared/api/generated/staff_me';
import { setLang, t as sharedT } from '../../../../shared/i18n';
import * as texts from './i18n';

const ME = {
  kind: 'user',
  id: 'u-1',
  role: '',
  branch: 'MAIN',
  name: 'Ploy',
  username: 'ploy',
  branchName: 'XC-B',
  branches: [],
};

const CONFIG = {
  branchId: 'MAIN',
  branchName: 'XC-B',
  shiftOpen: true,
  adminUrl: '/?branch=MAIN',
  rates: [{ code: 'USD', unitSize: '1', buy: '32.10000000', sell: '32.90000000', publication: 'pub-1' }],
  paymentMethods: [],
};

function staffMe(permissions: string[]): StaffMeView {
  return { userId: 'u-1', name: 'Ploy', roles: [], branchId: 'MAIN', branchName: 'XC-B', permissions };
}

function reversal(id: string, original: string, over: Partial<ReversalSummaryDto> = {}): ReversalSummaryDto {
  return {
    id,
    originalTransactionId: original,
    status: 'APPROVED',
    reason: `ลูกค้าขอคืน ${id}`,
    amount: '35120.00',
    requestedBy: 'u-teller',
    requestedAt: '2026-10-11T04:00:00Z',
    approvedAt: '2026-10-11T04:10:00Z',
    postedAt: null,
    documentNumber: null,
    ...over,
  };
}

const ROWS: DeskRefundRow[] = [
  {
    reversal: reversal('rv-1', 'tx-111'),
    receiptNumber: 'XC-0001',
    lines: [{ shopSide: 'BUY', currency: 'USD', quantity: '1000.0000', settleThb: '35120.00' }],
    payments: [{ method: 'CASH', amountThb: '35120.00' }],
  },
  {
    reversal: reversal('rv-2', 'tx-222', { amount: '1.00', reason: 'คืนครึ่ง' }),
    receiptNumber: 'XC-0002',
    lines: [{ shopSide: 'BUY', currency: 'EUR', quantity: '10.0000', settleThb: '1.00' }],
    payments: [{ method: 'CASH', amountThb: '1.00' }],
  },
];

const POSTED_DOC = 'XC-0001-R2';

interface Call {
  method: string;
  path: string;
  query: URLSearchParams;
  body: string | null;
  key: string | null;
}

type Reply = { status: number; body?: unknown; text?: string } | 'network';
type RefundReadReply = DeskRefunds | Reply;

interface World {
  staffMe: StaffMeView;
  refundReads: RefundReadReply[];
  posts: Reply[];
}

let world: World;
let calls: Call[] = [];
let postCount = 0;
let refundReadCount = 0;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data });
}

function install(): void {
  calls = [];
  postCount = 0;
  refundReadCount = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : null;
      calls.push({ method, path: u.pathname, query: u.searchParams, body, key: headers.get('idempotency-key') });
      if (u.pathname === '/api/v3/views/me') return view(ME);
      if (u.pathname === '/api/v3/views/desk_config' || u.pathname === '/api/v3/views/staff_desk_config') {
        return view(CONFIG);
      }
      if (u.pathname === '/api/v3/views/staff_me') {
        return view(world.staffMe);
      }
      if (u.pathname === '/api/v3/views/desk_refunds') {
        const r = world.refundReads[Math.min(refundReadCount, world.refundReads.length - 1)]!;
        refundReadCount++;
        if (r === 'network') throw new TypeError('Failed to fetch');
        if ('rows' in r) return view(r);
        if (r.text !== undefined) return new Response(r.text, { status: r.status });
        return json(r.status, r.body ?? {});
      }
      if (method === 'POST' && /^\/api\/reversals\/[^/]+\/post$/.test(u.pathname)) {
        const r = world.posts[Math.min(postCount, world.posts.length - 1)]!;
        postCount++;
        if (r === 'network') throw new TypeError('Failed to fetch');
        if (r.text !== undefined) return new Response(r.text, { status: r.status });
        return json(r.status, r.body ?? {});
      }
      return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
    }),
  );
}

const reads = (name: string) => calls.filter((c) => c.method === 'GET' && c.path === `/api/v3/views/${name}`);
const postCalls = () => calls.filter((c) => c.method === 'POST' && c.path.startsWith('/api/reversals/'));

function posted(id: string): ReversalSummaryDto {
  return reversal(id, 'tx-111', { status: 'POSTED', postedAt: '2026-10-11T05:00:00Z', documentNumber: POSTED_DOC });
}

let app: HTMLDivElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await vi.advanceTimersByTimeAsync(0);
  }
}

async function boot(over: Partial<World> = {}, pathname = '/desk'): Promise<HTMLDivElement> {
  world = {
    staffMe: staffMe(['ReversalRequest']),
    refundReads: [{ rows: ROWS }],
    posts: [{ status: 200, body: posted('rv-1') }],
    ...over,
  };
  install();
  window.history.replaceState({}, '', pathname);
  await import('../../main');
  await settle();
  return app;
}

beforeAll(async () => {
  vi.useFakeTimers();
  world = {
    staffMe: staffMe([]),
    refundReads: [{ rows: [] }],
    posts: [],
  };
  install();
  app = document.createElement('div');
  app.id = 'app';
  document.body.appendChild(app);
  await import('../../main');
  await settle();
  vi.useRealTimers();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
}, 60_000);

beforeEach(() => {
  setLang('th');
  vi.resetModules();
  vi.useFakeTimers();
  app = document.createElement('div');
  app.id = 'app';
  document.body.appendChild(app);
});

afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  window.history.replaceState({}, '', '/');
  setLang('th');
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const navItem = () => app.querySelector<HTMLButtonElement>('button[data-desk-task="refunds"]');
const stepSection = () => app.querySelector<HTMLElement>('[data-section="refunds"]');
const action = (act: string, id?: string) =>
  [...app.querySelectorAll<HTMLButtonElement>(`button[data-action="${act}"]`)].filter(
    (b) => id === undefined || b.dataset.id === id,
  );

async function click(el: HTMLElement | null | undefined): Promise<void> {
  expect(el, 'the control is there').toBeTruthy();
  el!.click();
  await settle();
}

async function open(): Promise<HTMLElement> {
  await click(navItem());
  const s = stepSection();
  expect(s, 'the refunds step is open').not.toBeNull();
  return s!;
}

async function postAndConfirm(id: string): Promise<void> {
  await click(action('post', id)[0]);
  const dialog = document.querySelector('dialog');
  expect(dialog, "S-3's ConfirmDialog is open").not.toBeNull();
  const buttons = dialog!.querySelectorAll('button');
  await click(buttons[buttons.length - 1] as HTMLButtonElement);
}

describe('Desk refunds post and read outcomes (#2189 §8 & §9)', () => {
  it('row 1: 409 with one of the five listed codes shows t(error.<code>), no Retry, nothing re-read; and a non-listed 409 (e.g. conflict) shows wordedError with no Retry', async () => {
    await boot({
      posts: [
        { status: 409, body: { error: { code: 'reversalWindowClosed', message: 'window closed on server' } } },
        { status: 409, body: { error: { code: 'otherConflict', message: 'already reversed elsewhere' } } },
      ],
    });
    await open();
    const readsBefore = reads('desk_refunds').length;

    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(texts.th['error.reversalWindowClosed']);
    expect(text(stepSection())).not.toContain('window closed on server');
    expect(action('retry')).toEqual([]);
    expect(reads('desk_refunds').length).toBe(readsBefore);

    // Non-listed 409 refusal must also show wordedError with no Retry (cannot be retried as connection error)
    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain('already reversed elsewhere');
    expect(text(stepSection())).not.toContain(texts.th['error.unknown']);
    expect(action('retry')).toEqual([]);
    expect(reads('desk_refunds').length).toBe(readsBefore);
  });

  it('row 2: Network error, 503, or a 200 body that is not JSON shows error.unknown with Retry; Retry resends the same key and body; once succeeded, currentPoster is cleared', async () => {
    await boot({
      posts: ['network', { status: 503, text: 'busy' }, { status: 200, body: posted('rv-1') }],
    });
    await open();

    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(texts.th['error.unknown']);
    expect(action('retry')).toHaveLength(1);

    await click(action('retry')[0]);
    expect(text(stepSection())).toContain(texts.th['error.unknown']);
    expect(action('retry')).toHaveLength(1);

    await click(action('retry')[0]);
    expect(text(stepSection())).toContain(POSTED_DOC);
    expect(action('retry')).toEqual([]);

    const p = postCalls();
    expect(p).toHaveLength(3);
    expect(p[1]!.key).toBe(p[0]!.key);
    expect(p[2]!.key).toBe(p[0]!.key);
    expect(p[1]!.body).toBe(p[0]!.body);
    expect(p[2]!.body).toBe(p[0]!.body);

    // Also verify 200 non-JSON offers retry, and once a retry succeeds, currentPoster is cleared so retry() cannot post again
    const { createRefundsStore } = await import('./refunds-store');
    const { ApiClient } = await import('../../../../shared/api/client');
    world.posts = [{ status: 200, text: 'not json' }, { status: 200, body: posted('rv-1') }];
    postCount = 0;
    const directStore = createRefundsStore(new ApiClient());
    directStore.startConfirm('rv-1');
    await directStore.submitPost();
    expect(directStore.canRetry()).toBe(true);
    expect(directStore.postError()).toBe(texts.th['error.unknown']);
    await directStore.retry();
    expect(directStore.canRetry()).toBe(false);
    const countAfterSuccess = postCalls().length;
    await directStore.retry();
    expect(postCalls().length, 'currentPoster cleared after retry succeeds').toBe(countAfterSuccess);
  });

  it('row 3: 403 roleNotAllowed / 400 validationFailed / non-JSON 429 shows wordedError text, no Retry, no second POST', async () => {
    const { createRefundsStore } = await import('./refunds-store');
    const { ApiClient } = await import('../../../../shared/api/client');

    await boot({
      posts: [
        { status: 403, body: { error: { code: 'roleNotAllowed', message: 'forbidden by role' } } },
        { status: 400, body: { error: { code: 'validationFailed', message: 'invalid reversal state' } } },
        { status: 429, text: 'rate limited by proxy' },
      ],
    });
    await open();

    // 403 roleNotAllowed -> keyed in shared i18n
    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(sharedT('error.roleNotAllowed'));
    expect(text(stepSection())).not.toContain(texts.th['error.unknown']);
    expect(action('retry')).toEqual([]);
    expect(postCalls()).toHaveLength(1);

    // 400 validationFailed -> unkeyed in shared i18n, falls back to server message via wordedError
    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain('invalid reversal state');
    expect(text(stepSection())).not.toContain(texts.th['error.unknown']);
    expect(action('retry')).toEqual([]);
    expect(postCalls()).toHaveLength(2);

    // Non-JSON 429 (4xx httpError) -> gets wordedError and no Retry
    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(sharedT('error.serverError'));
    expect(text(stepSection())).not.toContain(texts.th['error.unknown']);
    expect(action('retry')).toEqual([]);
    expect(postCalls()).toHaveLength(3);

    // Direct store verification that retry() sends no second POST after a 4xx refusal
    const directStore = createRefundsStore(new ApiClient());
    directStore.startConfirm('rv-1');
    await directStore.submitPost();
    const countAfter4xx = postCalls().length;
    await directStore.retry();
    expect(postCalls().length).toBe(countAfter4xx);
  });

  it('row 4: Post 200, then the desk_refunds read fails -> document number shown, wordedError shown, no "connection error", no post Retry; posted row removed so no second POST can be sent', async () => {
    await boot({
      refundReads: [
        { rows: ROWS },
        { status: 500, body: { error: { code: 'serverError', message: 'db down' } } },
      ],
      posts: [{ status: 200, body: posted('rv-1') }],
    });
    await open();
    expect(reads('desk_refunds')).toHaveLength(1);
    expect(action('post', 'rv-1')).toHaveLength(1);

    await postAndConfirm('rv-1');
    expect(postCalls()).toHaveLength(1);
    expect(reads('desk_refunds')).toHaveLength(2);

    const sectionText = text(stepSection());
    expect(sectionText).toContain(POSTED_DOC);
    expect(sectionText).toContain(sharedT('error.serverError'));
    expect(sectionText).not.toContain(texts.th['error.unknown']);
    expect(action('retry')).toEqual([]);

    // The posted reversal row rv-1 is removed locally before the re-read, so its Post button is gone and cannot send a second POST
    expect(app.querySelector('[data-reversal="rv-1"]')).toBeNull();
    expect(action('post', 'rv-1')).toEqual([]);
    action('post', 'rv-1')[0]?.click();
    await settle();
    expect(postCalls()).toHaveLength(1);

    // Also verify on a direct store instance that calling retry() after post 200 + read failure does NOT post again
    const { createRefundsStore } = await import('./refunds-store');
    const { ApiClient } = await import('../../../../shared/api/client');
    const directStore = createRefundsStore(new ApiClient());
    directStore.startConfirm('rv-1');
    await directStore.submitPost();
    expect(directStore.lastPostedDoc()).toBe(POSTED_DOC);
    expect(directStore.canRetry()).toBe(false);
    expect(directStore.postError()).toBe(sharedT('error.serverError'));
    const postsAfterSubmit = postCalls().length;
    await directStore.retry();
    expect(postCalls().length, 'no second POST when retry() is called after post succeeded').toBe(postsAfterSubmit);
  });

  it('row 5: Post 200, read 200 -> document number shown, no error, and exactly one more read; currentPoster is cleared so retry() is a no-op', async () => {
    const { createRefundsStore } = await import('./refunds-store');
    const { ApiClient } = await import('../../../../shared/api/client');
    const store = createRefundsStore(new ApiClient());

    world = {
      staffMe: staffMe(['ReversalRequest']),
      refundReads: [{ rows: ROWS }, { rows: [ROWS[1]!] }],
      posts: [{ status: 200, body: posted('rv-1') }],
    };
    install();

    await store.readRefunds();
    expect(reads('desk_refunds')).toHaveLength(1);

    store.startConfirm('rv-1');
    await store.submitPost();

    expect(store.lastPostedDoc()).toBe(POSTED_DOC);
    expect(store.postError()).toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(reads('desk_refunds')).toHaveLength(2);
    expect(postCalls()).toHaveLength(1);

    // Even if retry() is invoked directly after a successful post, currentPoster was cleared so no second POST happens
    await store.retry();
    expect(postCalls()).toHaveLength(1);
  });

  it('edge case: the read fails after post 200 and the teller presses Refresh later -> the list re-reads normally with no post', async () => {
    await boot({
      refundReads: [
        { rows: ROWS },
        { status: 500, body: { error: { code: 'serverError', message: 'read failed' } } },
        { rows: [ROWS[1]!] },
      ],
      posts: [{ status: 200, body: posted('rv-1') }],
    });
    await open();
    expect(reads('desk_refunds')).toHaveLength(1);

    await postAndConfirm('rv-1');
    expect(postCalls()).toHaveLength(1);
    expect(reads('desk_refunds')).toHaveLength(2);
    expect(text(stepSection())).toContain(POSTED_DOC);
    expect(action('retry')).toEqual([]);

    // Teller presses Refresh later
    await click(action('refresh')[0]);
    expect(postCalls(), 'no POST on refresh').toHaveLength(1);
    expect(reads('desk_refunds')).toHaveLength(3);
    expect(text(navItem())).toBe(`${texts.th.nav} (1)`);
  });
});
