// #4904 (T3) up-front tests: /app/book input validation (docs/design/web-input-validation.md).
// One test per §3 row K1–K7. A line at a field is `[data-field-error="<field>"]` inside the form;
// the page-top line is `.alert-error[role="alert"]` outside any field.
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
        start: '2099-10-15T03:00:00+00:00',
        end: '2099-10-15T03:30:00+00:00',
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
  sourceDate: '2099-10-15',
};

const SAMPLE_CREATE_OK = makeBookingCreateOutput({
  id: '11111111-1111-4000-8000-000000000001',
  bookingNo: 'B991015-0001',
  status: 'CONFIRMED',
  sourceChannel: 'APP',
  branchId: BRANCH_ID,
  slotId: SLOT_1_ID,
  customerId: 'cust-1',
  shopSide: 'BUY',
  currency: 'USD',
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

describe('#4904 Customer Book input validation (K1–K7)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let previewBodies: Record<string, unknown>[];
  let bookingBodies: Record<string, unknown>[];
  let createResponder: () => Response;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    previewBodies = [];
    bookingBodies = [];
    createResponder = () => json(200, SAMPLE_CREATE_OK);

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
        previewBodies.push(JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>);
        return json(200, SAMPLE_PREVIEW);
      }
      if (u.pathname === '/api/bookings' && method === 'POST') {
        bookingBodies.push(JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>);
        return createResponder();
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

  async function mount(opts: { pickSlot?: boolean } = {}): Promise<{
    form: HTMLFormElement;
    qtyInput: HTMLInputElement;
    sideSel: HTMLSelectElement;
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
    if (opts.pickSlot !== false) {
      const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
      slotSel.value = SLOT_1_ID;
      slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    }
    await flush();
    return {
      form: host.querySelector<HTMLFormElement>('form')!,
      qtyInput: host.querySelector<HTMLInputElement>('input[name="quantity"]')!,
      sideSel: host.querySelector<HTMLSelectElement>('select[name="shopSide"]')!,
      submitBtn: host.querySelector<HTMLButtonElement>('button[type="submit"]')!,
    };
  }

  function typeQty(qtyInput: HTMLInputElement, value: string): void {
    qtyInput.value = value;
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function pickSide(sideSel: HTMLSelectElement, value: 'BUY' | 'SELL'): void {
    sideSel.value = value;
    sideSel.dispatchEvent(new Event('change', { bubbles: true }));
  }

  /** The text of the line shown at a field, or null when there is none. */
  function fieldLine(field: string): string | null {
    return host.querySelector(`form [data-field-error="${field}"]`)?.textContent?.trim() || null;
  }

  /** The page-top line: an `.alert-error[role="alert"]` that sits in no field. */
  function pageTopLine(): string | null {
    const alerts = Array.from(host.querySelectorAll('.alert-error[role="alert"]'));
    const top = alerts.find((a) => !a.closest('.field') && !a.closest('[data-field-error]'));
    return top?.textContent?.trim() || null;
  }

  it('K1: pick BUY, type 100 → preview body customerSide CUSTOMER_BUY and amount.quantity "100"', async () => {
    const { qtyInput, sideSel } = await mount();
    pickSide(sideSel, 'BUY');
    typeQty(qtyInput, '100');
    await flush();

    expect(previewBodies.length, 'a preview is asked for').toBeGreaterThan(0);
    const last = previewBodies[previewBodies.length - 1]!;
    expect(last.customerSide).toBe('CUSTOMER_BUY');
    expect(last.amount).toEqual({ quantity: '100' });
  });

  it('K2: pick SELL → preview body customerSide CUSTOMER_SELL', async () => {
    const { qtyInput, sideSel } = await mount();
    pickSide(sideSel, 'SELL');
    typeQty(qtyInput, '100');
    await flush();

    expect(previewBodies.length, 'a preview is asked for').toBeGreaterThan(0);
    expect(previewBodies[previewBodies.length - 1]!.customerSide).toBe('CUSTOMER_SELL');
  });

  it('K3: quantity 1.23456 (5 decimals), submit → worded line at quantity; no POST /api/bookings', async () => {
    const { qtyInput, submitBtn } = await mount();
    typeQty(qtyInput, '1.23456');
    await flush();
    submitBtn.click();
    await flush();

    expect(bookingBodies, 'no create is sent').toHaveLength(0);
    const line = fieldLine('quantity');
    expect(line, 'a line shows at the quantity field').not.toBeNull();
    expect(line).not.toBe('');
    expect(line, 'the line is worded, never a raw code').not.toMatch(/tooManyDecimals|syntax|validationFailed/);
  });

  // §3 names 12345678901234567, but that has 17 integer digits and the server's
  // numeric(20,4) takes at most 16 (create.rs:164), so a correct check refuses it.
  // 9007199254740993 (2^53 + 1) is inside the bound and is the smallest integer that
  // Number() cannot hold: String(Number(it)) is "9007199254740992".
  it('K4: quantity 9007199254740993 (above 2^53, inside numeric(20,4)), submit → create body quantity is the exact string', async () => {
    const { qtyInput, submitBtn } = await mount();
    typeQty(qtyInput, '9007199254740993');
    await flush();
    submitBtn.click();
    await flush();

    expect(bookingBodies, 'one create is sent').toHaveLength(1);
    expect(bookingBodies[0]!.quantity).toBe('9007199254740993');
  });

  it('K5: no slot picked, submit → the native required check stops it; no POST /api/bookings', async () => {
    const { qtyInput, submitBtn } = await mount({ pickSlot: false });
    typeQty(qtyInput, '100');
    await flush();

    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    expect(slotSel.required, 'the slot select is required').toBe(true);
    expect(
      host.querySelector<HTMLSelectElement>('select[name="branchId"]')!.required,
      'the branch select is required',
    ).toBe(true);

    submitBtn.click();
    await flush();

    expect(slotSel.validity.valueMissing).toBe(true);
    expect(bookingBodies, 'no create is sent').toHaveLength(0);
  });

  it('K6: server 400 validationFailed on field quantity → the line shows at quantity, not at the page top', async () => {
    createResponder = () =>
      json(400, {
        error: { code: 'validationFailed', field: 'quantity', message: 'ข้อมูลไม่ถูกต้อง' },
      });
    const { qtyInput, submitBtn } = await mount();
    typeQty(qtyInput, '100');
    await flush();
    submitBtn.click();
    await flush();

    expect(bookingBodies, 'the create was sent').toHaveLength(1);
    expect(fieldLine('quantity')).toBe(
      sharedTh['input.invalid'].replace('{field}', screenTh.quantity),
    );
    expect(pageTopLine(), 'nothing at the page top').toBeNull();
  });

  it('K7: pick "buy foreign currency" → create body shopSide BUY (unchanged until the owner answers)', async () => {
    const { qtyInput, sideSel, submitBtn } = await mount();
    pickSide(sideSel, 'BUY');
    typeQty(qtyInput, '100');
    await flush();
    submitBtn.click();
    await flush();

    expect(bookingBodies, 'one create is sent').toHaveLength(1);
    expect(bookingBodies[0]!.shopSide).toBe('BUY');
  });
});
