import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../shared/i18n';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeDeskConfig,
  makeDeskPaymentMethod,
  makeDeskRate,
  makeMe,
  makeMeRole,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
} from '../../shared/testing/fixtures';
import { step as paymentStep } from './steps/payment/step';
import { DeskBookingsScreen } from './bookings/DeskBookingsScreen';

const BRANCH_ID = 'XC-B01';
const SLOT_ID = 'slot-1';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: BRANCH_ID,
  permissions: ['BookingsDesk', 'RatesRead'],
});

const DESK_BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261023-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Somchai Jaidee',
      phoneLast4: '5678',
      slotId: SLOT_ID,
      startAt: '2026-10-23T09:00:00Z',
      endAt: '2026-10-23T09:30:00Z',
    }),
  ],
});

const DESK_CONFIG = makeDeskConfig({
  branchId: BRANCH_ID,
  branchName: BRANCH_ID,
  rates: [makeDeskRate({ code: 'USD' })],
  paymentMethods: [
    makeDeskPaymentMethod({ id: 'Cash', enabled: true, buy: true, sell: true, account: '', instruction: '' }),
    makeDeskPaymentMethod({ id: 'Bank Transfer', enabled: true, buy: true, sell: true, account: '123456', instruction: '' }),
  ],
});

function createMockPaymentStore() {
  const [method, setMethod] = createSignal('Bank Transfer');
  const [ref, setRef] = createSignal('');
  const [account, setAccount] = createSignal('');
  const [fee, setFee] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [paymentChecked, setPaymentChecked] = createSignal(false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  const [stepIndex, setStepIndex] = createSignal(3);

  const quote = makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'p', version: 1 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
        unitSize: '1',
        baseRate: '35.12000000',
        finalRate: '35.12000000',
        thbAmount: '3512.00',
        feeThb: '0.00',
        settleThb: '3512.00',
        publishedRateLineId: 'l',
      }),
    ],
    benefits: [],
    stale: false,
  });

  return {
    path: {},
    step: stepIndex,
    config: () => DESK_CONFIG,
    configError: () => undefined,
    type: () => 'BUY',
    quote: () => quote,
    quoteError: () => undefined,
    quoting: () => false,
    passport: () => new File([new Uint8Array([1])], 'p.jpg', { type: 'image/jpeg' }),
    method,
    setMethod,
    ref,
    setRef,
    account,
    setAccount,
    fee,
    setFee,
    bankFee,
    setBankFee,
    notes,
    setNotes,
    slip: () => undefined,
    setSlip: () => {},
    paymentChecked,
    setPaymentChecked,
    countConfirmed,
    setCountConfirmed,
    entry: () => undefined,
    go: setStepIndex,
    submit: vi.fn(),
  };
}

describe('desk-text-bounds (#4982)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
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

  function setupBookingsServer() {
    const posts: Array<{ path: string; body: unknown }> = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/me') {
          return new Response(JSON.stringify({ version: 1, changed: true, data: STAFF_TELLER }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          return new Response(JSON.stringify({ version: 1, changed: true, data: DESK_BOOKINGS }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        if (method === 'GET' && (u.pathname === '/api/v3/views/desk_config' || u.pathname === '/api/v3/views/staff_desk_config')) {
          return new Response(JSON.stringify({ version: 1, changed: true, data: DESK_CONFIG }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        if (method === 'POST') {
          posts.push({ path: u.pathname, body: JSON.parse(String(init?.body ?? '{}')) });
          return new Response(JSON.stringify({ ok: true }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(JSON.stringify({ error: { code: 'notFound' } }), { status: 404 });
      }),
    );
    return { posts };
  }

  // Row 1: Payment reference: 0..100
  it('Row 1: Payment reference holds at most 100 characters', () => {
    const store = createMockPaymentStore();
    dispose = render(() => paymentStep.component({ store: store as any }), host);

    const refInput = host.querySelector<HTMLInputElement>('input[name="ref"]');
    expect(refInput, 'ref input exists').not.toBeNull();
    expect(refInput!.maxLength).toBe(100);
  });

  // Row 2: Payment account: 0..100
  it('Row 2: Payment account holds at most 100 characters', () => {
    const store = createMockPaymentStore();
    dispose = render(() => paymentStep.component({ store: store as any }), host);

    const accountInput = host.querySelector<HTMLInputElement>('input[name="account"]');
    expect(accountInput, 'account input exists').not.toBeNull();
    expect(accountInput!.maxLength).toBe(100);
  });

  // Row 3: Transaction notes: 0..500 with n / 500 counter
  it('Row 3: Transaction notes holds at most 500 characters and shows n / 500 counter', () => {
    const store = createMockPaymentStore();
    dispose = render(() => paymentStep.component({ store: store as any }), host);

    const notesTextarea = host.querySelector<HTMLTextAreaElement>('textarea[name="notes"]');
    expect(notesTextarea, 'notes textarea exists').not.toBeNull();
    expect(notesTextarea!.maxLength).toBe(500);
    expect(host.textContent).toContain('0 / 500');

    store.setNotes('Notes testing');
    expect(host.textContent).toContain('13 / 500');
  });

  // Row 4: Booking display name: 1..100
  it('Row 4: Booking display name holds at most 100 characters', async () => {
    setupBookingsServer();
    dispose = render(() => <DeskBookingsScreen />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull();
    });

    const newBtn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!;
    expect(newBtn).not.toBeNull();
    newBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-modal input[name="displayName"]')).not.toBeNull();
    });

    const nameInput = host.querySelector<HTMLInputElement>('.xc-modal input[name="displayName"]')!;
    expect(nameInput.maxLength).toBe(100);
  });

  // Row 5: Desk booking cancel reason: 1..200 with n / 200 counter, and blank or N+1 sends nothing
  it('Row 5: Desk booking cancel reason holds at most 200 characters, shows n / 200 counter, and blank or N+1 sends nothing', async () => {
    const server = setupBookingsServer();
    dispose = render(() => <DeskBookingsScreen />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull();
    });

    const cancelBtn = host.querySelector<HTMLButtonElement>('[data-booking-id="bk-1"] button[data-action="cancel"]')!;
    expect(cancelBtn).not.toBeNull();
    cancelBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-modal input[name="reason"]')).not.toBeNull();
    });

    const reasonInput = host.querySelector<HTMLInputElement>('.xc-modal input[name="reason"]')!;
    const confirmBtn = host.querySelector<HTMLButtonElement>('.xc-modal button[data-action="confirm-cancel"]')!;

    expect(reasonInput.maxLength).toBe(200);
    expect(host.textContent).toContain('0 / 200');

    // Blank sends nothing
    reasonInput.value = '';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    confirmBtn.click();
    expect(server.posts).toHaveLength(0);

    // Typing updates counter
    reasonInput.value = 'Customer sick';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    expect(host.textContent).toContain('13 / 200');

    // N+1 (201 chars) sends nothing
    reasonInput.value = 'a'.repeat(201);
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    expect(host.textContent).toContain('201 / 200');
    confirmBtn.click();
    expect(server.posts).toHaveLength(0);

    // Valid <= 200 sends request
    reasonInput.value = 'Valid reason within 200 chars';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    confirmBtn.click();
    await vi.waitFor(() => {
      expect(server.posts).toHaveLength(1);
    });
    expect(server.posts[0]!.path).toBe('/api/bookings/bk-1/cancel');
    expect(server.posts[0]!.body).toEqual({
      reason: 'Valid reason within 200 chars',
      branch: BRANCH_ID,
    });
  });
});
