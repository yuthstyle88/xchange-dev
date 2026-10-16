import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { BookingCreateOutput } from '../../../shared/api/generated/bookings';
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

const SAMPLE_CREATE_OK: BookingCreateOutput = {
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
};

interface RecordedRequest {
  method: string;
  url: string;
  path: string;
  headers: Headers;
  rawBody: string | undefined;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
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

describe('Customer Book in-flight submit guard (#2405)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let createResponder: (callIndex: number, req: RecordedRequest) => Response | Promise<Response>;
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
        return json(200, SAMPLE_PREVIEW);
      }
      if (u.pathname === '/api/bookings' && method === 'POST') {
        const idx = bookingWrites().length - 1;
        return createResponder(idx, rec);
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

  function bookingWrites(): RecordedRequest[] {
    return sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/bookings');
  }

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

  it('row 1: Double click Book -> One POST', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      return json(409, {
        error: { code: 'idempotencyConflict', message: 'Request already in flight' },
      });
    };

    const submitBtn = await mountAndFillForm();
    expect(submitBtn.disabled).toBe(false);

    // Double click Book while first request is in flight
    submitBtn.click();
    submitBtn.click();

    expect(bookingWrites()).toHaveLength(1);
    expect(submitBtn.disabled).toBe(true);

    firstPost.resolve(json(200, SAMPLE_CREATE_OK));
    await flush();

    expect(bookingWrites()).toHaveLength(1);
    expect(submitBtn.disabled).toBe(false);
  });

  it('row 2: Click during the in-flight request -> No idempotencyConflict error; success banner only', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      return json(409, {
        error: { code: 'idempotencyConflict', message: 'Request already in flight' },
      });
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    // Click again while the first request is still in flight
    submitBtn.click();

    firstPost.resolve(json(200, SAMPLE_CREATE_OK));
    await flush();

    expect(host.querySelector('[role="alert"]')).toBeNull();
    const successBanner = host.querySelector('.xc-book-success');
    expect(successBanner).not.toBeNull();
    expect(successBanner!.textContent).toContain(SAMPLE_CREATE_OK.id);
  });

  it('row 3: First request fails on the network, then retry -> Same key as the first', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      if (idx === 1) {
        return json(200, SAMPLE_CREATE_OK);
      }
      return json(409, {
        error: { code: 'idempotencyConflict', message: 'Request already in flight' },
      });
    };

    const submitBtn = await mountAndFillForm();

    // Customer double-clicks Book; without the guard, the second click hits 409 and wipes the stored key
    submitBtn.click();
    submitBtn.click();

    // First request fails on the network
    firstPost.reject(new TypeError('Failed to fetch'));
    await flush();
    expect(submitBtn.disabled).toBe(false);

    // Customer retries -> sends the retry POST with the exact same idempotency key as the first request
    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    const key1 = writes[0]!.headers.get('idempotency-key')?.trim();
    const retryKey = writes.at(-1)!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(retryKey).toBe(key1);
    expect(writes).toHaveLength(2);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('row 4: Retry after a network error -> Same key (kept, Book.keyed.test.tsx:185)', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    expect(submitBtn.disabled).toBe(true);
    expect(bookingWrites()).toHaveLength(1);

    firstPost.reject(new TypeError('Failed to fetch'));
    await flush();
    expect(submitBtn.disabled).toBe(false);

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const key1 = writes[0]!.headers.get('idempotency-key')?.trim();
    const key2 = writes[1]!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('row 5: After a 2xx, submit the same input again -> New key (kept, Book.keyed.test.tsx:305)', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      return json(200, { ...SAMPLE_CREATE_OK, id: '22222222-2222-4000-8000-000000000002' });
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    expect(submitBtn.disabled).toBe(true);
    expect(bookingWrites()).toHaveLength(1);

    firstPost.resolve(json(200, SAMPLE_CREATE_OK));
    await flush();
    expect(submitBtn.disabled).toBe(false);

    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const key1 = writes[0]!.headers.get('idempotency-key')?.trim();
    const key2 = writes[1]!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('edge case 1 (§9): Enter twice behaves like a double click', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      return json(409, {
        error: { code: 'idempotencyConflict', message: 'Request already in flight' },
      });
    };

    await mountAndFillForm();
    const form = host.querySelector<HTMLFormElement>('form.xc-book-form')!;
    expect(form).not.toBeNull();

    // Pressing Enter twice dispatches submit on the form twice while the first POST is in flight
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    expect(bookingWrites()).toHaveLength(1);

    firstPost.resolve(json(200, SAMPLE_CREATE_OK));
    await flush();

    expect(bookingWrites()).toHaveLength(1);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.xc-book-success')).not.toBeNull();
  });

  it('edge case 2 (§9): The guard must not block the retry after a refusal (kept, Book.test.tsx:616)', async () => {
    const firstPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return firstPost.promise;
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    expect(submitBtn.disabled).toBe(true);
    expect(bookingWrites()).toHaveLength(1);

    // Server refuses with 409 slotUnavailable -> submitting clears in finally so customer can retry
    firstPost.resolve(
      json(409, {
        error: { code: 'slotUnavailable', message: 'Slot is no longer available' },
      }),
    );
    await flush();
    expect(submitBtn.disabled).toBe(false);
    expect(host.querySelector('[role="alert"]')).not.toBeNull();

    // Retry succeeds with a fresh key
    submitBtn.click();
    await flush();

    const writes = bookingWrites();
    expect(writes).toHaveLength(2);
    const key1 = writes[0]!.headers.get('idempotency-key')?.trim();
    const key2 = writes[1]!.headers.get('idempotency-key')?.trim();
    expect(key2).not.toBe(key1);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.xc-book-success')).not.toBeNull();
  });
});
