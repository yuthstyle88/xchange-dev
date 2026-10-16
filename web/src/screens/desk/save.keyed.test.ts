// #2023 KW-4 Phase 1: desk/save.ts idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import type { QuoteDto } from '../../shared/api/generated/quote';
import type { TransactionDto } from '../../shared/api/generated/transactions';
import { OWNER_DESK } from './desk-path';
import { createDeskStore } from './desk-store';

interface WriteCall {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepAnswer =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown };

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

function quoteDto(id = 'q-1'): QuoteDto {
  return {
    id,
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: { id: 'pub-1', version: 3 },
    lines: [
      {
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
      },
    ],
    benefits: [],
    stale: false,
  };
}

function txDto(id: string, status: string, extra: Partial<TransactionDto> = {}): TransactionDto {
  return {
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
      {
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
        unitSize: '1',
        rateSnapshot: '35.12000000',
        thbAmount: '3512.00',
        feeThb: '0.00',
        settleThb: '3512.00',
      },
    ],
    payment: {
      method: 'Cash',
      reference: '',
      account: 'Cash drawer 1',
      amountThb: '3512.00',
      bankFeeThb: '0.00',
      paidAt: '2026-10-11T03:00:00Z',
    },
    ...extra,
  };
}

function makeHarness(createSteps: StepAnswer[]) {
  const creates: WriteCall[] = [];
  let stepIdx = 0;
  let uploads = 0;

  const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;

    if (method === 'GET' && path === '/api/v3/views/desk_config') {
      return jsonResponse(200, { version: 1, changed: true, data: { shiftOpen: true } });
    }
    if (method === 'POST' && path === '/api/quotes') {
      return jsonResponse(200, quoteDto('q-1'));
    }
    if (method === 'POST' && path === '/api/upload') {
      uploads += 1;
      return jsonResponse(200, { uploadKey: `key-${uploads}` });
    }
    if (method === 'POST' && path === '/api/transactions') {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      creates.push({
        method,
        path,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = createSteps[stepIdx++] ?? { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
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
    api: new ApiClient(fetchFn),
    creates,
  };
}

describe('desk/save.ts — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let disposers: Array<() => void> = [];

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    disposers = [];
  });

  afterEach(() => {
    disposers.forEach((d) => d());
    disposers = [];
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  async function readyStore(api: ApiClient) {
    let store!: ReturnType<typeof createDeskStore>;
    createRoot((dispose) => {
      disposers.push(dispose);
      store = createDeskStore({ api, path: OWNER_DESK, me: ME });
    });
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
    store.setNotes('initial note');
    store.setPaymentChecked(true);
    return store;
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { api, creates } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') },
    ]);

    const store = await readyStore(api);
    await expect(store.submit()).resolves.toBe('unknown');
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(2);
    expect(creates[0]!.key).toBeTruthy();
    expect(creates[1]!.key).toBe(creates[0]!.key);
    expect(creates[1]!.body).toBe(creates[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { api, creates } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') },
    ]);

    const store = await readyStore(api);
    await expect(store.submit()).resolves.toBe('unknown');
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(2);
    expect(creates[0]!.key).toBeTruthy();
    expect(creates[1]!.key).toBe(creates[0]!.key);
    expect(creates[1]!.body).toBe(creates[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { api, creates } = makeHarness([
      { kind: 'http', status: 400, body: { error: { code: 'validationFailed', message: 'ข้อมูลไม่ครบ' } } },
      { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') },
    ]);

    const store = await readyStore(api);
    await expect(store.submit()).resolves.toBe('refused');
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(2);
    expect(creates[0]!.key).toBeTruthy();
    expect(creates[1]!.key).toBeTruthy();
    expect(creates[1]!.key).not.toBe(creates[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { api, creates } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') },
    ]);

    const store = await readyStore(api);
    await expect(store.submit()).resolves.toBe('unknown');

    store.setNotes('edited note');
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(2);
    expect(creates[0]!.key).toBeTruthy();
    expect(creates[1]!.key).toBeTruthy();
    expect(creates[1]!.body).not.toBe(creates[0]!.body);
    expect(creates[1]!.key).not.toBe(creates[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { api, creates } = makeHarness([
      { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') },
      { kind: 'http', status: 200, body: txDto('tx-1', 'DRAFT') },
    ]);

    const store = await readyStore(api);
    await expect(store.submit()).resolves.toBe('saved');
    await expect(store.submit()).resolves.toBe('saved');

    expect(creates).toHaveLength(2);
    expect(creates[0]!.key).toBeTruthy();
    expect(creates[1]!.key).toBeTruthy();
    expect(creates[1]!.body).toBe(creates[0]!.body);
    expect(creates[1]!.key).not.toBe(creates[0]!.key);
  });
});
