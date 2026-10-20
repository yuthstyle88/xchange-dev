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

const SAMPLE_CREATE_OK_2 = makeBookingCreateOutput({
  id: '22222222-2222-4000-8000-000000000002',
  bookingNo: 'B261015-0002',
  status: 'CONFIRMED',
  sourceChannel: 'APP',
  branchId: BRANCH_ID,
  slotId: SLOT_1_ID,
  customerId: 'cust-1',
  shopSide: 'BUY',
  currency: 'USD',
  quantity: 100,
  qrToken: 'qr-token-test-2',
});

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

describe('Customer Book success banner reset (#2594)', () => {
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

  it('row 1: Book OK (banner BK-1), then a second submit fails -> No success banner; only the error', async () => {
    createResponder = (idx) => {
      if (idx === 0) return json(200, SAMPLE_CREATE_OK);
      return json(500, {
        error: { code: 'internal', message: 'Failed to process second booking' },
      });
    };

    const submitBtn = await mountAndFillForm();

    // First booking succeeds
    submitBtn.click();
    await flush();

    const banner1 = host.querySelector('.xc-book-success');
    expect(banner1).not.toBeNull();
    expect(banner1!.textContent).toContain(SAMPLE_CREATE_OK.bookingNo);

    // Second submit fails
    submitBtn.click();
    await flush();

    // Success banner must be gone, only error alert shown
    expect(host.querySelector('.xc-book-success')).toBeNull();
    const errorAlert = host.querySelector('[role="alert"]');
    expect(errorAlert).not.toBeNull();
  });

  it('row 2: Second submit in flight -> No old success banner while submitting', async () => {
    const secondPost = deferred<Response>();
    createResponder = (idx) => {
      if (idx === 0) return json(200, SAMPLE_CREATE_OK);
      return secondPost.promise;
    };

    const submitBtn = await mountAndFillForm();

    // First booking succeeds
    submitBtn.click();
    await flush();

    const banner1 = host.querySelector('.xc-book-success');
    expect(banner1).not.toBeNull();
    expect(banner1!.textContent).toContain(SAMPLE_CREATE_OK.bookingNo);

    // Second booking submitted, in flight
    submitBtn.click();
    await flush();

    // While second submit is in flight, the old success banner must not be shown
    expect(host.querySelector('.xc-book-success')).toBeNull();

    // Edge case: second booking finishes successfully, showing its own new id
    secondPost.resolve(json(200, SAMPLE_CREATE_OK_2));
    await flush();

    const banner2 = host.querySelector('.xc-book-success');
    expect(banner2).not.toBeNull();
    expect(banner2!.textContent).toContain(SAMPLE_CREATE_OK_2.bookingNo);
  });
});
