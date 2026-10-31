// #5799 FE-W4a up-front tests (docs/design/walkin-queue.md §12.4 row 1 save column, W4a.3, W4a.7): a held BOOKING ticket's booking
// goes in the create body as bookingId, never with queueTicketId. Written before the implementation by the ticket author
// claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own. Harness copied from save.walkin.test.ts.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import {
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  makeTransactionDto,
  makeTransactionLineDto,
  makeTransactionPaymentDto,
  type TransactionDto,
} from '../../shared/testing/fixtures';
import { OWNER_DESK } from './desk-path';
import { createDeskStore } from './desk-store';

interface CreateCall {
  method: string;
  path: string;
  body: Record<string, unknown>;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const ME = () => ({
  id: 'u-1',
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  permissions: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});

const PASSPORT = new File([new Uint8Array([1, 2, 3])], 'passport.jpg', { type: 'image/jpeg' });

function quoteDto(id = 'q-1') {
  return makeQuoteDto({
    id,
    type: 'BUY',
    channel: 'DESK',
    cashSettlement: true,
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 3 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
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

function txDto(id: string, status: string, extra: Partial<TransactionDto> = {}) {
  return makeTransactionDto({
    id,
    branchId: 'MAIN',
    mode: 'LIVE',
    status,
    documentRef: status === 'COMPLETED' ? 'XC-20261011-000001' : null,
    quoteId: 'q-1',
    customerId: 'CX-00000001',
    approvalRequestId: null,
    cancelReason: null,
    cancelledAt: null,
    recordedAt: '2026-10-11T03:00:00Z',
    lines: [
      makeTransactionLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
        unitSize: '1',
        rateSnapshot: '35.12000000',
        thbAmount: '3512.00',
        feeThb: '0.00',
        settleThb: '3512.00',
      }),
    ],
    payment: makeTransactionPaymentDto({
      method: 'Cash',
      reference: '',
      account: 'Cash drawer 1',
      amountThb: '3512.00',
      bankFeeThb: '0.00',
      paidAt: '2026-10-11T03:00:00Z',
    }),
    ...extra,
  });
}

function makeHarness() {
  const creates: CreateCall[] = [];
  let uploads = 0;
  let quotes = 0;

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;

    if (method === 'GET' && (path === '/api/v3/views/desk_config' || path === '/api/v3/views/staff_desk_config')) {
      return jsonResponse(200, { version: 1, changed: true, data: { shiftOpen: true } });
    }
    if (method === 'POST' && path === '/api/quotes') {
      quotes += 1;
      return jsonResponse(200, quoteDto(`q-${quotes}`));
    }
    if (method === 'POST' && (path === '/api/upload' || path === '/api/staff/upload')) {
      uploads += 1;
      return jsonResponse(200, { uploadKey: `key-${uploads}` });
    }
    if (method === 'POST' && path === '/api/transactions') {
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
      creates.push({ method, path, body });
      return jsonResponse(200, txDto('tx-1', 'DRAFT'));
    }
    if (method === 'POST' && path === '/api/transactions/tx-1/confirm') {
      return jsonResponse(200, txDto('tx-1', 'CONFIRMED'));
    }
    if (method === 'POST' && path === '/api/transactions/tx-1/complete') {
      return jsonResponse(200, txDto('tx-1', 'COMPLETED'));
    }
    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    creates,
  };
}

describe('desk/save.ts — held BOOKING ticket sends bookingId (W4a.3)', () => {
  let disposers: Array<() => void> = [];

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    disposers = [];
  });

  afterEach(() => {
    disposers.forEach((d) => d());
    disposers = [];
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  async function readyStore(store: ReturnType<typeof createDeskStore>) {
    await vi.waitFor(() => expect(store.config()).toBeDefined());
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
    store.setValueDate('2026-10-11');
    store.setMethod('Cash');
    store.setAccount('Cash drawer 1');
    store.setBankFee('0');
    store.setNotes('');
    store.setPaymentChecked(true);
    return store;
  }

  function newStore(api: ApiClient): ReturnType<typeof createDeskStore> {
    let store!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store = createDeskStore({ api, path: OWNER_DESK, me: ME });
    });
    return store;
  }

  it('row 1: heldBooking {bk-9} set -> create body bookingId "bk-9", queueTicketId null', async () => {
    const { api, creates } = makeHarness();
    const store = newStore(api);
    await readyStore(store);
    store.setHeldBooking({ bookingId: 'bk-9', ticketId: 't-b', displayNumber: 'B003' });

    await expect(store.submit()).resolves.toBe('saved');
    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.bookingId).toBe('bk-9');
    expect(creates[0]!.body.queueTicketId).toBeNull();
    expect(creates[0]!.body.kioskRequestId).toBeNull();
  });

  it('row 2: no heldBooking, no entry -> bookingId null, as today', async () => {
    const { api, creates } = makeHarness();
    const store = newStore(api);
    await readyStore(store);

    await expect(store.submit()).resolves.toBe('saved');
    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.bookingId).toBeNull();
  });

  it('row 3: heldBooking set, startNew(), a new sale -> bookingId null (W4a.7)', async () => {
    const { api, creates } = makeHarness();
    const store = newStore(api);
    await readyStore(store);
    store.setHeldBooking({ bookingId: 'bk-9', ticketId: 't-b', displayNumber: 'B003' });
    expect(store.heldBooking()?.bookingId).toBe('bk-9');

    store.startNew();
    expect(store.heldBooking()).toBeUndefined();

    await readyStore(store);
    await expect(store.submit()).resolves.toBe('saved');
    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.bookingId).toBeNull();
  });

  it('row 4: heldBooking set, backToEdit(), submit -> still bookingId "bk-9" (same sale)', async () => {
    const { api, creates } = makeHarness();
    const store = newStore(api);
    await readyStore(store);
    store.setHeldBooking({ bookingId: 'bk-9', ticketId: 't-b', displayNumber: 'B003' });

    store.backToEdit();
    expect(store.heldBooking()?.bookingId).toBe('bk-9');
    await store.requote();
    await expect(store.submit()).resolves.toBe('saved');
    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.bookingId).toBe('bk-9');
  });
});
