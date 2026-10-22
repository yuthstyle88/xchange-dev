// RW-2 (#2010, design docs/design/xchange-v3-web.md §12.23):
// C02 Book.tsx preview and booking create go through ApiClient and wordedError.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeBookingCreateOutput } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';
import { th as screenTh } from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-4000-8000-000000000001';

const SAMPLE_BRANCHES = {
  branches: [
    {
      id: BRANCH_ID,
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

describe('RW-2: C02 BookScreen preview and booking create through ApiClient & wordedError (#2010 §9)', () => {
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

  async function mountAndFillForm(url = '/app/book'): Promise<HTMLButtonElement> {
    window.history.replaceState({}, '', url);
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

  it('row 1: preview and create — each request carries x-xchange-request: 1; create also carries Idempotency-Key', async () => {
    const submitBtn = await mountAndFillForm();
    submitBtn.click();
    await flush();

    const previewReq = sentRequests.find(
      (r) => r.method === 'POST' && r.path === '/api/v3/compute/planner_preview',
    );
    expect(previewReq, 'POST /api/v3/compute/planner_preview was sent').toBeDefined();
    expect(
      previewReq!.headers.get('x-xchange-request'),
      'planner_preview carries x-xchange-request: 1',
    ).toBe('1');

    const createReq = sentRequests.find((r) => r.method === 'POST' && r.path === '/api/bookings');
    expect(createReq, 'POST /api/bookings was sent').toBeDefined();
    expect(
      createReq!.headers.get('x-xchange-request'),
      'booking create carries x-xchange-request: 1',
    ).toBe('1');
    expect(
      createReq!.headers.get('idempotency-key')?.trim(),
      'booking create carries non-empty Idempotency-Key',
    ).toBeTruthy();
  });

  it('row 2: create retry after a network failure reuses the same key and byte-equal body (still has branch)', async () => {
    let attempts = 0;
    createResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, SAMPLE_CREATE_OK);
    };

    const submitBtn = await mountAndFillForm();

    // First attempt fails with network TypeError
    submitBtn.click();
    await flush();

    // Second attempt retries without changing form fields
    submitBtn.click();
    await flush();

    const createPosts = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/bookings',
    );
    expect(createPosts).toHaveLength(2);
    const [first, second] = createPosts;
    expect(first!.headers.get('idempotency-key')?.trim()).toBeTruthy();
    expect(second!.headers.get('idempotency-key')).toBe(first!.headers.get('idempotency-key'));
    expect(second!.rawBody).toBe(first!.rawBody);
    const parsed = JSON.parse(second!.rawBody!);
    expect(parsed.branch).toBe(BRANCH_ID);
  });

  it("row 3: create answering 500 {code:'roleNotAllowedHere'} shows th['error.serverError']", async () => {
    let refusalCode = 'roleNotAllowedHere';
    createResponder = () =>
      json(500, {
        error: {
          code: refusalCode,
          message: 'Role is not allowed on this route',
        },
      });

    const submitBtn = await mountAndFillForm();
    submitBtn.click();
    await flush();

    let alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(alert!.textContent).not.toContain(screenTh['error.general']);
    expect(alert!.textContent).not.toContain('Role is not allowed on this route');

    // Also verify a screen-keyed code (capacityFull) on 500 still answers serverError (not key-first)
    refusalCode = 'capacityFull';
    submitBtn.click();
    await flush();

    alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain(screenTh['error.capacityFull']);
  });

  it('row 4: create answering 409 with an unknown code and a sentence shows that sentence (W10 (4))', async () => {
    const sentence = 'ไม่สามารถจองคิวได้เนื่องจากสาขากำลังปิดรอบ';
    createResponder = () =>
      json(409, {
        error: {
          code: 'bookingWindowLocked',
          message: sentence,
        },
      });

    const submitBtn = await mountAndFillForm();
    submitBtn.click();
    await flush();

    const alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sentence);
    expect(alert!.textContent).not.toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain(screenTh['error.general']);
    expect(alert!.textContent).not.toContain('bookingWindowLocked');
  });

  it('row 5: preview network TypeError shows error.network', async () => {
    previewResponder = () => {
      throw new TypeError('Failed to fetch');
    };

    await mountAndFillForm();

    const alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.network']);
    expect(alert!.textContent).not.toContain('Failed to fetch');
    expect(alert!.textContent).not.toContain(screenTh['error.general']);
  });

  it("row 6: ?ref=CODE sends referralCode: 'CODE' in the create body (unchanged)", async () => {
    const submitBtn = await mountAndFillForm('/app/book?ref=CODE');
    submitBtn.click();
    await flush();

    const createReq = sentRequests.find((r) => r.method === 'POST' && r.path === '/api/bookings');
    expect(createReq).toBeDefined();
    const parsed = JSON.parse(createReq!.rawBody!);
    expect(parsed.referralCode).toBe('CODE');
    expect(parsed.branch).toBe(BRANCH_ID);
  });

  it("row 7 (rework): validationFailed with field 'quoteId' while ?ref= is filled shows error.validationFailed and not the referral text", async () => {
    createResponder = () =>
      json(400, {
        error: {
          code: 'validationFailed',
          field: 'quoteId',
          message: 'Quote ID is invalid',
        },
      });

    const submitBtn = await mountAndFillForm('/app/book?ref=CODE');
    submitBtn.click();
    await flush();

    expect(host.textContent).not.toContain(screenTh['error.invalidReferralCode']);
    expect(host.querySelector('#book-referral-error')).toBeNull();
    const alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert, 'global alert is shown for quoteId validationFailed').not.toBeNull();
    expect(alert!.textContent).toContain(screenTh['error.validationFailed']);
    expect(alert!.textContent).not.toContain(screenTh['error.invalidReferralCode']);
  });
});


