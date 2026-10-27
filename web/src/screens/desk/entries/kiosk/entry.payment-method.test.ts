// #5229 CR-d: a kiosk request paid in cash is quoted for cash (design docs/design/blueprint/quote-transaction.md §13.8
// CR-d). The entry already calls `store.setMethod(answer.paymentMethod)` after type, currency and amount
// (`entry.ts:36-39`); with the desk store's method on the quote, the quote it triggers carries the method.
// Written before the implementation by the FE lane's test-writer (claude/8be3a451's order). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// [required] by this test (behaviour table row R7 of #5229): after `entry.load` on a real desk store, with the kiosk
// answering `paymentMethod: "Cash"`, the `POST /api/quotes` it triggers has `paymentMethod: "Cash"`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../../../shared/api/client';
import { makeQuoteDto, makeQuoteLineDto } from '../../../../shared/testing/fixtures';
import { OWNER_DESK } from '../../desk-path';
import { createDeskStore } from '../../desk-store';
import { entry } from './entry';

const GET_ANSWER = {
  paymentMethod: 'Cash',
  type: 'BUY',
  currency: 'USD',
  rate: 32.1,
  foreign: 500,
  thb: 16050,
  inputAmount: 500,
  branchName: 'XC-B',
  id: 'k-42',
  queue: 'K042',
};

function fakeServer() {
  const calls: { path: string; body: string }[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? init.body : '';
    calls.push({ path, body });
    let status = 200;
    let answer: unknown;
    if (path.startsWith('/api/v3/views/')) {
      answer = { version: 1, changed: true, data: { shiftOpen: true, rates: [] } };
    } else if (path === '/api/kiosk' && (JSON.parse(body) as { action?: string }).action === 'get') {
      answer = GET_ANSWER;
    } else if (path === '/api/quotes') {
      const sent = JSON.parse(body) as { paymentMethod?: string };
      answer = makeQuoteDto({
        id: 'q-1',
        type: 'BUY',
        channel: 'DESK',
        expiresAt: new Date(Date.now() + 300_000).toISOString(),
        cashSettlement: sent.paymentMethod === 'Cash',
        lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '500', settleThb: '16050.00' })],
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

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('the kiosk entry and the payment method', () => {
  it('R7 a kiosk request with paymentMethod "Cash": the quote it triggers has paymentMethod "Cash"', async () => {
    const server = fakeServer();
    let store!: ReturnType<typeof createDeskStore>;
    createRoot((d) => {
      disposers.push(d);
      store = createDeskStore({ api: server.api, path: OWNER_DESK, me: ME as never });
    });
    await entry.load(store, 'k-42', server.api);
    expect(store.method()).toBe('Cash');
    // past the quote debounce (`QUOTE_DEBOUNCE_MS` = 300 in desk-store.ts)
    await vi.advanceTimersByTimeAsync(400);
    for (let i = 0; i < 6; i++) await Promise.resolve();
    const quotes = server.quotes();
    expect(quotes.length, 'the entry triggered a quote').toBeGreaterThanOrEqual(1);
    for (const q of quotes) expect(q.paymentMethod, 'every quote the entry triggers is priced for cash').toBe('Cash');
    expect(store.quote()?.cashSettlement).toBe(true);
  });
});
