// #5229 CR-d, CR12: the save never creates a transaction on a quote priced for the other settlement kind (design
// docs/design/blueprint/quote-transaction.md §13.8; the server refuses a `Cash` create on a quote with
// `cash_settlement = false`, api/src/application/workflows/exchange/create.rs:247-249, design CR8).
// Written before the implementation by the FE lane's test-writer (claude/8be3a451's order). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// [required] by these tests (behaviour table rows R5-R6 of #5229):
// - `save.ts` step 2: before the create, a quote whose `cashSettlement !== (store.method() === 'Cash')` is unusable,
//   and a new quote is asked for first (as for an expired quote). The create names that new quote's id. This covers a
//   save pressed inside the 300 ms re-quote debounce that follows `setMethod`.
// - A quote whose `cashSettlement` matches the method is used as today: no extra quote, the create names it.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import {
  makeQuoteDto,
  makeQuoteLineDto,
  makeTransactionDto,
  makeTransactionLineDto,
  makeTransactionPaymentDto,
} from '../../shared/testing/fixtures';
import { OWNER_DESK, STAFF_DESK } from './desk-path';
import { createDeskStore } from './desk-store';

type Call = { method: string; path: string; body: string; at: number };

const ME = () => ({ id: 'u-1', userId: 'u-1', name: 'Ploy', roles: [], branch: 'MAIN', branchId: 'MAIN', branches: [] });
const PASSPORT = new File([new Uint8Array([1, 2, 3])], 'passport.jpg', { type: 'image/jpeg' });

function txDto(status: string) {
  return makeTransactionDto({
    id: 'tx-1',
    branchId: 'MAIN',
    mode: 'LIVE',
    status,
    documentRef: status === 'COMPLETED' ? 'XC-20261027-000001' : null,
    customerId: 'CX-00000001',
    lines: [makeTransactionLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '100', settleThb: '3210.00' })],
    payment: makeTransactionPaymentDto({ method: 'Cash', amountThb: '3210.00', bankFeeThb: '0.00' }),
  });
}

/**
 * The desk's server. `cashSettlement(body)` decides each quote's settlement kind; by default it follows the body's
 * `paymentMethod`, as the server's quote.rs:479 does.
 */
function fakeServer(cashSettlement: (sent: { paymentMethod?: string }) => boolean = (sent) => sent.paymentMethod === 'Cash') {
  const calls: Call[] = [];
  let clock = 0;
  let quotes = 0;
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? init.body : init?.body ? '[binary]' : '';
    calls.push({ method, path, body, at: clock++ });
    if (method === 'GET' && path.startsWith('/api/v3/views/')) {
      return json(200, { version: 1, changed: true, data: { shiftOpen: true } });
    }
    if (method === 'POST' && path === '/api/quotes') {
      quotes += 1;
      const sent = JSON.parse(body) as { paymentMethod?: string };
      return json(
        200,
        makeQuoteDto({
          id: `q-${quotes}`,
          type: 'BUY',
          channel: 'DESK',
          expiresAt: new Date(Date.now() + 300_000).toISOString(),
          cashSettlement: cashSettlement(sent),
          lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '100', settleThb: '3210.00' })],
        }),
      );
    }
    if (method === 'POST' && (path === '/api/upload' || path === '/api/staff/upload')) {
      return json(200, { uploadKey: 'key-1' });
    }
    if (method === 'POST' && path === '/api/transactions') return json(200, txDto('DRAFT'));
    if (method === 'POST' && path === '/api/transactions/tx-1/confirm') return json(200, txDto('CONFIRMED'));
    if (method === 'POST' && path === '/api/transactions/tx-1/complete') return json(200, txDto('COMPLETED'));
    return json(404, { error: { code: 'notFound', message: 'nf' } });
  });
  const to = (m: string, p: string) => calls.filter((c) => c.method === m && c.path === p);
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    quotes: () => to('POST', '/api/quotes'),
    creates: () => to('POST', '/api/transactions'),
  };
}

let disposers: (() => void)[] = [];
beforeEach(() => {
  // plain fake timers: the 300 ms re-quote debounce never fires unless a test advances the clock
  vi.useFakeTimers();
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
  vi.restoreAllMocks();
});

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

for (const [label, path] of [
  ['OWNER_DESK', OWNER_DESK],
  ['STAFF_DESK', STAFF_DESK],
] as const) {
  describe(`the save and the quote's settlement kind on ${label}`, () => {
    /** Step 0 quoted with no method (the first quote, `q-1`), then steps 1-3 filled, all but the method. */
    async function ready(server: ReturnType<typeof fakeServer>) {
      let store!: ReturnType<typeof createDeskStore>;
      createRoot((d) => {
        disposers.push(d);
        store = createDeskStore({ api: server.api, path, me: ME as never });
      });
      await flush();
      store.setType('BUY');
      store.setCurrency('USD');
      store.setAmount('100');
      await store.requote();
      store.setPassport(PASSPORT);
      store.setField('name', 'JOHN DOE', 'user');
      store.setField('passportNo', 'AA1234567', 'user');
      store.setField('nationality', 'USA', 'user');
      store.setField('birthDate', '1990-01-01', 'user');
      store.setField('passportExpiry', '2030-01-01', 'user');
      store.setIdentityChecked(true);
      store.setCustomerId('CX-00000001');
      store.setValueDate('2026-10-27');
      store.setAccount('Cash drawer 1');
      store.setBankFee('0');
      store.setPaymentChecked(true);
      return store;
    }

    it('R5 a quote with cashSettlement false, then Cash and an immediate save: the save re-quotes with "Cash" and creates on the new quote', async () => {
      const server = fakeServer();
      const store = await ready(server);
      const first = store.quote();
      expect(first?.id, 'the step-0 quote').toBe('q-1');
      expect(first?.cashSettlement, 'priced with no method').toBe(false);

      store.setMethod('Cash');
      // the save is pressed inside the re-quote debounce: no clock advance before it
      await store.submit();

      const creates = server.creates();
      expect(creates, 'one create').toHaveLength(1);
      const create = creates[0]!;
      const cashQuotes = server.quotes().filter((c) => (JSON.parse(c.body) as { paymentMethod?: string }).paymentMethod === 'Cash');
      expect(cashQuotes.length, 'a quote with paymentMethod "Cash" was asked for').toBeGreaterThanOrEqual(1);
      expect(cashQuotes[0]!.at, 'before the create').toBeLessThan(create.at);
      const quoteId = (JSON.parse(create.body) as { quoteId?: string }).quoteId;
      expect(quoteId, 'never the first (non-cash) quote').not.toBe('q-1');
      expect(quoteId, "the create names the second quote's id").toBe('q-2');
    });

    for (const [method, settles] of [
      ['Cash', true],
      ['PromptPay', false],
    ] as const) {
      it(`R6 a quote whose cashSettlement matches the method (${method}): no extra quote, the create uses it`, async () => {
        // every quote is answered with this settlement kind, so the step-0 quote already matches the method
        const server = fakeServer(() => settles);
        const store = await ready(server);
        store.setMethod(method);
        // let any re-quote the method change starts finish before the save
        await vi.advanceTimersByTimeAsync(400);
        await flush();
        const held = store.quote();
        expect(held?.cashSettlement, 'the held quote matches the method').toBe(settles);
        const before = server.quotes().length;

        await store.submit();

        expect(server.quotes().length, 'no extra quote at save').toBe(before);
        const creates = server.creates();
        expect(creates).toHaveLength(1);
        expect((JSON.parse(creates[0]!.body) as { quoteId?: string }).quoteId).toBe(held?.id);
      });
    }
  });
}
