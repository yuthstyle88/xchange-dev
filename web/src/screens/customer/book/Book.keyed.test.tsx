// KW-6a (#2103, design docs/design/xchange-v3-web.md §12.24):
// C02 Book.tsx booking create idempotency key lifecycle through mocked fetch only.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeBookingCreateOutput } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-4000-8000-000000000001';

const SAMPLE_BRANCHES = {
  branches: [
    {
      code: BRANCH_ID,
      name: 'สาขาสยามพารากอน',
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
    },
  ],
};

const SAMPLE_SLOTS = {
  version: 1,
  changed: true,
  data: {
    slots: [
      {
        id: SLOT_1_ID,
        start: '2026-10-15T03:00:00+00:00',
        end: '2026-10-15T03:30:00+00:00',
        available: 3,
      },
    ],
  },
};

const SAMPLE_PREVIEW = {
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-15',
};

const SAMPLE_CREATE_OK = makeBookingCreateOutput({
  id: '11111111-1111-4000-8000-000000000001',
  bookingNo: 'B261015-0001',
  status: 'CONFIRMED',
  sourceChannel: 'APP',
  branchId: BRANCH_ID,
  slotId: SLOT_1_ID,
  customerId: 'cust-1',
  shopSide: 'BUY',
  currency: 'USD',
  quantity: 100,
  qrToken: 'qr-token-test-xyz',
});

interface RecordedRequest {
  method: string;
  url: string;
  path: string;
  headers: Headers;
  rawBody: string | undefined;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('KW-6a: C02 BookScreen keyedWrite behaviour (#2103 §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let previewResponder: (req: RecordedRequest) => Response | Promise<Response>;
  let createResponder: (req: RecordedRequest) => Response | Promise<Response>;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    previewResponder = () => json(200, SAMPLE_PREVIEW);
    createResponder = () => json(200, SAMPLE_CREATE_OK);

    fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const rawBody = typeof init?.body === 'string' ? init.body : undefined;

      const rec: RecordedRequest = {
        method,
        url: input,
        path: u.pathname,
        headers,
        rawBody,
      };
      sentRequests.push(rec);

      if (u.pathname === '/api/public/host') {
        return json(200, { org: 'XC', env: 'test' });
      }
      if (u.pathname.startsWith('/api/public/orgs/')) {
        return json(200, SAMPLE_BRANCHES);
      }
      if (u.pathname === '/api/v3/views/booking_slots') {
        return json(200, SAMPLE_SLOTS);
      }
      if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
        return previewResponder(rec);
      }
      if (u.pathname === '/api/bookings' && method === 'POST') {
        return createResponder(rec);
      }
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    window.history.replaceState({}, '', '/app/book');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  async function mountAndFillForm(): Promise<HTMLButtonElement> {
    window.history.replaceState({}, '', '/app/book');
    dispose = render(() => <BookScreen fetch={fetchStub} />, host);
    await flush();

    await vi.waitFor(() => {
      expect(host.querySelector(`option[value="${BRANCH_ID}"]`)).not.toBeNull();
    });

    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = BRANCH_ID;
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(host.querySelector(`option[value="${SLOT_1_ID}"]`)).not.toBeNull();
    });

    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = SLOT_1_ID;
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    expect(submitBtn).not.toBeNull();
    return submitBtn;
  }

  function bookingWrites(): RecordedRequest[] {
    return sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/bookings');
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and a byte-equal body', async () => {
    let attempts = 0;
    createResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('row 2: submit -> 500 -> submit again sends the same key', async () => {
    let attempts = 0;
    createResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(500, { error: { code: 'internalError', message: 'Internal server error' } });
      }
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    let attempts = 0;
    createResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(409, { error: { code: 'slotUnavailable', message: 'Slot is no longer available' } });
      }
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    let attempts = 0;
    createResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '250';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(second!.rawBody).not.toBe(first!.rawBody);
    expect(JSON.parse(second!.rawBody!).quantity).toBe(250);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('rework: submit -> 500 -> preview 500 -> preview 200 -> submit with the same input reuses the same key', async () => {
    let attempts = 0;
    createResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(500, { error: { code: 'internalError', message: 'Internal server error' } });
      }
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    previewResponder = () =>
      json(500, { error: { code: 'internalError', message: 'Preview failed' } });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    previewResponder = () => json(200, SAMPLE_PREVIEW);
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });
});
