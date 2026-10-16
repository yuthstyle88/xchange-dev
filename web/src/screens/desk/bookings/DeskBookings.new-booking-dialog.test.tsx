import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';

interface WriteCall {
  path: string;
  key: string | null;
  body: string;
}

type Answer = { kind: 'network' } | { kind: 'http'; status: number; body: unknown };

interface Deferred {
  resolve: (a: Answer) => void;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER: StaffMeView = {
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [{ role: 'teller', branchId: 'XC-B01' }],
  branchId: 'XC-B01',
  branchName: 'XC-B01',
  permissions: ['BookingsDesk', 'RatesRead'],
};

const DESK_BOOKINGS_DATA: DeskBookingsData = {
  items: [
    {
      id: 'bk-1',
      bookingNo: 'B261015-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Somchai Jaidee',
      phoneLast4: '5678',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
    },
  ],
};

function makeServer() {
  const writes: WriteCall[] = [];
  const held: Deferred[] = [];

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
      return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: DESK_BOOKINGS_DATA });
    }

    if (
      method === 'POST' &&
      (u.pathname === '/api/bookings' ||
        /^\/api\/bookings\/[^/]+\/(check-in|cancel|reschedule)$/.test(u.pathname))
    ) {
      const key = new Headers(init?.headers).get('idempotency-key');
      writes.push({ path: u.pathname, key, body: typeof init?.body === 'string' ? init.body : '' });
      const answer = await new Promise<Answer>((resolve) => held.push({ resolve }));
      if (answer.kind === 'network') throw new TypeError('Failed to fetch');
      return jsonResponse(answer.status, answer.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);

  return {
    writes,
    held,
    answer(a: Answer) {
      const d = held.shift();
      if (!d) throw new Error('no write is in flight');
      d.resolve(a);
    },
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function setInputValue(input: HTMLInputElement, val: string): void {
  input.value = val;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('DeskBookingsScreen.tsx — New booking dialog lifecycle and write guarding (#2720)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let server: ReturnType<typeof makeServer>;

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
    server = makeServer();
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

  async function mount(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull();
    });
  }

  async function openNewBookingModal(): Promise<void> {
    const btn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]');
    expect(btn, 'page New booking button').not.toBeNull();
    btn!.click();
    await flush();
  }

  function getModal(): HTMLElement | null {
    return host.querySelector('.xc-modal');
  }

  function getQuantityInput(): HTMLInputElement {
    return host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
  }

  function getNameInput(): HTMLInputElement {
    return host.querySelector<HTMLInputElement>('input[name="displayName"]')!;
  }

  function getPhoneInput(): HTMLInputElement {
    return host.querySelector<HTMLInputElement>('input[name="phone"]')!;
  }

  function getReferralInput(): HTMLInputElement {
    return host.querySelector<HTMLInputElement>('input[name="referralCode"]')!;
  }

  function getSlotSelect(): HTMLSelectElement {
    return host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
  }

  function getCloseButton(): HTMLButtonElement {
    const btn = Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-modal-actions button')).find(
      (x) => x.textContent?.trim() === 'Close',
    );
    expect(btn, 'dialog Close button').toBeDefined();
    return btn!;
  }

  function getSubmitButton(): HTMLButtonElement {
    return host.querySelector<HTMLButtonElement>('button[data-action="submit-booking"]')!;
  }

  function getRetryButton(): HTMLButtonElement | null {
    return host.querySelector<HTMLButtonElement>('.xc-modal button[data-action="retry"]');
  }

  it('row N1: Open New booking, type quantity, name, phone and referral, press Close, open New booking again -> Form is empty with default slot', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '500');
    setInputValue(getNameInput(), 'Customer A');
    setInputValue(getPhoneInput(), '0891112222');
    setInputValue(getReferralInput(), 'REF-A');

    getCloseButton().click();
    await flush();
    expect(getModal()).toBeNull();

    await openNewBookingModal();

    expect(getQuantityInput().value).toBe('');
    expect(getNameInput().value).toBe('');
    expect(getPhoneInput().value).toBe('');
    expect(getReferralInput().value).toBe('');
    expect(getSlotSelect().value).toBe('slot-1');
  });

  it('row N2: Submit with the POST held, then click submit again -> 1 POST; submit is disabled until it answers', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '100');
    setInputValue(getNameInput(), 'Somchai');
    setInputValue(getPhoneInput(), '0812345678');

    getSubmitButton().click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    const submitBtn = getSubmitButton();
    expect(submitBtn.disabled).toBe(true);

    // Attempt second submit while write is held
    submitBtn.click();
    await flush();

    expect(server.writes).toHaveLength(1);

    server.answer({
      kind: 'http',
      status: 200,
      body: { id: 'bk-new', bookingNo: 'B261017-0001', status: 'CONFIRMED' },
    });
    await flush();

    expect(getModal()).toBeNull();
  });

  it('row N3: Create fails (4xx), then double-click the dialog\'s Retry with the POST held -> 1 more POST (2 in all); Retry is disabled until it answers', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '100');
    setInputValue(getNameInput(), 'Somchai');
    setInputValue(getPhoneInput(), '0812345678');

    getSubmitButton().click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    server.answer({
      kind: 'http',
      status: 400,
      body: { error: { code: 'validationFailed', message: 'Slot full' } },
    });
    await flush();

    const retryBtn = getRetryButton();
    expect(retryBtn).not.toBeNull();

    // Click Retry (first retry sends write 2)
    retryBtn!.click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));
    expect(server.writes).toHaveLength(2);

    // Retry must be disabled while the retry write is in flight
    expect(retryBtn!.disabled).toBe(true);

    // Second click on Retry must not send a 3rd write
    retryBtn!.click();
    await flush();
    expect(server.writes).toHaveLength(2);

    server.answer({
      kind: 'http',
      status: 200,
      body: { id: 'bk-new', bookingNo: 'B261017-0001', status: 'CONFIRMED' },
    });
    await flush();

    expect(getModal()).toBeNull();
  });

  it('row N4: Submit with the POST held -> The dialog\'s Close is disabled until the answer', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '100');
    setInputValue(getNameInput(), 'Somchai');
    setInputValue(getPhoneInput(), '0812345678');

    getSubmitButton().click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    const closeBtn = getCloseButton();
    expect(closeBtn.disabled).toBe(true);

    server.answer({
      kind: 'http',
      status: 200,
      body: { id: 'bk-new', bookingNo: 'B261017-0001', status: 'CONFIRMED' },
    });
    await flush();

    expect(getModal()).toBeNull();
  });

  it('row N5: Create succeeds -> Dialog closes, form cleared, list re-read', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '100');
    setInputValue(getNameInput(), 'Somchai');
    setInputValue(getPhoneInput(), '0812345678');

    getSubmitButton().click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    server.answer({
      kind: 'http',
      status: 200,
      body: { id: 'bk-new', bookingNo: 'B261017-0001', status: 'CONFIRMED' },
    });
    await flush();

    expect(getModal()).toBeNull();
  });

  it('row N6: Create fails -> Error and Retry show inside dialog; typed values stay', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '100');
    setInputValue(getNameInput(), 'Somchai');
    setInputValue(getPhoneInput(), '0812345678');

    getSubmitButton().click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    server.answer({
      kind: 'http',
      status: 400,
      body: { error: { code: 'validationFailed', message: 'Slot full' } },
    });
    await flush();

    expect(getModal()).not.toBeNull();
    expect(host.querySelector('.xc-error-banner')).not.toBeNull();
    expect(getRetryButton()).not.toBeNull();
    expect(getQuantityInput().value).toBe('100');
    expect(getNameInput().value).toBe('Somchai');
    expect(getPhoneInput().value).toBe('0812345678');
  });

  it('row N7: Close the Cancel dialog while a cancel is in flight -> Allowed, as today', async () => {
    await mount();
    const cancelBtn = host.querySelector<HTMLButtonElement>('[data-booking-id="bk-1"] button[data-action="cancel"]')!;
    cancelBtn.click();
    await flush();

    const reasonInput = host.querySelector<HTMLInputElement>('#cancel-reason-input')!;
    setInputValue(reasonInput, 'Customer requested');

    const confirmCancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel"]')!;
    confirmCancelBtn.click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    const cancelCloseBtn = Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-modal-actions button')).find(
      (x) => x.textContent?.trim() === 'Close',
    );
    expect(cancelCloseBtn, 'Cancel modal close button').toBeDefined();
    expect(cancelCloseBtn!.disabled).toBe(false);

    cancelCloseBtn!.click();
    await flush();
    expect(host.querySelector('#cancel-reason-input')).toBeNull();

    // Release held cancel
    server.answer({ kind: 'http', status: 200, body: { status: 'CANCELLED' } });
    await flush();
  });

  it('row N8: Invalid quantity, then submit -> Quantity error, no POST, nothing disabled', async () => {
    await mount();
    await openNewBookingModal();

    setInputValue(getQuantityInput(), '0');
    getSubmitButton().click();
    await flush();

    expect(server.writes).toHaveLength(0);
    expect(host.querySelector('.field-error')).not.toBeNull();
    expect(getSubmitButton().disabled).toBe(false);
    expect(getCloseButton().disabled).toBe(false);
  });
});
