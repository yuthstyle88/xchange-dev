// F40: booking form reloads slot counts after a booking (#6328)
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
      id: BRANCH_ID,
      code: BRANCH_ID,
      name: 'สาขาสยามพารากอน',
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
    },
  ],
};

function makeSlots(available: number) {
  return {
    version: 1,
    changed: true,
    data: {
      slots: [
        {
          id: SLOT_1_ID,
          start: '2026-10-15T03:00:00+00:00',
          end: '2026-10-15T03:30:00+00:00',
          available,
        },
      ],
    },
  };
}

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

describe('F40: booking form reloads slot counts after a booking (#6328)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let slotsResponder: (req: RecordedRequest) => Response | Promise<Response>;
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

    slotsResponder = () => json(200, makeSlots(4));
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
        return slotsResponder(rec);
      }
      if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
        return json(200, SAMPLE_PREVIEW);
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

  it('slot view answers available: 4 before and 3 after; after a successful submit the slot view is read again and the option shows 3', async () => {
    let slotReads = 0;
    slotsResponder = () => {
      slotReads++;
      if (slotReads === 1) {
        return json(200, makeSlots(4));
      }
      return json(200, makeSlots(3));
    };

    const submitBtn = await mountAndFillForm();

    const slotOptionBefore = host.querySelector<HTMLOptionElement>(`option[value="${SLOT_1_ID}"]`);
    expect(slotOptionBefore).not.toBeNull();
    expect(slotOptionBefore!.textContent).toContain('4');

    submitBtn.click();
    await flush();

    await vi.waitFor(() => {
      expect(slotReads).toBeGreaterThanOrEqual(2);
      const slotOptionAfter = host.querySelector<HTMLOptionElement>(`option[value="${SLOT_1_ID}"]`);
      expect(slotOptionAfter).not.toBeNull();
      expect(slotOptionAfter!.textContent).toContain('3');
    });

    const successBanner = host.querySelector('.xc-book-success');
    expect(successBanner).not.toBeNull();
  });

  it('a failed reload after success keeps the success panel and the QR; it must not show the load error over them', async () => {
    let slotReads = 0;
    slotsResponder = () => {
      slotReads++;
      if (slotReads === 1) {
        return json(200, makeSlots(4));
      }
      return json(500, {
        error: { code: 'internal', message: 'Failed to reload slots' },
      });
    };

    const submitBtn = await mountAndFillForm();

    submitBtn.click();
    await flush();

    await vi.waitFor(() => {
      expect(slotReads).toBeGreaterThanOrEqual(2);
    });

    // Keeps success panel and QR
    const successBanner = host.querySelector('.xc-book-success');
    expect(successBanner).not.toBeNull();
    expect(successBanner!.textContent).toContain(SAMPLE_CREATE_OK.bookingNo);

    const qrElement = host.querySelector('.xc-booking-qr');
    expect(qrElement).not.toBeNull();
    expect(qrElement!.getAttribute('data-qr-token')).toBe(SAMPLE_CREATE_OK.qrToken);

    // Must not show the load error over them
    const globalError = host.querySelector('.xc-book-screen > .alert-error');
    expect(globalError).toBeNull();
  });
});
