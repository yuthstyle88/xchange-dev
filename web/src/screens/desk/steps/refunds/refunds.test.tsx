// DK-L #1346: the desk's pending-refunds step, beside the exchange flow (design `docs/design/xchange-v3-wave3-desk.md`
// §5.5.3, behaviour table §5.5.4 RF-9 … RF-19; owner answers M-R1 A, M-R2 A, M-R3 A). Written before the
// implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller claude/5edf7bb5's
// order). Do not edit, delete or weaken; add tests in a file of your own.
//
// Every test mounts the real `/desk` page through `main.tsx`, as the page's entry runs it (the #539 lesson), against a
// fake server that answers the session (`me`), the desk config, `staff_me`, `desk_refunds`, RV-3's post and TD-2's
// `receipt_document`. RV-3 #684 (`POST /api/reversals/{id}/post`) is not merged yet, so its call is matched by path
// only; its answer is RV-2's `ReversalSummaryDto` with the new `documentNumber` (#684 §8).
//
// The test's handles ([proposed]; the design names no selectors):
//   - the nav item: one `button[data-desk-task="refunds"]` in the desk nav, **outside** `ol.steps` (frame tests P1/P2/P6
//     count `ol.steps li` as the exchange steps only), reading `คืนเงินที่รอบันทึก (n)` (RF-17);
//   - the open step: `[data-section="refunds"]` in the panel; while it is open the exchange step's body is not shown;
//     `button[data-action="close"]` closes it and the exchange step comes back (RF-18); `button[data-action="refresh"]`;
//   - a row: `[data-reversal="<id>"]`, its B04 link `a[href="/desk/transactions/<originalTransactionId>"]`, and
//     `button[data-action="post"][data-id="<id>"]`; the confirm is S-3's `ConfirmDialog` (its last button confirms);
//   - after a success: the document number in the step, and `button[data-action="print"]`;
//   - an unknown outcome: `button[data-action="retry"]`.
//
// Readings of the design, stated so they can be argued on #1346:
//   - `desk_refunds` is read **once when the page loads** (so the nav item can show n, RF-17), **not again** when the step
//     is opened, once after a successful post (RF-15) and on the refresh button. RF-10's "exactly one GET" is counted
//     over load + open + 10 minutes.
//   - RF-18's "step 2": the desk is on the step it opens at (01, the rate step) with its fields typed, because reaching
//     step 02 needs a live quote. What is pinned is the same: the step and every typed field come back unchanged, and
//     the step reads or writes nothing of the exchange.
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
  type MeRole,
  type MeView,
  type ReversalSummaryDto,
} from '../../../../shared/testing/fixtures';
import { setLang } from '../../../../shared/i18n';

interface Texts {
  th: Record<string, string>;
  en: Record<string, string>;
  [other: string]: unknown;
}
const textFiles = import.meta.glob<Texts>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>('./*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });

function texts(): Texts {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/desk/steps/refunds/i18n.ts exists').toBeDefined();
  return m!;
}

const NAV_LABEL = 'คืนเงินที่รอบันทึก';
const ERROR_CODES = ['reversalWindowClosed', 'noOpenShift', 'stockInsufficient', 'staleVersion', 'idempotencyConflict'];

// ── fixtures ────────────────────────────────────────────────────────────────────────────────────────────────────

const CONFIG = {
  branchId: 'MAIN',
  branchName: 'XC-B',
  shiftOpen: true,
  adminUrl: '/?branch=MAIN',
  rates: [{ code: 'USD', unitSize: '1', buy: '32.10000000', sell: '32.90000000', publication: 'pub-1' }],
  paymentMethods: [],
};

function me(permissions: string[], roles: MeRole[] = []) {
  return makeMe({ id: 'u-1', name: 'Ploy', roles, permissions, branch: 'MAIN', branchName: 'XC-B', branches: [] });
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
const postCalls = () => calls.filter((c) => c.method === 'POST' && c.path.startsWith('/api/reversals/'));

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

/** Post → confirm in S-3's ConfirmDialog (its last button confirms). */
async function postAndConfirm(id: string): Promise<void> {
  await click(action('post', id)[0]);
  const dialog = document.querySelector('dialog');
  expect(dialog, "S-3's ConfirmDialog is open").not.toBeNull();
  const buttons = dialog!.querySelectorAll('button');
  await click(buttons[buttons.length - 1] as HTMLButtonElement);
}

// ── RF-9: who sees the step ─────────────────────────────────────────────────────────────────────────────────────

describe('RF-9: the step is shown only to ReversalRequest on OWNER_DESK', () => {
  it('without ReversalRequest: no nav item, and desk_refunds is never called', async () => {
    await boot({ me: me(['TransactionRead', 'ExchangeTransact']) });
    expect(navItem()).toBeNull();
    expect(reads('desk_refunds')).toEqual([]);
  });

  // Retired by G-ME (docs/design/web-silent-failures.md §5, the #1719 standard): "when me fails: no nav item" was the
  // silent hide. A failed `me` read is now worded and the nav item is shown disabled with the reason (desk_refunds is
  // still never called); see refunds.me-error.test.tsx.

  it('on STAFF_DESK (/staff/desk): no nav item, and desk_refunds is never called (O-8)', async () => {
    await boot({}, '/staff/desk');
    expect(navItem()).toBeNull();
    expect(reads('desk_refunds')).toEqual([]);
  });

  it('with ReversalRequest on OWNER_DESK: the nav item is there, outside the exchange step bar', async () => {
    await boot();
    expect(navItem()).not.toBeNull();
    expect(app.querySelector('ol.steps button[data-desk-task="refunds"]'), 'not an item of ol.steps').toBeNull();
  });

  it('owner with ReversalRequest on me (while staff_me answers 401): task visible, desk_refunds read, no staff_me request', async () => {
    await boot({
      me: me(
        ['ReversalRequest'],
        [
          { role: 'EXECUTIVE', branchId: null },
          { role: 'ADMIN', branchId: null },
        ],
      ),
    });
    expect(navItem()).not.toBeNull();
    expect(reads('desk_refunds')).toHaveLength(1);
    expect(reads('staff_me')).toEqual([]);
  });
});

// ── RF-10: one read, no polling ─────────────────────────────────────────────────────────────────────────────────

describe('RF-10: the step reads desk_refunds once and never on a timer', () => {
  it('load + open is exactly one GET desk_refunds; 10 minutes of fake time add none', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval', 'clearTimeout', 'clearInterval'] });
    await boot();
    await open();
    expect(reads('desk_refunds')).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(reads('desk_refunds'), 'no read on a timer').toHaveLength(1);
  });

  it('the refresh button reads it once more', async () => {
    await boot();
    await open();
    await click(action('refresh')[0]);
    expect(reads('desk_refunds')).toHaveLength(2);
  });
});

// ── RF-11: the row as the server sent it ────────────────────────────────────────────────────────────────────────

describe('RF-11: every row shows the server strings unchanged and links to B04', () => {
  it('receipt number, sides, currencies, quantities, THB, payment methods, reason; the B04 link', async () => {
    await boot();
    await open();
    for (const row of ROWS) {
      const el = app.querySelector(`[data-reversal="${row.reversal.id}"]`);
      expect(el, `row ${row.reversal.id}`).not.toBeNull();
      const t = text(el);
      const figures = [
        row.receiptNumber,
        row.reversal.reason,
        row.reversal.amount,
        ...row.lines.flatMap((l) => [l.shopSide, l.currency, l.quantity, l.settleThb]),
        ...row.payments.flatMap((p) => [p.method, p.amountThb]),
      ];
      for (const f of figures) expect(t, `${row.reversal.id} shows ${f}`).toContain(f);
      expect(
        el!.querySelector(`a[href="/desk/transactions/${row.reversal.originalTransactionId}"]`),
        'the B04 link',
      ).not.toBeNull();
    }
  });

  it('the source does no arithmetic on a figure and names no role', () => {
    const own = Object.entries(sources).filter(([p]) => !p.includes('.test.'));
    expect(own.length, 'the folder has source files').toBeGreaterThan(0);
    const role = /['"`](EXECUTIVE|ADMIN|RATE_MANAGER|BRANCH_MANAGER|TELLER|FINANCE|MARKETING|VERIFICATION_REVIEWER|OWNER|MANAGER|FRONT|BACKEND)['"`]/;
    for (const [path, src] of own) {
      src.split('\n').forEach((line, i) => {
        expect(/Number\(|parseFloat|parseInt|Math\.|toFixed/.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
        expect(role.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });

  it('no file is named step.tsx (M-R1 A: beside the flow, not collected as an exchange step)', () => {
    expect(Object.keys(sources)).not.toContain('./step.tsx');
  });
});

// ── RF-12 … RF-15: the post ─────────────────────────────────────────────────────────────────────────────────────

describe('RF-12: post → confirm sends POST /api/reversals/<id>/post {} with a key', () => {
  it('one POST, body {}, a non-empty Idempotency-Key', async () => {
    await boot();
    await open();
    await postAndConfirm('rv-1');
    const p = postCalls();
    expect(p).toHaveLength(1);
    expect(p[0]!.path).toBe('/api/reversals/rv-1/post');
    expect(JSON.parse(p[0]!.body ?? 'null')).toEqual({});
    expect(p[0]!.key, 'an Idempotency-Key').toBeTruthy();
  });

  it('cancelling the confirm sends nothing', async () => {
    await boot();
    await open();
    await click(action('post', 'rv-1')[0]);
    const buttons = document.querySelector('dialog')!.querySelectorAll('button');
    await click(buttons[0] as HTMLButtonElement);
    expect(postCalls()).toEqual([]);
  });
});

describe('RF-13: an unknown outcome retries with the same key and body', () => {
  it('a network failure, then retry: the same key and a byte-equal body', async () => {
    await boot({ posts: ['network', { status: 200, body: posted('rv-1') }] });
    await open();
    await postAndConfirm('rv-1');
    await click(action('retry')[0]);
    const p = postCalls();
    expect(p).toHaveLength(2);
    expect(p[1]!.key).toBe(p[0]!.key);
    expect(p[1]!.body).toBe(p[0]!.body);
  });

  it('a 5xx or a body that is not JSON is unknown too, and offers the retry', async () => {
    for (const reply of [{ status: 503, text: 'busy' }, { status: 200, text: 'not json' }]) {
      vi.resetModules();
      document.body.innerHTML = '';
      app = document.createElement('div');
      app.id = 'app';
      document.body.appendChild(app);
      await boot({ posts: [reply] });
      await open();
      await postAndConfirm('rv-1');
      expect(action('retry').length, `retry offered after ${JSON.stringify(reply)}`).toBe(1);
    }
  });
});

describe('RF-14: a refusal code is shown as t(error.<code>), TH and EN, and nothing else is read', () => {
  it.each(ERROR_CODES)('%s', async (code) => {
    const { th, en } = texts();
    expect(th[`error.${code}`], `th error.${code}`).toBeTruthy();
    expect(en[`error.${code}`], `en error.${code}`).toBeTruthy();
    await boot({ posts: [{ status: 409, body: { error: { code, message: 'server words' } } }] });
    await open();
    const before = calls.length;
    await postAndConfirm('rv-1');
    expect(text(app)).toContain(th[`error.${code}`]!);
    expect(text(app)).not.toContain('server words');
    expect(calls.slice(before).filter((c) => c.method === 'GET'), 'no other request').toEqual([]);
  });
});

describe('RF-15: a success shows the document number and reads desk_refunds once more', () => {
  it('the document number, then exactly one more read', async () => {
    await boot();
    await open();
    expect(reads('desk_refunds')).toHaveLength(1);
    await postAndConfirm('rv-1');
    expect(text(stepSection())).toContain(POSTED_DOC);
    expect(reads('desk_refunds')).toHaveLength(2);
  });
});

// ── RF-16: texts ────────────────────────────────────────────────────────────────────────────────────────────────

describe('RF-16: the folder text table', () => {
  it('exports th and en with the same keys, no empty text, and no zh', () => {
    const t = texts();
    expect(Object.keys(t).filter((k) => k !== 'default').sort()).toEqual(['en', 'th']);
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const [k, v] of Object.entries(t.th)) expect(v, `th.${k}`).toBeTruthy();
    for (const [k, v] of Object.entries(t.en)) expect(v, `en.${k}`).toBeTruthy();
  });
});

// ── RF-17: the nav item and its count ───────────────────────────────────────────────────────────────────────────

describe('RF-17: the nav item counts the last read', () => {
  it(`reads "${NAV_LABEL} (n)" with n = the rows of the last read`, async () => {
    await boot();
    expect(text(navItem())).toBe(`${NAV_LABEL} (2)`);
  });

  it('n = 0: the item still shows, and the open step shows an empty state', async () => {
    await boot({ refunds: { rows: [] } });
    expect(text(navItem())).toBe(`${NAV_LABEL} (0)`);
    const s = await open();
    expect(s.querySelectorAll('[data-reversal]').length).toBe(0);
    expect(text(s), 'an empty-state text').not.toBe('');
  });

  it('after a successful post the count follows the new read', async () => {
    await boot();
    await open();
    world.refunds = { rows: [ROWS[1]!] };
    await postAndConfirm('rv-1');
    expect(text(navItem())).toBe(`${NAV_LABEL} (1)`);
  });
});

// ── RF-18: the exchange in progress is untouched ────────────────────────────────────────────────────────────────

describe('RF-18: opening and closing the step leaves the exchange as it was', () => {
  it('the step and its typed fields come back unchanged, and the step bar never moved', async () => {
    await boot();
    const currentStep = () => text(app.querySelector('ol.steps li[aria-current="step"]'));
    const currency = () => app.querySelector<HTMLSelectElement>('select[name="currency"]');
    const amount = () => app.querySelector<HTMLInputElement>('input[name="amount"]');
    expect(currency()).not.toBeNull();
    currency()!.value = 'USD';
    currency()!.dispatchEvent(new Event('change', { bubbles: true }));
    amount()!.value = '150.25';
    amount()!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    const stepBefore = currentStep();

    await open();
    expect(currentStep(), 'the bar does not move while the step is open').toBe(stepBefore);
    expect(currency(), "the exchange step's body is not shown while the refunds step is open").toBeNull();
    await click(action('close')[0]);
    expect(stepSection()).toBeNull();
    expect(currentStep()).toBe(stepBefore);
    expect(currency()!.value).toBe('USD');
    expect(amount()!.value).toBe('150.25');
    expect(
      calls.filter((c) => c.method !== 'GET'),
      'the step wrote nothing of the exchange',
    ).toEqual([]);
  });
});

// ── RF-19: print through ReceiptFrame ───────────────────────────────────────────────────────────────────────────

describe('RF-19: after a success, print reads receipt_document once and prints through ReceiptFrame', () => {
  it('print → one receipt_document?number=<the document> → ReceiptFrame.print(html)', async () => {
    // the module the page will load after vi.resetModules(): import it first, so the spy is on the page's instance
    const { ReceiptFrame } = await import('../../../../shared/receipt/ReceiptFrame');
    const print = vi.spyOn(ReceiptFrame, 'print').mockResolvedValue(undefined);
    await boot();
    await open();
    expect(action('print'), 'no print before a post').toEqual([]);
    await postAndConfirm('rv-1');
    await click(action('print')[0]);
    expect(reads('receipt_document')).toHaveLength(1);
    expect(reads('receipt_document')[0]!.query.get('number')).toBe(POSTED_DOC);
    expect(print).toHaveBeenCalledTimes(1);
    expect(print).toHaveBeenCalledWith(RECEIPT_HTML);
  });

  it('no other print code in the folder', () => {
    for (const [path, src] of Object.entries(sources).filter(([p]) => !p.includes('.test.'))) {
      expect(/contentWindow|<iframe|window\.print|\.print\(\)/.test(src), `${path}: prints only through ReceiptFrame`).toBe(false);
    }
  });
});
