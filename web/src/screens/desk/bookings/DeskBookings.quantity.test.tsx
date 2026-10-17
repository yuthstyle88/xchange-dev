// #2198: Desk bookings (B02) — New booking quantity validation (§8, §9).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { BookingCreateOutput } from '../../../shared/api/generated/bookings';
import type { DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { th } from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';

const STAFF_TELLER: StaffMeView = {
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [{ role: 'teller', branchId: BRANCH_ID }],
  branchId: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
};

const INITIAL_DESK_BOOKINGS: DeskBookingsData = {
  items: [
    {
      id: 'BK-test-0001',
      bookingNo: 'B261016-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Alice Customer',
      phoneLast4: '4567',
      readiness: 'ready',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    },
  ],
};

interface RecordedPost {
  path: string;
  body: unknown;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen (B02) — New booking quantity validation (#2198 §8 & §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let bookingPosts: RecordedPost[];

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    bookingPosts = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          return jsonResponse(200, { version: 1, changed: true, data: INITIAL_DESK_BOOKINGS });
        }
        if (method === 'POST' && u.pathname === '/api/bookings') {
          const parsedBody = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
          bookingPosts.push({ path: u.pathname, body: parsedBody });
          return jsonResponse(200, {
            id: 'BK-new-001',
            bookingNo: 'B261016-0099',
            status: 'CONFIRMED',
            sourceChannel: 'DESK',
            branchId: BRANCH_ID,
            slotId: SLOT_1_ID,
            customerId: 'cust-new',
            shopSide: 'SELL',
            currency: 'USD',
            quantity: Number((parsedBody as { quantity?: number })?.quantity ?? 0),
            qrToken: null,
          } satisfies BookingCreateOutput);
        }
        return jsonResponse(404, { error: { code: 'notFound', message: u.pathname } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('staff');
    setLang('th');
  });

  async function openNewBookingModal(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const newBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]');
    expect(newBookingBtn, 'new booking button rendered').not.toBeNull();
    newBookingBtn!.click();
    await flush();

    const nameEl = host.querySelector<HTMLInputElement>('input[name="displayName"]')!;
    nameEl.value = 'Charlie Customer';
    nameEl.dispatchEvent(new Event('input', { bubbles: true }));
    nameEl.dispatchEvent(new Event('change', { bubbles: true }));

    const phoneEl = host.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneEl.value = '+66812345678';
    phoneEl.dispatchEvent(new Event('input', { bubbles: true }));
    phoneEl.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function setQuantity(value: string): HTMLInputElement {
    const qtyEl = host.querySelector<HTMLInputElement>('input[name="quantity"]');
    expect(qtyEl, 'quantity input exists').not.toBeNull();
    qtyEl!.value = value;
    qtyEl!.dispatchEvent(new Event('input', { bubbles: true }));
    qtyEl!.dispatchEvent(new Event('change', { bubbles: true }));
    return qtyEl!;
  }

  function clickSave(): void {
    const submitBtn = host.querySelector<HTMLButtonElement>('button[data-action="submit-booking"]');
    expect(submitBtn, 'submit booking button exists').not.toBeNull();
    submitBtn!.click();
  }

  function fieldRefusalElement(): Element | null {
    const qtyEl = host.querySelector<HTMLInputElement>('input[name="quantity"]');
    const group = qtyEl?.closest('.xc-form-group');
    return group?.querySelector('[role="alert"], .field-error') ?? null;
  }

  it("row 1: Quantity '' -> Save sends no POST and shows validation refusal beside the quantity field", async () => {
    await openNewBookingModal();

    const qtyEl = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    expect(qtyEl.value).toBe('');

    clickSave();
    await flush();

    expect(bookingPosts).toHaveLength(0);
    const refusal = fieldRefusalElement();
    expect(refusal, 'validation refusal shown beside quantity field').not.toBeNull();
    expect(refusal!.textContent?.trim()).toBe(th['error.validationFailed']);
    expect(qtyEl.getAttribute('aria-invalid')).toBe('true');
  });

  it("row 2: Quantity 'abc' or '0' -> Save sends no POST and shows validation refusal beside the quantity field", async () => {
    await openNewBookingModal();

    const qtyEl = setQuantity('abc');
    clickSave();
    await flush();

    expect(bookingPosts).toHaveLength(0);
    expect(fieldRefusalElement()?.textContent?.trim()).toBe(th['error.validationFailed']);
    expect(qtyEl.getAttribute('aria-invalid')).toBe('true');

    setQuantity('0');
    clickSave();
    await flush();

    expect(bookingPosts).toHaveLength(0);
    expect(fieldRefusalElement()?.textContent?.trim()).toBe(th['error.validationFailed']);
    expect(qtyEl.getAttribute('aria-invalid')).toBe('true');
  });

  it('row 3: Quantity 250 -> Save sends POST with quantity: 250 after refusing an empty quantity', async () => {
    await openNewBookingModal();

    // Empty quantity is refused first without sending POST
    clickSave();
    await flush();
    expect(bookingPosts).toHaveLength(0);
    expect(fieldRefusalElement()?.textContent?.trim()).toBe(th['error.validationFailed']);

    // Entering valid positive integer 250 sends POST with quantity: 250
    setQuantity('250');
    clickSave();
    await flush();

    expect(bookingPosts).toHaveLength(1);
    expect(bookingPosts[0]!.body).toEqual({
      branch: BRANCH_ID,
      branchId: BRANCH_ID,
      slotId: SLOT_1_ID,
      shopSide: 'SELL',
      currency: 'USD',
      quantity: 250,
      displayName: 'Charlie Customer',
      phone: '+66812345678',
    });
  });

  it("edge case: Quantity '2.5' -> Save sends no POST and shows validation refusal beside the quantity field", async () => {
    await openNewBookingModal();

    const qtyEl = setQuantity('2.5');
    clickSave();
    await flush();

    expect(bookingPosts).toHaveLength(0);
    const refusal = fieldRefusalElement();
    expect(refusal, 'validation refusal shown beside quantity field for fractional input').not.toBeNull();
    expect(refusal!.textContent?.trim()).toBe(th['error.validationFailed']);
    expect(qtyEl.getAttribute('aria-invalid')).toBe('true');
  });
});
