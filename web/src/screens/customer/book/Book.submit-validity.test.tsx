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

describe('Customer Book submit constraint validation (#4811)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let bookingPosts: number;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    bookingPosts = 0;

    fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();

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
        bookingPosts++;
        return json(200, SAMPLE_CREATE_OK);
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

  async function mountWithQuantity(qty: string): Promise<{
    qtyInput: HTMLInputElement;
    submitBtn: HTMLButtonElement;
  }> {
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
    qtyInput.value = qty;
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    return { qtyInput, submitBtn };
  }

  it.each(['0', '-50'])(
    'with amount %s, clicking submit sends no POST /api/bookings and the field shows its own validity message',
    async (amount) => {
      const { qtyInput, submitBtn } = await mountWithQuantity(amount);

      let invalidFired = false;
      qtyInput.addEventListener('invalid', () => {
        invalidFired = true;
      });

      submitBtn.click();
      await flush();

      expect(bookingPosts).toBe(0);
      expect(qtyInput.validity.rangeUnderflow).toBe(true);
      expect(qtyInput.validity.valid).toBe(false);
      expect(qtyInput.validationMessage).not.toBe('');
      expect(invalidFired).toBe(true);
    },
  );
});
