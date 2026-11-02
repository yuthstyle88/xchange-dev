// FE-L #5982: the desk's standing low-cash banner (design docs/design/xchange-v3-wave3-desk.md §5.4c L1-L5 and §5.4e;
// server ledger-inventory.md §5.1.3 item 2, `MyShiftBalance.low`).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere. The fake server and `ready()` follow save.stock-wait.test.ts.
//
// [required] by these tests:
// - `web/src/screens/desk/low-cash.ts` exports `createLowCash(api: ApiClient): LowCash`, called inside a reactive owner,
//   with `low(): string[]` (the codes of the `my_shift` balances with `low: true`, in the view's order) and
//   `reload(): Promise<void>` (reads `GET /api/v3/views/my_shift`, keeps the last good answer, never throws).
//   It also reads when `document` fires `visibilitychange` and `visibilityState` is `visible`, and stops listening when
//   its owner is disposed. No timer: it never reads on its own (L5). A reload asked while one is in flight never runs a
//   second request at the same time, and what `low()` ends on reflects the server after the later ask.
// - `web/src/screens/desk/LowCashBanner.tsx` exports `LowCashBanner(props: { lowCash: { low(): string[] } })`: when
//   `low()` is not empty, one `[data-testid="low-cash"]` with `role="status"` holding `lowCashTitle` (the owner's text)
//   then `lowCashCurrencies` with the codes joined by ", "; nothing when it is empty.
// - `createDeskStore` exposes `lowCash: LowCash` built on the store's own api. It does not read `my_shift` by itself.
// - The save path (`save.ts`) calls `store.lowCash.reload()` after a save ends COMPLETED, and after every
//   409 `stockInsufficient` answer (at complete, §5.4a S1, and at create, S6).
// - `DeskPage` renders the banner once, above the steps, while the shift is open, from `store.lowCash` when the store has
//   one; it calls `store.lowCash.reload()` when the shift turns open (at mount when there is no `shift` prop).
// - The desk holds no level of its own: it shows only the server's `low` (user-set settings, PR #5993).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import {
  makeMyShift,
  makeMyShiftBalance,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  makeTransactionDto,
  makeTransactionLineDto,
  makeTransactionPaymentDto,
} from '../../shared/testing/fixtures';
import { DeskPage } from './DeskPage';
import { createFrameStore } from './frame-store';
import { collectSteps } from './steps';

type LowCash = { low(): string[]; reload(): Promise<void> };
type LowCashModule = { createLowCash: (api: ApiClient) => LowCash };
type BannerModule = { LowCashBanner: (props: { lowCash: { low(): string[] } }) => JSX.Element };
type Store = Record<string, any>;
type StoreModule = { createDeskStore: (deps: { api: ApiClient; path: unknown; me: () => unknown }) => Store };
type PathModule = { OWNER_DESK: Record<string, any>; STAFF_DESK: Record<string, any> };
const lowCashMods = import.meta.glob<LowCashModule>('./low-cash.ts');
const bannerMods = import.meta.glob<BannerModule>('./LowCashBanner.tsx');
const storeMods = import.meta.glob<StoreModule>('./desk-store.ts');
const pathMods = import.meta.glob<PathModule>('./desk-path.ts');

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

// The exact words of §5.4c.
// #6013: restated to the OD wording the owner approved ("ok ข้อความ OD ใช้ได้", #564).
const TITLE_TH = 'เงินในลิ้นชักเหลือน้อย ให้ผู้จัดการเบิก OD จากตู้เซฟ';
const TITLE_EN = 'The drawer is running low. Ask the manager to move cash from the safe';
const CURRENCIES_TH = (c: string) => `สกุลที่เหลือน้อย: ${c}`;
const CURRENCIES_EN = (c: string) => `Low on: ${c}`;

const MY_SHIFT = '/api/v3/views/my_shift';

let disposers: (() => void)[] = [];
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
  vi.restoreAllMocks();
  setLang('th');
});

// ── the fake server ───────────────────────────────────────────────────────────────────────

type Call = { method: string; path: string; key: string | null; body: string; at: number };
type Answer = { status: number; body?: unknown };
type Route = (call: Call, nth: number) => Answer | Promise<Answer>;

function fakeServer() {
  const calls: Call[] = [];
  const routes = new Map<string, Route>();
  const counts = new Map<string, number>();
  let clock = 0;
  let inFlight = 0;
  let maxInFlight = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0];
    const headers = new Headers(init?.headers);
    const body = init?.body === undefined || init?.body === null ? '' : typeof init.body === 'string' ? init.body : '[binary]';
    const call: Call = { method, path, key: headers.get('idempotency-key'), body, at: clock++ };
    calls.push(call);
    const k = `${method} ${path}`;
    const n = counts.get(k) ?? 0;
    counts.set(k, n + 1);
    const route = routes.get(k);
    if (path === MY_SHIFT) maxInFlight = Math.max(maxInFlight, ++inFlight);
    try {
      const a: Answer = route ? await route(call, n) : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
      return new Response(JSON.stringify(a.body ?? {}), { status: a.status, headers: { 'content-type': 'application/json' } });
    } finally {
      if (path === MY_SHIFT) inFlight -= 1;
    }
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    calls,
    maxInFlight: () => maxInFlight,
    on(method: string, path: string, route: Route) {
      routes.set(`${method.toUpperCase()} ${path}`, route);
    },
    callsTo(method: string, path: string): Call[] {
      return calls.filter((c) => c.method === method.toUpperCase() && c.path === path);
    },
  };
}

const view = (data: unknown): Answer => ({ status: 200, body: { version: 1, changed: true, data } });
const failed: Answer = { status: 500, body: { error: { code: 'serverError', message: 'x' } } };
const refusal = (status: number, code: string, message = 'refused'): Answer => ({ status, body: { error: { code, message } } });

/** `my_shift` with the given codes low, and `fine` codes not low. */
function shift(low: string[], fine: string[] = []) {
  return makeMyShift({
    id: 'sh-1',
    status: 'OPEN',
    balances: [
      ...fine.map((code) => makeMyShiftBalance({ code, opening: '1000', expected: '900', lowCashLevel: '100', low: false })),
      ...low.map((code) => makeMyShiftBalance({ code, opening: '1000', expected: '50', lowCashLevel: '100', low: true })),
    ],
  });
}

function lowCashOn(s: ReturnType<typeof fakeServer>, mod: LowCashModule): LowCash {
  let lc!: LowCash;
  createRoot((dispose) => {
    disposers.push(dispose);
    lc = mod.createLowCash(s.api);
  });
  return lc;
}

function setVisibility(state: 'visible' | 'hidden') {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  document.dispatchEvent(new Event('visibilitychange'));
}

// ── LowCash ───────────────────────────────────────────────────────────────────────────────

describe('FE-L LowCash reader (low-cash.ts)', () => {
  it('L1 the codes with low: true, in the view order', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, () =>
      view(
        makeMyShift({
          balances: [
            makeMyShiftBalance({ code: 'THB', low: true, lowCashLevel: '5000' }),
            makeMyShiftBalance({ code: 'USD', low: false, lowCashLevel: '100' }),
            makeMyShiftBalance({ code: 'EUR', low: true, lowCashLevel: '50' }),
          ],
        }),
      ),
    );
    const lc = lowCashOn(s, mod);
    expect(lc.low()).toEqual([]);
    await lc.reload();
    expect(lc.low()).toEqual(['THB', 'EUR']);
  });

  it('L2 none low, or no shift: empty', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, (_c, n) => (n === 0 ? view(shift([], ['THB', 'USD'])) : view(null)));
    const lc = lowCashOn(s, mod);
    await lc.reload();
    expect(lc.low()).toEqual([]);
    await lc.reload();
    expect(lc.low()).toEqual([]);
  });

  it('L3 the next read after a top-up clears it', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, (_c, n) => (n === 0 ? view(shift(['THB'])) : view(shift([], ['THB']))));
    const lc = lowCashOn(s, mod);
    await lc.reload();
    expect(lc.low()).toEqual(['THB']);
    await lc.reload();
    expect(lc.low()).toEqual([]);
  });

  it('L4 a failed read keeps the last answer and does not throw', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, (_c, n) => (n === 0 ? view(shift(['USD'])) : failed));
    const lc = lowCashOn(s, mod);
    await lc.reload();
    await expect(lc.reload()).resolves.toBeUndefined();
    expect(lc.low()).toEqual(['USD']);
  });

  it('L4 a failed first read: empty, no throw', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, () => failed);
    const lc = lowCashOn(s, mod);
    await expect(lc.reload()).resolves.toBeUndefined();
    expect(lc.low()).toEqual([]);
  });

  it('L5 no trigger, no request: nothing is read on a timer', async () => {
    vi.useFakeTimers();
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, () => view(shift(['THB'])));
    const lc = lowCashOn(s, mod);
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(s.callsTo('GET', MY_SHIFT)).toHaveLength(0);
    await lc.reload();
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(s.callsTo('GET', MY_SHIFT)).toHaveLength(1);
  });

  it('the page becoming visible reads again; becoming hidden does not; disposal stops it', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    s.on('GET', MY_SHIFT, () => view(shift(['THB'])));
    let lc!: LowCash;
    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      lc = mod.createLowCash(s.api);
    });
    setVisibility('hidden');
    await Promise.resolve();
    expect(s.callsTo('GET', MY_SHIFT)).toHaveLength(0);
    setVisibility('visible');
    await vi.waitFor(() => expect(lc.low()).toEqual(['THB']));
    expect(s.callsTo('GET', MY_SHIFT)).toHaveLength(1);
    dispose();
    setVisibility('visible');
    await new Promise((r) => setTimeout(r, 20));
    expect(s.callsTo('GET', MY_SHIFT), 'no read after the owner is disposed').toHaveLength(1);
  });

  it('two triggers at once: never two reads in flight, and the result reflects the later ask', async () => {
    const mod = await load(lowCashMods, './low-cash.ts');
    const s = fakeServer();
    // The server's drawer state at the moment each request arrives.
    let state = shift(['THB']);
    let releaseFirst!: () => void;
    const firstHeld = new Promise<void>((r) => (releaseFirst = r));
    s.on('GET', MY_SHIFT, async (_c, n) => {
      const answer = view(state);
      if (n === 0) await firstHeld;
      return answer;
    });
    const lc = lowCashOn(s, mod);
    const a = lc.reload();
    await vi.waitFor(() => expect(s.callsTo('GET', MY_SHIFT)).toHaveLength(1));
    state = shift([], ['THB']); // topped up after the first request was sent
    const b = lc.reload();
    releaseFirst();
    await Promise.all([a, b]);
    await vi.waitFor(() => expect(lc.low()).toEqual([]));
    expect(s.maxInFlight(), 'at most one read in flight').toBe(1);
    expect(s.callsTo('GET', MY_SHIFT).length).toBeLessThanOrEqual(2);
  });
});

// ── LowCashBanner ─────────────────────────────────────────────────────────────────────────

describe('FE-L LowCashBanner', () => {
  let host: HTMLDivElement;
  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });
  afterEach(() => host.remove());

  async function mountBanner(low: () => string[]) {
    const { LowCashBanner } = await load(bannerMods, './LowCashBanner.tsx');
    disposers.push(render(() => <LowCashBanner lowCash={{ low }} />, host));
  }
  const banner = () => host.querySelector<HTMLElement>('[data-testid="low-cash"]');

  it('L1 a banner with role status: the owner text, then the currencies joined by ", "', async () => {
    await mountBanner(() => ['THB', 'EUR']);
    const el = banner();
    expect(el).not.toBeNull();
    expect(el!.getAttribute('role')).toBe('status');
    const text = el!.textContent ?? '';
    const iTitle = text.indexOf(TITLE_TH);
    const iList = text.indexOf(CURRENCIES_TH('THB, EUR'));
    expect(iTitle, 'the owner text').toBeGreaterThanOrEqual(0);
    expect(iList, 'the currencies after it').toBeGreaterThan(iTitle);
  });

  it('L2 nothing low: no banner', async () => {
    await mountBanner(() => []);
    expect(banner()).toBeNull();
  });

  it('L3 the banner goes when low() empties, with nothing to press', async () => {
    const [low, setLow] = createSignal<string[]>(['USD']);
    await mountBanner(low);
    expect(banner()).not.toBeNull();
    expect(banner()!.querySelector('button'), 'nothing to press').toBeNull();
    setLow([]);
    expect(banner()).toBeNull();
  });

  it('EN words', async () => {
    await mountBanner(() => ['USD']);
    setLang('en');
    await vi.waitFor(() => expect(banner()?.textContent).toContain(TITLE_EN));
    expect(banner()!.textContent).toContain(CURRENCIES_EN('USD'));
  });
});

// ── DeskPage places the banner ────────────────────────────────────────────────────────────

describe('FE-L DeskPage', () => {
  const STEP = {
    './steps/rate/step.tsx': {
      step: { index: 0, label: () => 'Rate', component: () => <div data-testid="body-0">rate body</div> },
    },
  };
  let host: HTMLDivElement;
  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });
  afterEach(() => host.remove());

  function mount(low: () => string[], shift?: { open: () => boolean; Panel: () => JSX.Element }) {
    const steps = collectSteps(STEP);
    const reload = vi.fn(async () => {});
    const store = Object.assign(createFrameStore(steps.map((s) => s.index)), { lowCash: { low, reload } });
    disposers.push(render(() => <DeskPage store={store} steps={steps} shift={shift} />, host));
    return reload;
  }
  const banners = () => host.querySelectorAll<HTMLElement>('[data-testid="low-cash"]');

  it('open from the start: one read at mount, one banner above the steps', () => {
    const reload = mount(() => ['THB']);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(banners()).toHaveLength(1);
    const panel = host.querySelector('#panel')!;
    expect(
      banners()[0].compareDocumentPosition(panel) & Node.DOCUMENT_POSITION_FOLLOWING,
      'the banner comes before the step panel',
    ).toBeTruthy();
  });

  it('shift not open: no read, no banner; turning open reads once and shows it', () => {
    const [open, setOpen] = createSignal(false);
    const reload = mount(() => ['THB'], { open, Panel: () => <p data-testid="shift-panel">panel</p> });
    expect(reload).not.toHaveBeenCalled();
    expect(banners()).toHaveLength(0);
    setOpen(true);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(banners()).toHaveLength(1);
  });

  it('nothing low: no banner on the open desk', () => {
    mount(() => []);
    expect(banners()).toHaveLength(0);
  });
});

// ── the desk store and the save path ──────────────────────────────────────────────────────

const ME = () => ({ userId: 'u-1', username: 'ploy', name: 'Ploy', roles: [], branchId: 'MAIN', branchName: 'XC-B', branches: [] });
const PASSPORT = new File([new Uint8Array([1, 2, 3])], 'passport.jpg', { type: 'image/jpeg' });

function txAnswer(id: string, status: string, documentRef: string | null = null): Answer {
  return {
    status: 200,
    body: makeTransactionDto({
      id,
      branchId: 'MAIN',
      mode: 'LIVE',
      status,
      documentRef,
      quoteId: 'q-1',
      customerId: 'c-1',
      approvalRequestId: null,
      cancelReason: null,
      cancelledAt: null,
      recordedAt: '2026-11-03T03:00:00Z',
      lines: [makeTransactionLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '100', unitSize: '1', settleThb: '3210.00' })],
      payment: makeTransactionPaymentDto({ method: 'Cash', reference: '', account: '', amountThb: '3210.00', bankFeeThb: '0.00', paidAt: '2026-11-03T03:00:00Z' }),
    }),
  };
}

type Flow = { create?: Answer[]; complete?: Answer[] };

function serverFor(flow: Flow = {}) {
  const s = fakeServer();
  for (const v of ['desk_config', 'staff_desk_config']) s.on('GET', `/api/v3/views/${v}`, () => view({ shiftOpen: true }));
  s.on('GET', MY_SHIFT, () => view(shift(['THB'])));
  let quotes = 0;
  s.on('POST', '/api/quotes', (call) => {
    quotes += 1;
    const cashSettlement = (JSON.parse(call.body || '{}') as { paymentMethod?: string }).paymentMethod === 'Cash';
    return {
      status: 200,
      body: makeQuoteDto({
        id: `q-${quotes}`,
        type: 'BUY',
        channel: 'DESK',
        expiresAt: new Date(Date.now() + 300_000).toISOString(),
        publication: makeQuotePublicationDto({ id: 'pub-1', version: 3 }),
        lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '100', unitSize: '1', settleThb: '3210.00', publishedRateLineId: 'line-1' })],
        cashSettlement,
      }),
    };
  });
  for (const up of ['/api/upload', '/api/staff/upload']) {
    let n = 0;
    s.on('POST', up, () => ({ status: 200, body: { uploadKey: `key-${++n}` } }));
  }
  const seq = (list: Answer[] | undefined, dflt: () => Answer) => (_c: Call, n: number) => (list && list[n]) ?? dflt();
  s.on('POST', '/api/transactions', seq(flow.create, () => txAnswer('tx-1', 'DRAFT')));
  s.on('POST', '/api/transactions/tx-1/confirm', () => txAnswer('tx-1', 'CONFIRMED'));
  s.on('POST', '/api/transactions/tx-1/complete', seq(flow.complete, () => txAnswer('tx-1', 'COMPLETED', 'XC-20261103-000001')));
  return s;
}

async function ready(s: ReturnType<typeof fakeServer>) {
  const { createDeskStore } = await load(storeMods, './desk-store.ts');
  const paths = await load(pathMods, './desk-path.ts');
  let store!: Store;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path: paths.STAFF_DESK, me: ME });
  });
  await vi.waitFor(() => expect(store.config()).toBeDefined());
  store.setType('BUY');
  store.setCurrency('USD');
  store.setAmount('100');
  await store.requote();
  store.setPassport(PASSPORT);
  for (const [k, v] of Object.entries({ name: 'JOHN DOE', passportNo: 'AA1234567', nationality: 'USA', birthDate: '1990-01-01', passportExpiry: '2030-01-01' })) {
    store.setField(k, v, 'user');
  }
  store.setIdentityChecked(true);
  store.setCustomerId('CX-00000001');
  store.setValueDate('2026-11-03');
  store.setMethod('Cash');
  store.setAccount('Cash drawer 1');
  store.setBankFee('0');
  store.setNotes('');
  store.setPaymentChecked(true);
  return store;
}

/** `my_shift` reads that arrived after the given call. */
const readsAfter = (s: ReturnType<typeof fakeServer>, after: Call | undefined) =>
  s.callsTo('GET', MY_SHIFT).filter((c) => after !== undefined && c.at > after.at);

describe('FE-L desk store and save path', () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));

  it('the store has a lowCash on its own api, and does not read my_shift by itself', async () => {
    const s = serverFor();
    const store = await ready(s);
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    expect(s.callsTo('GET', MY_SHIFT), 'no read without a trigger').toHaveLength(0);
    expect(store.lowCash, 'store.lowCash').toBeDefined();
    await store.lowCash.reload();
    expect(s.callsTo('GET', MY_SHIFT)).toHaveLength(1);
    expect(store.lowCash.low()).toEqual(['THB']);
  });

  it('a save that ends COMPLETED reads my_shift after it', async () => {
    const s = serverFor();
    const store = await ready(s);
    expect(await store.submit()).toBe('saved');
    const complete = s.callsTo('POST', '/api/transactions/tx-1/complete').at(-1);
    await vi.waitFor(() => expect(readsAfter(s, complete).length).toBeGreaterThanOrEqual(1));
  });

  it('S1 complete answers 409 stockInsufficient: my_shift is read after it', async () => {
    const s = serverFor({ complete: [refusal(409, 'stockInsufficient', 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 1711.46')] });
    const store = await ready(s);
    expect(await store.submit()).toBe('saving');
    const complete = s.callsTo('POST', '/api/transactions/tx-1/complete')[0];
    await vi.waitFor(() => expect(readsAfter(s, complete).length).toBeGreaterThanOrEqual(1));
  });

  it('S6 create answers 409 stockInsufficient: my_shift is read after it', async () => {
    const s = serverFor({ create: [refusal(409, 'stockInsufficient', 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 1711.46')] });
    const store = await ready(s);
    expect(await store.submit()).toBe('refused');
    const create = s.callsTo('POST', '/api/transactions')[0];
    await vi.waitFor(() => expect(readsAfter(s, create).length).toBeGreaterThanOrEqual(1));
  });
});
