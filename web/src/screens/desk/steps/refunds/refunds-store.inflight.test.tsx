// #2428: Desk refunds — the Post button must not start a second post while one is in flight (a second confirm would
// send POST /api/reversals/{id}/post again under a NEW Idempotency-Key and show the server's refusal). Render and mock
// setup copied from the neighbouring refunds-store.outcome.test.tsx.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DeskRefundRow, DeskRefunds } from '../../../../shared/api/generated/desk_refunds';
import type { ReversalSummaryDto } from '../../../../shared/api/generated/reversal';
import type { StaffMeView } from '../../../../shared/api/generated/staff_me';
import { setLang } from '../../../../shared/i18n';
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
  body: string | null;
  key: string | null;
}

/** `hang`: the post stays in flight until the test calls `release()`. */
type Reply = { status: number; body?: unknown; text?: string } | 'network' | 'hang';

interface World {
  staffMe: StaffMeView;
  refundReads: DeskRefunds[];
  posts: Reply[];
}

let world: World;
let calls: Call[] = [];
let postCount = 0;
let refundReadCount = 0;
let hanging: Array<(r: Response) => void> = [];

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
  hanging = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : null;
      calls.push({ method, path: u.pathname, body, key: headers.get('idempotency-key') });
      if (u.pathname === '/api/v3/views/me') return view(ME);
      if (u.pathname === '/api/v3/views/desk_config' || u.pathname === '/api/v3/views/staff_desk_config') {
        return view(CONFIG);
      }
      if (u.pathname === '/api/v3/views/staff_me') return view(world.staffMe);
      if (u.pathname === '/api/v3/views/desk_refunds') {
        const r = world.refundReads[Math.min(refundReadCount, world.refundReads.length - 1)]!;
        refundReadCount++;
        return view(r);
      }
      if (method === 'POST' && /^\/api\/reversals\/[^/]+\/post$/.test(u.pathname)) {
        const r = world.posts[Math.min(postCount, world.posts.length - 1)]!;
        postCount++;
        if (r === 'hang') return new Promise<Response>((resolve) => hanging.push(resolve));
        if (r === 'network') throw new TypeError('Failed to fetch');
        if (r.text !== undefined) return new Response(r.text, { status: r.status });
        return json(r.status, r.body ?? {});
      }
      return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
    }),
  );
}

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

/** Answers every post still in flight with 200 posted, so nothing is left pending after the test. */
async function releaseAll(): Promise<void> {
  const pending = hanging;
  hanging = [];
  for (const resolve of pending) resolve(json(200, posted('rv-1')));
  await settle();
}

async function boot(over: Partial<World> = {}): Promise<HTMLDivElement> {
  world = {
    staffMe: staffMe(['ReversalRequest']),
    refundReads: [{ rows: ROWS }],
    posts: [{ status: 200, body: posted('rv-1') }],
    ...over,
  };
  install();
  window.history.replaceState({}, '', '/desk');
  await import('../../main');
  await settle();
  return app;
}

beforeAll(async () => {
  vi.useFakeTimers();
  world = { staffMe: staffMe([]), refundReads: [{ rows: [] }], posts: [] };
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

afterEach(async () => {
  await releaseAll();
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
const dialog = () => document.querySelector('dialog');
const action = (act: string, id?: string) =>
  [...app.querySelectorAll<HTMLButtonElement>(`button[data-action="${act}"]`)].filter(
    (b) => id === undefined || b.dataset.id === id,
  );
const postButton = (id: string) => action('post', id)[0];

async function click(el: HTMLElement | null | undefined): Promise<void> {
  expect(el, 'the control is there').toBeTruthy();
  el!.click();
  await settle();
}

async function open(): Promise<void> {
  await click(navItem());
  expect(stepSection(), 'the refunds step is open').not.toBeNull();
}

async function postAndConfirm(id: string): Promise<void> {
  await click(postButton(id));
  expect(dialog(), "S-3's ConfirmDialog is open").not.toBeNull();
  const buttons = dialog()!.querySelectorAll('button');
  await click(buttons[buttons.length - 1] as HTMLButtonElement);
}

async function directStore() {
  const { createRefundsStore } = await import('./refunds-store');
  const { ApiClient } = await import('../../../../shared/api/client');
  return createRefundsStore(new ApiClient());
}

describe('Desk refunds: no second post while one is in flight (#2428 §8 & §9)', () => {
  it('row 1: confirm Post on A, then click Post on A again while in flight -> no dialog opens; exactly one POST /api/reversals/A/post', async () => {
    await boot({ posts: ['hang'] });
    await open();

    await postAndConfirm('rv-1');
    expect(postCalls()).toHaveLength(1);
    expect(dialog()).toBeNull();
    expect(postButton('rv-1')!.disabled, 'Post on A is disabled while A is in flight').toBe(true);

    postButton('rv-1')!.click();
    await settle();
    expect(dialog(), 'no dialog opens').toBeNull();

    // The store itself refuses a second confirm of A while A is in flight.
    const store = await directStore();
    store.startConfirm('rv-1');
    void store.submitPost();
    await settle();
    expect(store.posting()).toBe(true);
    store.startConfirm('rv-1');
    expect(store.confirmingId(), 'no dialog opens from the store').toBeNull();
    await store.submitPost();
    await settle();

    const p = postCalls();
    expect(p).toHaveLength(2);
    expect(p.every((c) => c.path === '/api/reversals/rv-1/post')).toBe(true);
    expect(new Set(p.map((c) => c.key)).size, 'one key per store, no second keyed post').toBe(2);
  });

  it('row 2: confirm Post on A, click Post on B while A is in flight -> no dialog opens; B is not posted', async () => {
    await boot({ posts: ['hang'] });
    await open();

    await postAndConfirm('rv-1');
    expect(postCalls()).toHaveLength(1);
    expect(postButton('rv-2')!.disabled, 'Post on B is disabled while A is in flight').toBe(true);

    postButton('rv-2')!.click();
    await settle();
    expect(dialog(), 'no dialog opens').toBeNull();

    const store = await directStore();
    store.startConfirm('rv-1');
    void store.submitPost();
    await settle();
    store.startConfirm('rv-2');
    expect(store.confirmingId(), 'no dialog opens for B').toBeNull();
    await store.submitPost();
    await settle();

    expect(postCalls().some((c) => c.path === '/api/reversals/rv-2/post'), 'B is not posted').toBe(false);
  });

  it("row 3: A's post fails with a network error -> posting clears; Retry resends A with the SAME key", async () => {
    await boot({ posts: ['network', { status: 200, body: posted('rv-1') }] });
    await open();

    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(texts.th['error.unknown']);
    expect(postButton('rv-1')!.disabled, 'posting cleared after the network error').toBe(false);
    expect(postButton('rv-2')!.disabled).toBe(false);
    expect(action('retry')).toHaveLength(1);

    await click(action('retry')[0]);
    const p = postCalls();
    expect(p).toHaveLength(2);
    expect(p[1]!.path).toBe('/api/reversals/rv-1/post');
    expect(p[1]!.key).toBeTruthy();
    expect(p[1]!.key).toBe(p[0]!.key);
    expect(text(stepSection())).toContain(POSTED_DOC);

    world.posts = ['network', { status: 200, body: posted('rv-1') }];
    postCount = 0;
    const store = await directStore();
    store.startConfirm('rv-1');
    await store.submitPost();
    expect(store.posting(), 'posting cleared after the network error').toBe(false);
    expect(store.canRetry()).toBe(true);
    await store.retry();
    expect(store.posting()).toBe(false);
    const q = postCalls();
    expect(q[q.length - 1]!.key).toBe(q[q.length - 2]!.key);
  });

  it("row 4: after a successful post the remaining rows' Post buttons are enabled", async () => {
    await boot({
      refundReads: [{ rows: ROWS }, { rows: [ROWS[1]!] }],
      posts: [{ status: 200, body: posted('rv-1') }],
    });
    await open();

    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(POSTED_DOC);
    expect(postButton('rv-1')).toBeUndefined();
    expect(postButton('rv-2')!.disabled).toBe(false);

    await click(postButton('rv-2'));
    expect(dialog(), 'the next row can be confirmed').not.toBeNull();

    const store = await directStore();
    store.startConfirm('rv-1');
    await store.submitPost();
    expect(store.posting()).toBe(false);
  });

  it('edge: posting is cleared after a 4xx refusal, so another row can be posted', async () => {
    await boot({
      posts: [
        { status: 400, body: { error: { code: 'validationFailed', message: 'invalid reversal state' } } },
        { status: 200, body: posted('rv-2') },
      ],
    });
    await open();

    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain('invalid reversal state');
    expect(postButton('rv-2')!.disabled, 'posting cleared after the refusal').toBe(false);

    await postAndConfirm('rv-2');
    const p = postCalls();
    expect(p).toHaveLength(2);
    expect(p[1]!.path).toBe('/api/reversals/rv-2/post');

    world.posts = [{ status: 400, body: { error: { code: 'validationFailed', message: 'invalid reversal state' } } }];
    postCount = 0;
    const store = await directStore();
    store.startConfirm('rv-1');
    await store.submitPost();
    expect(store.posting()).toBe(false);
    store.startConfirm('rv-2');
    expect(store.confirmingId()).toBe('rv-2');
  });

  it('edge: Cancel on the dialog is unaffected', async () => {
    await boot();
    await open();

    await click(postButton('rv-1'));
    const buttons = dialog()!.querySelectorAll('button');
    await click(buttons[0] as HTMLButtonElement);
    expect(dialog()).toBeNull();
    expect(postCalls()).toEqual([]);
    expect(postButton('rv-1')!.disabled).toBe(false);
    expect(postButton('rv-2')!.disabled).toBe(false);

    const store = await directStore();
    store.startConfirm('rv-1');
    store.cancelConfirm();
    expect(store.confirmingId()).toBeNull();
    expect(store.posting()).toBe(false);
    expect(postCalls()).toEqual([]);
  });
});
