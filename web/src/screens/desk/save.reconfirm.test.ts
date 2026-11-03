// CR-f: the teller confirms the figure that is paid (design docs/design/blueprint/quote-transaction.md §13.9, rows
// CR13-CR18; CR12 restated). Written before the implementation by the ticket author (claude/b091f5e0, on the xchange
// lane controller claude/608dff's order). Do not edit, delete or weaken these tests; add your own in another file.
//
// [required] by these tests:
// - `save.ts` step 2: when the held quote is unusable (expired, or `cashSettlement !== (method === 'Cash')`), the save
//   asks for a new quote as today. If every line's `settleThb` and `finalRate` equal the held quote's, the create names
//   the new quote. Otherwise there is no create: `outcome()` is `'reconfirm'`, `paymentChecked()` is false, `step()` is
//   3, `quote()` is the new quote, and `reconfirm()` is `{ fromNet, toNet, fromRate, toRate }` (the first line's
//   strings). The next save after the teller ticks again creates on that quote with no extra quote.
// - `desk-store.ts` `setMethod`: a move between cash and non-cash un-ticks the confirm box; a move between two non-cash
//   methods leaves it.
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
type Figures = { settleThb: string; finalRate: string; roundingThb?: string };
type QuoteAsk = { n: number; paymentMethod?: string };

const ME = () => ({ id: 'u-1', userId: 'u-1', name: 'Ploy', roles: [], branch: 'MAIN', branchId: 'MAIN', branches: [] });
const PASSPORT = new File([new Uint8Array([1, 2, 3])], 'passport.jpg', { type: 'image/jpeg' });

const PREVIEW: Figures = { settleThb: '671.45', finalRate: '33.57260000' };
const CASH: Figures = { settleThb: '671.50', finalRate: '33.57260000', roundingThb: '0.05' };

function txDto(status: string) {
  return makeTransactionDto({
    id: 'tx-1',
    branchId: 'MAIN',
    mode: 'LIVE',
    status,
    documentRef: status === 'COMPLETED' ? 'XC-20261103-000006' : null,
    customerId: 'CX-00000001',
    lines: [makeTransactionLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '20', settleThb: '671.50' })],
    payment: makeTransactionPaymentDto({ method: 'Cash', amountThb: '671.50', bankFeeThb: '0.00' }),
  });
}

/**
 * The desk's server. Each quote is cash-settled when asked with `paymentMethod: "Cash"` (quote.rs, CR-b);
 * `figures(ask)` and `expired(ask)` decide its line figures and whether it is already expired.
 */
function fakeServer(figures: (ask: QuoteAsk) => Figures, expired: (ask: QuoteAsk) => boolean = () => false) {
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
      const ask: QuoteAsk = { n: quotes, paymentMethod: sent.paymentMethod };
      const f = figures(ask);
      return json(
        200,
        makeQuoteDto({
          id: `q-${quotes}`,
          type: 'BUY',
          channel: 'DESK',
          expiresAt: new Date(Date.now() + (expired(ask) ? -1_000 : 300_000)).toISOString(),
          cashSettlement: sent.paymentMethod === 'Cash',
          lines: [
            makeQuoteLineDto({
              currency: 'USD',
              shopSide: 'BUY',
              quantity: '20',
              finalRate: f.finalRate,
              settleThb: f.settleThb,
              roundingThb: f.roundingThb ?? '0.00',
            }),
          ],
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

/** `DeskStore.reconfirm()` (§13.9.2), read without depending on the type before it exists. */
function reconfirmOf(store: unknown): unknown {
  const read = (store as { reconfirm?: () => unknown }).reconfirm;
  return typeof read === 'function' ? read() : 'DeskStore.reconfirm() is missing';
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
  describe(`CR-f the teller confirms the figure that is paid, on ${label}`, () => {
    /** Step 0 quoted with no method (`q-1`), steps 1-3 filled, the box ticked, then the payment step's submit. */
    async function ready(server: ReturnType<typeof fakeServer>) {
      let store!: ReturnType<typeof createDeskStore>;
      createRoot((d) => {
        disposers.push(d);
        store = createDeskStore({ api: server.api, path, me: ME as never });
      });
      await flush();
      store.setType('BUY');
      store.setCurrency('USD');
      store.setAmount('20');
      await store.requote();
      store.setPassport(PASSPORT);
      store.setField('name', 'JOHN DOE', 'user');
      store.setField('passportNo', 'AA1234567', 'user');
      store.setField('nationality', 'USA', 'user');
      store.setField('birthDate', '1990-01-01', 'user');
      store.setField('passportExpiry', '2030-01-01', 'user');
      store.setIdentityChecked(true);
      store.setCustomerId('CX-00000001');
      store.setValueDate('2026-11-03');
      store.setAccount('Cash drawer 1');
      store.setBankFee('0');
      return store;
    }

    /** What the payment step's submit does before `store.submit()` (steps/payment/step.tsx onSubmit). */
    function pressSave(store: ReturnType<typeof createDeskStore>) {
      store.go(4);
      return store.submit();
    }

    it('CR13 non-cash 671.45, then Cash and an immediate save; the cash quote is 671.50: no create, the teller confirms again', async () => {
      const server = fakeServer((ask) => (ask.paymentMethod === 'Cash' ? CASH : PREVIEW));
      const store = await ready(server);
      expect(store.quote()?.lines[0]?.settleThb, 'the step-0 quote').toBe('671.45');
      store.setMethod('Cash');
      store.setPaymentChecked(true);

      const out = await pressSave(store);
      await flush();

      expect(server.creates(), 'no transaction is created on a net the teller did not confirm').toHaveLength(0);
      expect(out, 'the save answers reconfirm').toBe('reconfirm');
      expect(store.outcome()).toBe('reconfirm');
      expect(store.paymentChecked(), 'the confirm box is un-ticked').toBe(false);
      expect(store.step(), 'back on the payment step').toBe(3);
      expect(store.quote()?.id, 'the new quote is the one on screen').toBe('q-2');
      expect(store.quote()?.lines[0]?.settleThb).toBe('671.50');
      expect(reconfirmOf(store)).toEqual({
        fromNet: '671.45',
        toNet: '671.50',
        fromRate: '33.57260000',
        toRate: '33.57260000',
      });
    });

    it('CR14 after CR13, the teller ticks again and saves: one create on the CR13 quote, no extra quote', async () => {
      const server = fakeServer((ask) => (ask.paymentMethod === 'Cash' ? CASH : PREVIEW));
      const store = await ready(server);
      store.setMethod('Cash');
      store.setPaymentChecked(true);
      await pressSave(store);
      await flush();
      expect(server.creates(), 'CR13 created nothing').toHaveLength(0);
      const quotesBefore = server.quotes().length;

      store.setPaymentChecked(true);
      const out = await pressSave(store);
      await flush();

      expect(server.quotes().length, 'no extra quote at the second save').toBe(quotesBefore);
      const creates = server.creates();
      expect(creates, 'one create').toHaveLength(1);
      expect((JSON.parse(creates[0]!.body) as { quoteId?: string }).quoteId, 'names the confirmed quote').toBe('q-2');
      expect(out).toBe('saved');
      expect(reconfirmOf(store), 'cleared once the next save starts').toBeUndefined();
    });

    it('CR15 non-cash 3210.00, then Cash and an immediate save; the cash quote is also 3210.00: created on the new quote', async () => {
      const same: Figures = { settleThb: '3210.00', finalRate: '32.10000000' };
      const server = fakeServer(() => same);
      const store = await ready(server);
      store.setMethod('Cash');
      store.setPaymentChecked(true);

      const out = await pressSave(store);
      await flush();

      const creates = server.creates();
      expect(creates, 'one create').toHaveLength(1);
      expect((JSON.parse(creates[0]!.body) as { quoteId?: string }).quoteId).toBe('q-2');
      expect(out).toBe('saved');
      expect(reconfirmOf(store)).toBeUndefined();
    });

    it('CR16 the held quote has expired and the new one has another rate and net: no create, both rates are named', async () => {
      const server = fakeServer(
        (ask) => (ask.n === 1 ? PREVIEW : { settleThb: '672.00', finalRate: '33.60000000' }),
        (ask) => ask.n === 1,
      );
      const store = await ready(server);
      // a non-cash method matches the step-0 quote's settlement kind, so only its expiry makes it unusable
      store.setMethod('PromptPay');
      store.setPaymentChecked(true);

      const out = await pressSave(store);
      await flush();

      expect(server.creates(), 'no create on a rate the teller did not see').toHaveLength(0);
      expect(out).toBe('reconfirm');
      expect(store.paymentChecked()).toBe(false);
      expect(store.step()).toBe(3);
      expect(store.quote()?.id).toBe('q-2');
      expect(reconfirmOf(store)).toEqual({
        fromNet: '671.45',
        toNet: '672.00',
        fromRate: '33.57260000',
        toRate: '33.60000000',
      });
    });

    it('CR17 the held quote has expired and the new one has the same figures: created on the new quote', async () => {
      const server = fakeServer(
        () => PREVIEW,
        (ask) => ask.n === 1,
      );
      const store = await ready(server);
      store.setMethod('PromptPay');
      store.setPaymentChecked(true);

      const out = await pressSave(store);
      await flush();

      const creates = server.creates();
      expect(creates, 'one create').toHaveLength(1);
      expect((JSON.parse(creates[0]!.body) as { quoteId?: string }).quoteId).toBe('q-2');
      expect(out).toBe('saved');
    });

    it('CR18 a move between cash and non-cash un-ticks the confirm box; a move between two non-cash methods does not', async () => {
      const server = fakeServer(() => PREVIEW);
      const store = await ready(server);
      store.setMethod('PromptPay');
      store.setPaymentChecked(true);

      store.setMethod('Alipay');
      expect(store.paymentChecked(), 'non-cash to non-cash').toBe(true);

      store.setMethod('Cash');
      expect(store.paymentChecked(), 'non-cash to cash').toBe(false);

      store.setPaymentChecked(true);
      store.setMethod('PromptPay');
      expect(store.paymentChecked(), 'cash to non-cash').toBe(false);
    });
  });
}
