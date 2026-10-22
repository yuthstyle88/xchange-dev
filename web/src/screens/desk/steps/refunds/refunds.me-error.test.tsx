// G-ME, GME-R (`docs/design/web-silent-failures.md` §5): the desk's pending-refunds task when `me` fails.
// Written before the implementation by the ticket author (claude/e8020026's sub-agent). Do not edit, delete or weaken;
// add tests in a file of your own. The harness (the real `/desk` page booted through `main.tsx`, a fake server) is
// copied from refunds.test.tsx (DK-L #1346).
//
// What changes (the #1719 standard, replacing the retired RF-9 row "when me fails: no nav item"): the task stops
// reading `me` itself and uses the page's session store (`sharedSession().me()`, `.meError()`, `.retryMe()`), which
// already reads `me` on this page. On OWNER_DESK, while `me()` is undefined and `meError()` is set:
//   - the nav item `button[data-desk-task="refunds"]` is on screen, **disabled**, with
//     `title = permissionsUnavailableText(meError())`, outside `ol.steps`; the step cannot be opened;
//   - the shared line `[data-permissions-unavailable]` (role="alert") shows in the desk nav;
//   - `desk_refunds` is never read (the permission is unknown);
//   - the line's retry (`button[data-action="permissions-retry"]`) calls `retryMe()`; a good answer removes the line,
//     and the task follows the permission as today (read `desk_refunds` once, enable the nav item).
// A failed poll after a good `me` keeps today's behaviour: `me()` holds the last good answer, so the permissions are
// known and no line shows.
// What stays (guards): with `me` loaded and no `ReversalRequest`, no nav item (hidden, not disabled), no line, no read;
// STAFF_DESK never shows the task (O-8); `desk_refunds` is read once on load and never on a timer (RF-10), even though
// the session polls `me` every 60 s.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeDeskRefundLine,
  makeDeskRefundPayment,
  makeDeskRefundRow,
  makeMe,
  makeReceiptDocumentDto,
  makeReceiptDocumentViewData,
  makeReversalSummaryDto,
  type DeskRefunds,
  type MeView,
  type ReversalSummaryDto,
} from '../../../../shared/testing/fixtures';
import { setLang } from '../../../../shared/i18n';
import { th as shared } from '../../../../shared/i18n/th';

// ── fixtures ────────────────────────────────────────────────────────────────────────────────────────────────────

const CONFIG = {
  branchId: 'MAIN',
  branchName: 'XC-B',
  shiftOpen: true,
  adminUrl: '/?branch=MAIN',
  rates: [{ code: 'USD', unitSize: '1', buy: '32.10000000', sell: '32.90000000', publication: 'pub-1' }],
  paymentMethods: [],
};

function me(permissions: string[]) {
  return makeMe({ id: 'u-1', name: 'Ploy', roles: [], permissions, branch: 'MAIN', branchName: 'XC-B', branches: [] });
}

function reversal(id: string, original: string, over: Partial<ReversalSummaryDto> = {}) {
  return makeReversalSummaryDto({
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
  });
}

/** Figures that are not what arithmetic would give, so a screen that computed or reformatted them shows otherwise. */
const ROWS = [
  makeDeskRefundRow({
    reversal: reversal('rv-1', 'tx-111'),
    receiptNumber: 'XC-0001',
    lines: [
      makeDeskRefundLine({ shopSide: 'BUY', currency: 'USD', quantity: '1000.0000', settleThb: '35120.00' }),
      makeDeskRefundLine({ shopSide: 'SELL', currency: 'JPY', quantity: '0.5000', settleThb: '0.13' }),
    ],
    payments: [
      makeDeskRefundPayment({ method: 'CASH', amountThb: '35000.00' }),
      makeDeskRefundPayment({ method: 'TRANSFER', amountThb: '120.00' }),
    ],
  }),
  makeDeskRefundRow({
    reversal: reversal('rv-2', 'tx-222', { amount: '1.00', reason: 'คืนครึ่ง' }),
    receiptNumber: 'XC-0002',
    lines: [makeDeskRefundLine({ shopSide: 'BUY', currency: 'EUR', quantity: '10.0000', settleThb: '1.00' })],
    payments: [makeDeskRefundPayment({ method: 'CASH', amountThb: '1.00' })],
  }),
];

const POSTED_DOC = 'XC-0001-R2';
const RECEIPT_HTML = '<!doctype html><html><body>ใบคืนเงิน XC-0001-R2</body></html>';

// ── the fake server ─────────────────────────────────────────────────────────────────────────────────────────────

interface Call {
  method: string;
  path: string;
  query: URLSearchParams;
  body: string | null;
  key: string | null;
}
type Reply = { status: number; body?: unknown; text?: string } | 'network';

interface World {
  me: MeView | 'fail';
  refunds: DeskRefunds;
  /** How the post answers, call by call (the last repeats). */
  posts: Reply[];
}

let world: World;
let calls: Call[] = [];
let posts = 0;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data });
}

function install(): void {
  calls = [];
  posts = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : null;
      calls.push({ method, path: u.pathname, query: u.searchParams, body, key: headers.get('idempotency-key') });
      if (u.pathname === '/api/v3/views/me') {
        return world.me === 'fail' ? json(500, { error: { code: 'serverError', message: 'x' } }) : view(world.me);
      }
      if (u.pathname === '/api/v3/views/desk_config' || u.pathname === '/api/v3/views/staff_desk_config') {
        return view(CONFIG);
      }
      if (u.pathname === '/api/v3/views/staff_me') {
        return json(401, { error: { code: 'employeeAuthRequired', message: 'employee auth required' } });
      }
      if (u.pathname === '/api/v3/views/desk_refunds') return view(world.refunds);
      if (u.pathname === '/api/v3/views/receipt_document') {
        const data = makeReceiptDocumentViewData({
          receipt: makeReceiptDocumentDto({ number: POSTED_DOC, revision: 2, issuedAt: '2026-10-11T05:00:00Z', html: RECEIPT_HTML }),
        });
        return view(data);
      }
      if (method === 'POST' && /^\/api\/reversals\/[^/]+\/post$/.test(u.pathname)) {
        const r = world.posts[Math.min(posts, world.posts.length - 1)]!;
        posts++;
        if (r === 'network') throw new TypeError('Failed to fetch');
        if (r.text !== undefined) return new Response(r.text, { status: r.status });
        return json(r.status, r.body ?? {});
      }
      return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
    }),
  );
}

const reads = (name: string) => calls.filter((c) => c.method === 'GET' && c.path === `/api/v3/views/${name}`);

function posted(id: string) {
  return reversal(id, 'tx-111', { status: 'POSTED', postedAt: '2026-10-11T05:00:00Z', documentNumber: POSTED_DOC });
}

// ── mounting the real page ──────────────────────────────────────────────────────────────────────────────────────

let app: HTMLDivElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await vi.advanceTimersByTimeAsync(0);
  }
}

/** Boots `/desk` (or `pathname`) through `main.tsx`, as the page's entry runs it. */
async function boot(over: Partial<World> = {}, pathname = '/desk'): Promise<HTMLDivElement> {
  world = {
    me: me(['ReversalRequest']),
    refunds: { rows: ROWS },
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
    me: me([]),
    refunds: { rows: [] },
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

const navItem = () => app.querySelector<HTMLButtonElement>('button[data-desk-task="refunds"]');
const stepSection = () => app.querySelector<HTMLElement>('[data-section="refunds"]');

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

// ── G-ME ────────────────────────────────────────────────────────────────────────────────────────────────────────

const line = () => app.querySelector<HTMLElement>('[data-permissions-unavailable]');
const retryPermissions = () => app.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');

describe('G-ME: me fails on OWNER_DESK ⇒ the nav item shows disabled with the reason, and the shared line', () => {
  it('the nav item is present, disabled, with the worded reason as title, outside the step bar', async () => {
    await boot({ me: 'fail' });
    const nav = navItem();
    expect(nav, 'the nav item is not hidden').not.toBeNull();
    expect(nav!.disabled).toBe(true);
    expect(nav!.title).toBe(`${shared['shell.permissions.unavailable']} ${shared['error.serverError']}`);
    expect(app.querySelector('ol.steps button[data-desk-task="refunds"]')).toBeNull();
  });

  it('the shared role="alert" line shows in the desk nav', async () => {
    await boot({ me: 'fail' });
    const p = line();
    expect(p).not.toBeNull();
    expect(p!.getAttribute('role')).toBe('alert');
    expect(p!.textContent).toContain(shared['shell.permissions.unavailable']);
    expect(p!.closest('nav.desk-nav'), 'inside the desk nav').not.toBeNull();
    expect(p!.closest('ol.steps'), 'not an item of ol.steps').toBeNull();
  });

  it('desk_refunds is never read, not even after 10 minutes of failing me polls; the step cannot open', async () => {
    await boot({ me: 'fail' });
    navItem()!.click();
    await settle();
    expect(stepSection()).toBeNull();
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    await settle();
    expect(reads('desk_refunds')).toEqual([]);
    expect(navItem()!.disabled).toBe(true);
  });

  it('retry re-reads me through the session at once; with ReversalRequest the line goes, desk_refunds is read once', async () => {
    await boot({ me: 'fail' });
    const before = reads('me').length;
    world.me = me(['ReversalRequest']);
    await click(retryPermissions());
    expect(reads('me').length, 'one more me read').toBe(before + 1);
    expect(line()).toBeNull();
    expect(navItem()!.disabled).toBe(false);
    expect(navItem()!.title).toBe('');
    expect(reads('desk_refunds')).toHaveLength(1);
    await open();
    expect(stepSection()).not.toBeNull();
  });

  it('retry that brings a me without ReversalRequest: the line goes and the nav item is hidden, as today', async () => {
    await boot({ me: 'fail' });
    world.me = me(['TransactionRead']);
    await click(retryPermissions());
    expect(line()).toBeNull();
    expect(navItem()).toBeNull();
    expect(reads('desk_refunds')).toEqual([]);
  });

  it('on STAFF_DESK (/staff/desk) a failing me still shows no refunds nav item and reads nothing (O-8)', async () => {
    await boot({ me: 'fail' }, '/staff/desk');
    expect(navItem()).toBeNull();
    expect(reads('desk_refunds')).toEqual([]);
  });
});

describe('G-ME guards: me loaded ⇒ today’s behaviour', () => {
  it('with ReversalRequest: nav item enabled, no title, no line; desk_refunds read once and not again in 10 minutes', async () => {
    await boot();
    expect(navItem()).not.toBeNull();
    expect(navItem()!.disabled).toBe(false);
    expect(navItem()!.title).toBe('');
    expect(line()).toBeNull();
    expect(reads('desk_refunds')).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    await settle();
    expect(reads('me').length, 'the session polled me meanwhile').toBeGreaterThan(1);
    expect(reads('desk_refunds')).toHaveLength(1);
  });

  it('without ReversalRequest: no nav item (hidden, not disabled), no line, no desk_refunds', async () => {
    await boot({ me: me(['TransactionRead', 'ExchangeTransact']) });
    expect(navItem()).toBeNull();
    expect(line()).toBeNull();
    expect(reads('desk_refunds')).toEqual([]);
  });

  it('a failed poll after a good me keeps the nav item enabled and shows no line (me() holds the last answer)', async () => {
    await boot();
    world.me = 'fail';
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(navItem()).not.toBeNull();
    expect(navItem()!.disabled).toBe(false);
    expect(line()).toBeNull();
    expect(reads('desk_refunds')).toHaveLength(1);
  });
});
