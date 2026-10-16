// #2659: `desk-store.ts` `backToEdit()` re-quotes for the kept amount and currency after a refused save.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import type { QuoteDto } from '../../shared/api/generated/quote';

type Store = Record<string, any>;
type StoreModule = { createDeskStore: (deps: { api: ApiClient; path: unknown; me: () => unknown }) => Store };
type PathModule = { OWNER_DESK: Record<string, any>; STAFF_DESK: Record<string, any> };
const storeMods = import.meta.glob<StoreModule>('./desk-store.ts');
const pathMods = import.meta.glob<PathModule>('./desk-path.ts');

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const ME = () => ({
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});
const CONFIG = { shiftOpen: true };

let disposers: (() => void)[] = [];
async function makeStore(server: ReturnType<typeof fakeServer>, which: 'OWNER_DESK' | 'STAFF_DESK' = 'OWNER_DESK') {
  const { createDeskStore } = await load(storeMods, './desk-store.ts');
  const paths = await load(pathMods, './desk-path.ts');
  const path = paths[which];
  let store!: Store;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: server.api, path, me: ME });
  });
  return { store, path };
}

afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
});

type Call = {
  method: string;
  path: string;
  key: string | null;
  body: string;
  at: number;
};

type Answer = { status: number; body?: unknown; raw?: string; throws?: boolean };
type Route = (call: Call, nth: number) => Answer | Promise<Answer>;

function fakeServer() {
  const calls: Call[] = [];
  const routes = new Map<string, Route>();
  const counts = new Map<string, number>();
  let clock = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0];
    const headers = new Headers(init?.headers);
    const body =
      init?.body === undefined || init?.body === null
        ? ''
        : typeof init.body === 'string'
          ? init.body
          : '[binary]';
    const call: Call = { method, path, key: headers.get('idempotency-key'), body, at: clock++ };
    calls.push(call);
    const k = `${method} ${path}`;
    const n = counts.get(k) ?? 0;
    counts.set(k, n + 1);
    const route = routes.get(k);
    const a: Answer = route
      ? await route(call, n)
      : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    if (a.throws) throw new TypeError('Failed to fetch');
    const text = a.raw ?? JSON.stringify(a.body ?? {});
    return new Response(text, {
      status: a.status,
      headers: { 'content-type': a.raw ? 'text/html' : 'application/json' },
    });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    fetchFn,
    calls,
    on(method: string, path: string, route: Route) {
      routes.set(`${method.toUpperCase()} ${path}`, route);
    },
    callsTo(method: string, path: string): Call[] {
      return calls.filter((c) => c.method === method.toUpperCase() && c.path === path);
    },
  };
}

const view = (data: unknown): Answer => ({
  status: 200,
  body: { version: 1, changed: true, data },
});

function quoteDto(over: Partial<QuoteDto> & { quantity?: string } = {}): QuoteDto {
  const q = over.quantity ?? '100';
  return {
    id: over.id ?? 'q-1',
    type: over.type ?? 'BUY',
    channel: 'DESK',
    expiresAt: over.expiresAt ?? new Date(Date.now() + 300_000).toISOString(),
    publication: { id: 'pub-1', version: 3 },
    lines: over.lines ?? [
      {
        currency: 'USD',
        shopSide: 'BUY',
        quantity: q,
        unitSize: '1',
        baseRate: '32.10',
        finalRate: '32.10',
        thbAmount: '3210.00',
        feeThb: '0.00',
        settleThb: '3210.00',
        publishedRateLineId: 'line-1',
      },
    ],
    benefits: [],
    stale: false,
  };
}

const refusal = (status: number, code: string, message = 'refused', field?: string): Answer => ({
  status,
  body: { error: { code, message, ...(field ? { field } : {}) } },
});

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

function serverWithQuotes(answerFor?: (call: Call, n: number) => Answer | Promise<Answer>) {
  const s = fakeServer();
  for (const v of ['desk_config', 'staff_desk_config']) s.on('GET', `/api/v3/views/${v}`, () => view(CONFIG));
  s.on(
    'POST',
    '/api/quotes',
    answerFor ??
      ((c, n) => ({
        status: 200,
        body: quoteDto({ id: `q-${n + 1}`, quantity: JSON.parse(c.body).lines[0].quantity }),
      })),
  );
  return s;
}

describe('DeskStore backToEdit (#2659)', () => {
  beforeEach(() => vi.useFakeTimers());

  it('B1 requests exactly one new quote for the kept amount after a refused save and sets quote() once answered', async () => {
    const s = serverWithQuotes();
    const { store } = await makeStore(s);
    store.setCurrency('USD');
    store.setType('BUY');
    store.setAmount('100');
    await store.requote();
    expect(s.callsTo('POST', '/api/quotes')).toHaveLength(1);
    expect(store.quote()?.id).toBe('q-1');

    store.go(4);
    await store.submit();
    expect(store.outcome()).toBe('refused');

    store.backToEdit();
    await flush();

    const quoteCalls = s.callsTo('POST', '/api/quotes');
    expect(quoteCalls).toHaveLength(2);
    expect(JSON.parse(quoteCalls[1].body).lines[0]).toEqual(
      expect.objectContaining({ currency: 'USD', shopSide: 'BUY', quantity: '100' }),
    );
    expect(store.quote()?.id).toBe('q-2');
    expect(store.quote()?.lines[0].quantity).toBe('100');
  });

  it('B2 keeps the newer quote when amount is edited while the backToEdit quote is in flight', async () => {
    let releaseBackToEditQuote!: () => void;
    const backToEditQuoteHeld = new Promise<void>((r) => {
      releaseBackToEditQuote = r;
    });
    const s = serverWithQuotes(async (c, n) => {
      if (n === 1) await backToEditQuoteHeld;
      const quantity = JSON.parse(c.body).lines[0].quantity;
      return { status: 200, body: quoteDto({ id: `q-${n + 1}`, quantity }) };
    });
    const { store } = await makeStore(s);
    store.setCurrency('USD');
    store.setType('BUY');
    store.setAmount('100');
    await store.requote();
    expect(store.quote()?.id).toBe('q-1');

    store.go(4);
    await store.submit();
    expect(store.outcome()).toBe('refused');

    store.backToEdit();
    expect(s.callsTo('POST', '/api/quotes')).toHaveLength(2);

    store.setAmount('200');
    await vi.advanceTimersByTimeAsync(301);
    await flush();

    expect(s.callsTo('POST', '/api/quotes')).toHaveLength(3);
    expect(store.quote()?.lines[0].quantity).toBe('200');

    releaseBackToEditQuote();
    await flush();

    expect(store.quote()?.lines[0].quantity).toBe('200');
    expect(store.quote()?.id).toBe('q-3');
  });

  it('B3 sends no quote request and leaves quote() undefined when amount is empty', async () => {
    const s = serverWithQuotes();
    const { store } = await makeStore(s);
    store.setCurrency('USD');
    store.setAmount('');
    await flush();
    expect(s.callsTo('POST', '/api/quotes')).toHaveLength(0);

    store.go(4);
    store.backToEdit();
    await vi.advanceTimersByTimeAsync(500);
    await flush();

    expect(s.callsTo('POST', '/api/quotes')).toHaveLength(0);
    expect(store.quote()).toBeUndefined();
    expect(store.step()).toBe(0);
  });

  it('B4 sets quoteError() and leaves quote() undefined when the new quote is refused', async () => {
    const s = serverWithQuotes((c, n) => {
      if (n === 1) return refusal(409, 'rateUnavailable', 'เรทไม่พร้อมใช้งาน');
      return { status: 200, body: quoteDto({ id: `q-${n + 1}`, quantity: JSON.parse(c.body).lines[0].quantity }) };
    });
    const { store } = await makeStore(s);
    store.setCurrency('USD');
    store.setType('BUY');
    store.setAmount('100');
    await store.requote();
    expect(store.quote()?.id).toBe('q-1');

    store.go(4);
    await store.submit();
    expect(store.outcome()).toBe('refused');

    store.backToEdit();
    await flush();

    expect(store.quote()).toBeUndefined();
    expect(store.quoteError()).toEqual({ message: 'เรทไม่พร้อมใช้งาน' });
  });

  it('B5 clears staffDraftId and resets step to 0', async () => {
    const s = serverWithQuotes();
    const { store } = await makeStore(s);
    store.setStaffDraftId('draft-1');
    store.go(4);
    store.backToEdit();

    expect(store.staffDraftId()).toBe('');
    expect(store.step()).toBe(0);
  });
});
