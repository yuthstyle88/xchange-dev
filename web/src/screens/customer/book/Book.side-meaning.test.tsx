// F33 (#564 issuecomment-6657891388, customer simulation on xc-v3 at 7caf1e7, 2026-11-05): the booking form says
// "ซื้อเงินต่างประเทศ" and estimates at the customer-buy price, but stored `shop_side` BUY; the sale of the same
// booking is shop SELL. One meaning from here on: `shopSide` is the shop's side, the label is the customer's
// words, and the customer's side is the opposite of the shop's. The kiosk already uses `type: SELL` for a
// customer who buys.
//
// Written before the implementation by the ticket's author (claude/33691c06, sub-controller of the xchange lane
// controller claude/022a7453). Do not edit, delete or weaken these tests; add your own in another file.
// The harness is Book.keyed.test.tsx's, copied.
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

describe('F33: the booking side has one meaning', () => {
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

  /** Chooses the side option whose visible text is `label` (the customer's words). */
  function chooseByLabel(label: string): void {
    const sel = host.querySelector<HTMLSelectElement>('select[name="shopSide"]')!;
    const option = Array.from(sel.options).find((o) => o.textContent?.trim() === label);
    expect(option, `an option reads "${label}"`).toBeDefined();
    sel.value = option!.value;
    sel.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function lastBody(path: string): Record<string, unknown> {
    const rec = sentRequests.filter((r) => r.method === 'POST' && r.path === path).at(-1);
    expect(rec, `POST ${path} was sent`).toBeDefined();
    return JSON.parse(rec!.rawBody ?? '{}') as Record<string, unknown>;
  }

  it('F33-1: "ซื้อเงินต่างประเทศ" estimates at the customer-buy price and books shop SELL', async () => {
    const submitBtn = await mountAndFillForm();
    chooseByLabel('ซื้อเงินต่างประเทศ');
    await flush();

    expect(lastBody('/api/v3/compute/planner_preview').customerSide).toBe('CUSTOMER_BUY');
    submitBtn.click();
    await flush();
    expect(lastBody('/api/bookings').shopSide).toBe('SELL');
  });

  it('F33-2: "ขายเงินต่างประเทศ" estimates at the customer-sell price and books shop BUY', async () => {
    const submitBtn = await mountAndFillForm();
    chooseByLabel('ขายเงินต่างประเทศ');
    await flush();

    expect(lastBody('/api/v3/compute/planner_preview').customerSide).toBe('CUSTOMER_SELL');
    submitBtn.click();
    await flush();
    expect(lastBody('/api/bookings').shopSide).toBe('BUY');
  });

  it('F33-3: the preview and the booking never name the same side for the shop and the customer', async () => {
    for (const label of ['ซื้อเงินต่างประเทศ', 'ขายเงินต่างประเทศ']) {
      sentRequests = [];
      dispose?.();
      host.innerHTML = '';
      const submitBtn = await mountAndFillForm();
      chooseByLabel(label);
      await flush();
      const customer = lastBody('/api/v3/compute/planner_preview').customerSide;
      submitBtn.click();
      await flush();
      const shop = lastBody('/api/bookings').shopSide;
      expect(`${String(customer)}/${String(shop)}`, label).toMatch(/^(CUSTOMER_BUY\/SELL|CUSTOMER_SELL\/BUY)$/);
    }
  });
});
