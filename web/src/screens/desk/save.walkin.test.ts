// #5385 WQ-FE2: desk save sends the walk-in queue ticket (queueTicketId) in the create body
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import {
  makeQueueTicketDto,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  makeTransactionDto,
  makeTransactionLineDto,
  makeTransactionPaymentDto,
  type TransactionDto,
} from '../../shared/testing/fixtures';
import { OWNER_DESK } from './desk-path';
import { createDeskStore, type DeskEntryState } from './desk-store';

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

describe('desk/save.ts — walk-in queueTicketId (§8, #5385)', () => {
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

  it('row 1: walk-in, setWalkInTicket(makeQueueTicketDto({ id: "qt-1" })), then submit', async () => {
    const { api, creates } = makeHarness();
    let store!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store = createDeskStore({ api, path: OWNER_DESK, me: ME });
    });
    await readyStore(store);
    store.setWalkInTicket(makeQueueTicketDto({ id: 'qt-1' }));

    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.queueTicketId).toBe('qt-1');
  });

  it('row 2: a sale with no walk-in ticket (an entry sale, or no ticket set)', async () => {
    const { api: api1, creates: creates1 } = makeHarness();
    let store1!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store1 = createDeskStore({ api: api1, path: OWNER_DESK, me: ME });
    });
    await readyStore(store1);
    await expect(store1.submit()).resolves.toBe('saved');
    expect(creates1).toHaveLength(1);
    expect(creates1[0]!.body.queueTicketId).toBeNull();

    const { api: api2, creates: creates2 } = makeHarness();
    let store2!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store2 = createDeskStore({ api: api2, path: OWNER_DESK, me: ME });
    });
    store2.setEntry({
      kind: 'booking',
      payloadExtras: () => ({ bookingId: 'b-123' }),
      refusal: () => undefined,
    } as unknown as DeskEntryState);
    await readyStore(store2);
    await expect(store2.submit()).resolves.toBe('saved');
    expect(creates2).toHaveLength(1);
    expect(creates2[0]!.body.queueTicketId).toBeNull();
  });

  it('row 3: walk-in ticket set, startNew(), a new sale with no ticket, submit', async () => {
    const { api, creates } = makeHarness();
    let store!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store = createDeskStore({ api, path: OWNER_DESK, me: ME });
    });
    await readyStore(store);
    store.setWalkInTicket(makeQueueTicketDto({ id: 'qt-1' }));
    expect(store.walkInTicket()?.id).toBe('qt-1');

    store.startNew();
    expect(store.walkInTicket()).toBeUndefined();

    await readyStore(store);
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.queueTicketId).toBeNull();
  });

  it('row 4: walk-in ticket set, backToEdit(), submit', async () => {
    const { api, creates } = makeHarness();
    let store!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store = createDeskStore({ api, path: OWNER_DESK, me: ME });
    });
    await readyStore(store);
    store.setWalkInTicket(makeQueueTicketDto({ id: 'qt-1' }));

    store.backToEdit();
    expect(store.walkInTicket()?.id).toBe('qt-1');

    await store.requote();
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(1);
    expect(creates[0]!.body.queueTicketId).toBe('qt-1');
  });
});
