// #5229 CR-d: the desk sends `paymentMethod` on `POST /api/quotes` and re-quotes when the method's cash-ness changes
// (design docs/design/blueprint/quote-transaction.md §13.8 CR-d, :1080 "a method change re-quotes"; the server stores
// `cash_settlement = payment_method.is_some_and(is_cash)`, api/src/application/workflows/exchange/quote.rs:479, and
// `is_cash` is `Cash` only, api/src/domain/payment_method.rs:25).
// Written before the implementation by the FE lane's test-writer (claude/8be3a451's order). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// [required] by these tests (behaviour table rows R1-R4 of #5229):
// - `requote()` adds `paymentMethod: method()` to the quote body when the method is not '', and leaves the key out when
//   it is '' (step 0's preview, as today, CR7).
// - `setMethod(m)` sets the signal and, only when the cash-ness changes ('' / non-cash <-> 'Cash'), re-quotes through
//   the same 300 ms debounce as `setType` / `setFee` (`QUOTE_DEBOUNCE_MS`). A change between two non-cash methods does
//   not re-quote.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import { makeQuoteDto, makeQuoteLineDto } from '../../shared/testing/fixtures';
import { OWNER_DESK, STAFF_DESK } from './desk-path';
import { createDeskStore } from './desk-store';

type Call = { path: string; body: string };

/** Answers the config view and `POST /api/quotes`; the quote's `cashSettlement` follows the body, as the server's does. */
function fakeServer() {
  const calls: Call[] = [];
  let n = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? init.body : '';
    calls.push({ path, body });
    let status = 200;
    let answer: unknown;
    if (path.startsWith('/api/v3/views/')) {
      answer = { version: 1, changed: true, data: { shiftOpen: true, rates: [] } };
    } else if (path === '/api/quotes') {
      n += 1;
      const sent = JSON.parse(body) as { paymentMethod?: string };
      answer = makeQuoteDto({
        id: `q-${n}`,
        type: 'BUY',
        channel: 'DESK',
        expiresAt: new Date(Date.now() + 300_000).toISOString(),
        cashSettlement: sent.paymentMethod === 'Cash',
        lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '100', settleThb: '3210.00' })],
      });
    } else {
      status = 404;
      answer = { error: { code: 'notFound', message: 'nf' } };
    }
    return new Response(JSON.stringify(answer), { status, headers: { 'content-type': 'application/json' } });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    quotes: () => calls.filter((c) => c.path === '/api/quotes').map((c) => JSON.parse(c.body) as Record<string, unknown>),
  };
}

const ME = () => ({ id: 'u-1', userId: 'u-1', name: 'Ploy', roles: [], branch: 'MAIN', branchId: 'MAIN', branches: [] });
let disposers: (() => void)[] = [];

function makeStore(api: ApiClient, path: typeof OWNER_DESK) {
  let store!: ReturnType<typeof createDeskStore>;
  createRoot((d) => {
    disposers.push(d);
    store = createDeskStore({ api, path, me: ME as never });
  });
  return store;
}

/** Past the quote debounce (`QUOTE_DEBOUNCE_MS` = 300 in desk-store.ts), then let the answer land. */
async function settle() {
  await vi.advanceTimersByTimeAsync(400);
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
  vi.restoreAllMocks();
});

for (const [label, path] of [
  ['OWNER_DESK', OWNER_DESK],
  ['STAFF_DESK', STAFF_DESK],
] as const) {
  describe(`desk quote and the payment method on ${label}`, () => {
    async function ready(server: ReturnType<typeof fakeServer>) {
      const store = makeStore(server.api, path);
      store.setCurrency('USD');
      store.setAmount('100');
      await settle();
      return store;
    }

    it('R1 step 0, currency and amount with no method: the quote body has no paymentMethod key', async () => {
      const server = fakeServer();
      const store = await ready(server);
      expect(store.method()).toBe('');
      const quotes = server.quotes();
      expect(quotes.length, 'step 0 asked for a quote').toBeGreaterThanOrEqual(1);
      for (const q of quotes) expect('paymentMethod' in q, 'no paymentMethod key without a method').toBe(false);
    });

    it('R2 then setMethod("Cash"): one new POST /api/quotes whose body has paymentMethod "Cash"', async () => {
      const server = fakeServer();
      const store = await ready(server);
      const before = server.quotes().length;
      store.setMethod('Cash');
      await settle();
      const after = server.quotes();
      expect(after.length, 'one new quote after the method became Cash').toBe(before + 1);
      expect(after.at(-1)!.paymentMethod).toBe('Cash');
      expect(store.quote()?.cashSettlement, 'the store holds the cash quote').toBe(true);
    });

    it('R3 Cash -> PromptPay: a new quote with paymentMethod "PromptPay"', async () => {
      const server = fakeServer();
      const store = await ready(server);
      store.setMethod('Cash');
      await settle();
      const before = server.quotes().length;
      store.setMethod('PromptPay');
      await settle();
      const after = server.quotes();
      expect(after.length, 'one new quote after Cash -> PromptPay').toBe(before + 1);
      expect(after.at(-1)!.paymentMethod).toBe('PromptPay');
      expect(store.quote()?.cashSettlement, 'the store holds the non-cash quote').toBe(false);
    });

    it('R4 PromptPay -> Credit Card (both non-cash): no new quote', async () => {
      const server = fakeServer();
      const store = await ready(server);
      store.setMethod('Cash');
      await settle();
      store.setMethod('PromptPay');
      await settle();
      // precondition: the PromptPay quote was asked for (R3), so the count below is not vacuous
      expect(server.quotes().at(-1)!.paymentMethod).toBe('PromptPay');
      const before = server.quotes().length;
      store.setMethod('Credit Card');
      await settle();
      expect(server.quotes().length, 'no new quote between two non-cash methods').toBe(before);
      expect(store.method()).toBe('Credit Card');
    });
  });
}
