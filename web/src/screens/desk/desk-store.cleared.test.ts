import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { makeQuoteDto, makeQuoteLineDto, makeQuotePublicationDto, type QuoteDto } from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { OWNER_DESK, STAFF_DESK } from './desk-path';
import { createDeskStore, type DeskStore } from './desk-store';
import { step as rateStep } from './steps/rate/step';

const ME = () => ({
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});

const CONFIG = {
  shiftOpen: true,
  rates: [
    { code: 'USD', name: 'US Dollar', unitSize: '1', buyRate: '32.10', sellRate: '32.50' },
  ],
};

type Call = {
  method: string;
  path: string;
  key: string | null;
  body: string;
};

type Answer = { status: number; body?: unknown };
type Route = (call: Call, nth: number) => Answer | Promise<Answer>;

function fakeServer() {
  const calls: Call[] = [];
  const routes = new Map<string, Route>();
  const counts = new Map<string, number>();

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const headers = new Headers(init?.headers);
    const body =
      init?.body === undefined || init?.body === null
        ? ''
        : typeof init.body === 'string'
          ? init.body
          : '[binary]';
    const call: Call = { method, path, key: headers.get('idempotency-key'), body };
    calls.push(call);
    const k = `${method} ${path}`;
    const n = counts.get(k) ?? 0;
    counts.set(k, n + 1);
    const route = routes.get(k);
    const a: Answer = route
      ? await route(call, n)
      : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    return new Response(JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': 'application/json' },
    });
  });

  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    calls,
    on(method: string, path: string, route: Route) {
      routes.set(`${method.toUpperCase()} ${path}`, route);
    },
    callsTo(method: string, path: string): Call[] {
      return calls.filter((c) => c.method === method.toUpperCase() && c.path === path);
    },
  };
}

function quoteDto(over: Partial<QuoteDto> & { quantity?: string; currency?: string } = {}) {
  const q = over.quantity ?? '100';
  const curr = over.currency ?? 'USD';
  return makeQuoteDto({
    id: over.id ?? `q-${q}`,
    type: over.type ?? 'BUY',
    channel: 'DESK',
    expiresAt: over.expiresAt ?? new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 3 }),
    lines: over.lines ?? [
      makeQuoteLineDto({
        currency: curr,
        shopSide: 'BUY',
        quantity: q,
        unitSize: '1',
        baseRate: '32.10',
        finalRate: '32.10',
        thbAmount: '3210.00',
        feeThb: '0.00',
        settleThb: '3210.00',
        publishedRateLineId: 'line-1',
      }),
    ],
    benefits: [],
    stale: false,
  });
}

function serverWithQuotes(answerFor?: Route) {
  const s = fakeServer();
  for (const v of ['desk_config', 'staff_desk_config']) {
    s.on('GET', `/api/v3/views/${v}`, () => ({
      status: 200,
      body: { version: 1, changed: true, data: CONFIG },
    }));
  }
  s.on(
    'POST',
    '/api/quotes',
    answerFor ??
      ((c, n) => {
        const line = JSON.parse(c.body).lines[0];
        return {
          status: 200,
          body: quoteDto({
            id: `q-${n}-${line.quantity}`,
            quantity: line.quantity,
            currency: line.currency,
          }),
        };
      }),
  );
  return s;
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

for (const [which, path] of [
  ['OWNER_DESK', OWNER_DESK],
  ['STAFF_DESK', STAFF_DESK],
] as const) {
  describe(`DeskStore cleared amount/currency on ${which} (#2196)`, () => {
    let disposers: Array<() => void> = [];
    let host: HTMLDivElement;

    beforeEach(() => {
      vi.useFakeTimers();
      setLang('th');
      host = document.createElement('div');
      document.body.appendChild(host);
    });

    afterEach(() => {
      for (const d of disposers) d();
      disposers = [];
      host.remove();
      document.body.innerHTML = '';
      vi.useRealTimers();
      setLang('th');
    });

    function makeStore(s: ReturnType<typeof fakeServer>): DeskStore {
      let store!: DeskStore;
      createRoot((dispose) => {
        disposers.push(dispose);
        store = createDeskStore({ api: s.api, path, me: ME as unknown as () => never });
      });
      return store;
    }

    // Row 1: Three edits inside 300 ms | One quote for the last amount (kept, S2, desk-store.test.ts:180)
    it('Row 1: three edits inside 300 ms ask for one quote with the last amount, and clearing inside 300 ms resets prior quote without quoting', async () => {
      const s = serverWithQuotes();
      const store = makeStore(s);
      await flush();

      store.setCurrency('USD');
      store.setType('BUY');
      store.setAmount('1');
      await vi.advanceTimersByTimeAsync(100);
      store.setAmount('10');
      await vi.advanceTimersByTimeAsync(100);
      store.setAmount('100');
      await vi.advanceTimersByTimeAsync(299);
      expect(s.callsTo('POST', '/api/quotes')).toHaveLength(0);
      await vi.advanceTimersByTimeAsync(2);
      await flush();
      expect(s.callsTo('POST', '/api/quotes')).toHaveLength(1);
      expect(JSON.parse(s.callsTo('POST', '/api/quotes')[0]!.body).lines[0].quantity).toBe('100');
      expect(store.quote()?.lines[0]?.quantity).toBe('100');

      // Three rapid edits ending in '' inside 300 ms clear the quote and make no new quote call
      store.setAmount('200');
      await vi.advanceTimersByTimeAsync(100);
      store.setAmount('300');
      await vi.advanceTimersByTimeAsync(100);
      store.setAmount('');
      await vi.advanceTimersByTimeAsync(305);
      await flush();
      expect(s.callsTo('POST', '/api/quotes')).toHaveLength(1);
      expect(store.quote()).toBeUndefined();
    });

    // Row 2: An older answer after a newer one | Dropped (kept, S6, desk-store.test.ts:245)
    it('Row 2: drops an older quote answer that arrives after a newer quote or after clearing', async () => {
      let releaseFirst!: () => void;
      const firstHeld = new Promise<void>((r) => {
        releaseFirst = r;
      });
      let releaseThird!: () => void;
      const thirdHeld = new Promise<void>((r) => {
        releaseThird = r;
      });

      const s = serverWithQuotes(async (c, n) => {
        if (n === 0) await firstHeld;
        if (n === 2) await thirdHeld;
        const line = JSON.parse(c.body).lines[0];
        return {
          status: 200,
          body: quoteDto({ id: `q-${n}`, quantity: line.quantity, currency: line.currency }),
        };
      });

      const store = makeStore(s);
      await flush();
      store.setCurrency('USD');
      store.setAmount('1');
      const first = store.requote();
      store.setAmount('2');
      await store.requote();
      expect(store.quote()?.id).toBe('q-1');
      releaseFirst();
      await first;
      await flush();
      expect(store.quote()?.id).toBe('q-1');

      // Now start a 3rd quote request and clear before it answers
      store.setAmount('3');
      const third = store.requote();
      store.setAmount('');
      await store.requote();
      expect(store.quote()).toBeUndefined();
      releaseThird();
      await third;
      await flush();
      expect(store.quote()).toBeUndefined();
    });

    // Row 3: Quote for 500, then the box cleared | `quote()` is `undefined`; Next is refused
    it('Row 3: clears quote() to undefined and refuses Next immediately when amount input is cleared after quoting 500', async () => {
      const s = serverWithQuotes();
      const store = makeStore(s);
      await flush();

      const disposeStep = render(() => rateStep.component({ store }), host);
      disposers.push(disposeStep);

      const amountInput = host.querySelector<HTMLInputElement>('input[name="amount"]')!;
      const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
      expect(amountInput).not.toBeNull();
      expect(submitBtn).not.toBeNull();

      amountInput.value = '500';
      amountInput.dispatchEvent(new Event('input', { bubbles: true }));
      await vi.advanceTimersByTimeAsync(300);
      await flush();
      expect(store.quote()?.lines[0]?.quantity).toBe('500');

      // Clear the amount through the rendered input[name="amount"] and press Next WITHOUT advancing timers
      amountInput.value = '';
      amountInput.dispatchEvent(new Event('input', { bubbles: true }));
      submitBtn.click();

      expect(store.step()).toBe(0);
      expect(store.quote()).toBeUndefined();
      expect(store.quoteError()).toBeUndefined();
      expect(store.quoting()).toBe(false);
    });

    // Row 4: "5" requested, box cleared before the answer | No quote after the answer lands
    it('Row 4: drops an in-flight quote for "5" when the amount box is cleared before the answer lands', async () => {
      let releaseFive!: () => void;
      const fiveHeld = new Promise<void>((r) => {
        releaseFive = r;
      });

      const s = serverWithQuotes(async (c, n) => {
        await fiveHeld;
        const line = JSON.parse(c.body).lines[0];
        return {
          status: 200,
          body: quoteDto({ id: `q-${n}`, quantity: line.quantity, currency: line.currency }),
        };
      });

      const store = makeStore(s);
      await flush();
      store.setCurrency('USD');
      store.setAmount('5');
      const inFlight = store.requote();
      expect(store.quoting()).toBe(true);

      // Clear the amount box before the answer for "5" arrives
      store.setAmount('');
      await vi.advanceTimersByTimeAsync(300);
      const quotingWhileHeld = store.quoting();

      releaseFive();
      await inFlight;
      await flush();

      expect(store.quote()).toBeUndefined();
      expect(quotingWhileHeld).toBe(false);
      expect(store.quoting()).toBe(false);
    });

    // Row 5: Box cleared, then 300 typed | A new quote for 300
    it('Row 5: clears the previous quote when the box is cleared, then obtains a new quote when 300 is typed', async () => {
      const s = serverWithQuotes();
      const store = makeStore(s);
      await flush();

      store.setCurrency('USD');
      store.setAmount('500');
      await vi.advanceTimersByTimeAsync(300);
      await flush();
      expect(store.quote()?.lines[0]?.quantity).toBe('500');

      store.setAmount('');
      await vi.advanceTimersByTimeAsync(300);
      await flush();
      expect(store.quote()).toBeUndefined();

      store.setAmount('300');
      await vi.advanceTimersByTimeAsync(300);
      await flush();
      expect(store.quote()?.lines[0]?.quantity).toBe('300');
    });

    // Edge case: The currency is cleared instead of the amount: the same reset applies
    it('Edge case: resets quote, quoteError, quoting, and drops in-flight answers when currency is cleared', async () => {
      let holdSecond = false;
      let refuseThird = false;
      let releaseSecond!: () => void;
      const secondHeld = new Promise<void>((r) => {
        releaseSecond = r;
      });

      const s = serverWithQuotes(async (c, n) => {
        if (refuseThird) {
          return {
            status: 400,
            body: { error: { code: 'validationFailed', field: 'quantity', message: 'bad amount' } },
          };
        }
        if (holdSecond) await secondHeld;
        const line = JSON.parse(c.body).lines[0];
        return {
          status: 200,
          body: quoteDto({ id: `q-${n}`, quantity: line.quantity, currency: line.currency }),
        };
      });

      const store = makeStore(s);
      await flush();

      store.setCurrency('USD');
      store.setAmount('500');
      await store.requote();
      expect(store.quote()?.lines[0]?.quantity).toBe('500');

      // Start an in-flight quote, then clear currency before the answer lands
      holdSecond = true;
      store.setAmount('600');
      const inFlight = store.requote();
      expect(store.quoting()).toBe(true);

      store.setCurrency('');
      await vi.advanceTimersByTimeAsync(300);
      expect(store.quote()).toBeUndefined();
      expect(store.quoting()).toBe(false);

      releaseSecond();
      await inFlight;
      await flush();

      expect(store.quote()).toBeUndefined();

      // Also verify quoteError is cleared when currency is cleared
      holdSecond = false;
      refuseThird = true;
      store.setCurrency('USD');
      store.setAmount('abc');
      await store.requote();
      expect(store.quoteError()).toBeDefined();

      store.setCurrency('');
      await vi.advanceTimersByTimeAsync(300);
      expect(store.quoteError()).toBeUndefined();
    });
  });
}
