import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { errorText, setLang } from '../../../../shared/i18n';
import { en as sharedEn } from '../../../../shared/i18n/en';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { OWNER_DESK } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import * as texts from './i18n';
import { step as customerStep } from './step';

const ME = () => ({
  id: 'u-1',
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  permissions: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});
const NO_ME = () => undefined;

type Call = { method: string; path: string; body: unknown };

interface CalledRow {
  id: string;
  displayNumber: string;
  serviceType: 'WALK_IN' | 'BOOKING';
  status: 'CALLED' | 'SERVING';
  tellerId: string | null;
}

interface BookingItem {
  id: string;
  bookingNo: string;
  status: string;
  ticketNumber: string | null;
}

function stubServer(opts: {
  called?: CalledRow[];
  bookings?: BookingItem[];
  boardNetworkFailure?: boolean;
}) {
  const calls: Call[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, body });
    if (method === 'GET' && path === '/api/v3/views/queue_board') {
      if (opts.boardNetworkFailure) {
        throw new TypeError('Failed to fetch');
      }
      return new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: {
            waiting: [],
            called: (opts.called ?? []).map((c) => ({ ...c, tellerName: null, calledAt: null })),
            myCounter: { status: 'ACTIVE' },
          },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }
    if (method === 'GET' && path === '/api/v3/views/desk_bookings') {
      return new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: {
            items: (opts.bookings ?? []).map((b) => ({
              ...b,
              customerDisplayName: null,
              phoneLast4: null,
              readiness: 'READY',
              slotId: 'slot-1',
              startAt: '2026-10-31T09:00:00Z',
              endAt: '2026-10-31T09:30:00Z',
            })),
          },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }
    if (method === 'POST' && path === '/api/rpc' && (body as { action?: string })?.action === 'reserveCustomer') {
      return new Response(JSON.stringify({ customerId: 'CU-WALKIN-1', existing: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ error: { code: 'notFound', message: 'not found' } }), {
      status: 404,
      headers: { 'content-type': 'application/json' },
    });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    reserves: () =>
      calls.filter(
        (c): c is Call & { body: { action: string; payload: Record<string, unknown> } } =>
          c.method === 'POST' && (c.body as { action?: string })?.action === 'reserveCustomer',
      ),
    boardReads: () => calls.filter((c) => c.path === '/api/v3/views/queue_board'),
  };
}

const HELD_BOOKING: CalledRow = {
  id: 't-9',
  displayNumber: 'B003',
  serviceType: 'BOOKING',
  status: 'SERVING',
  tellerId: 'u-1',
};
const HELD_WALK_IN: CalledRow = {
  id: 't-4',
  displayNumber: 'W004',
  serviceType: 'WALK_IN',
  status: 'SERVING',
  tellerId: 'u-1',
};
const BK9: BookingItem = {
  id: 'bk-9',
  bookingNo: 'B261031-0003',
  status: 'IN_SERVICE',
  ticketNumber: 'B003',
};

let disposers: (() => void)[] = [];
let host: HTMLDivElement;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  for (const d of disposers) d();
  disposers = [];
  host.remove();
  vi.restoreAllMocks();
  setLang('th');
});

function atCustomerStep(s: ReturnType<typeof stubServer>, me: () => unknown = ME): DeskStore {
  let store!: DeskStore;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path: OWNER_DESK, me: me as never });
  });
  store.go(1);
  disposers.push(render(() => customerStep.component({ store }), host));
  return store;
}

const clickSkip = () => {
  const btn = [...host.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => b.textContent?.includes(texts.th.skip) || b.textContent?.includes(texts.en.skip),
  );
  expect(btn).toBeDefined();
  btn!.click();
};

describe('F32b: desk passport skip refuses on a booking, and booking refusals are worded (#6322)', () => {
  it('row 1: no booking held, held-ticket read answers none/walkIn -> skip as today (WALK-IN reserved, step 2)', async () => {
    for (const called of [[], [HELD_WALK_IN]]) {
      const s = stubServer({ called });
      const store = atCustomerStep(s);
      clickSkip();
      await vi.waitFor(() => expect(store.step()).toBe(2));
      expect(s.reserves()).toHaveLength(1);
      expect(s.reserves()[0]!.body.payload).toMatchObject({
        name: 'WALK-IN',
        passportNo: 'WALKIN',
        nationality: 'XXX',
      });
      expect(s.reserves()[0]!.body.payload).not.toHaveProperty('bookingId');
      for (const d of disposers) d();
      disposers = [];
      host.replaceChildren();
    }
  });

  it('row 2: store.heldBooking() set -> no reserveCustomer call; skipNotOnBooking shown; still step 1', async () => {
    const s = stubServer({});
    const store = atCustomerStep(s);
    store.setHeldBooking({ bookingId: 'bk-7', ticketId: 't-7', displayNumber: 'B001' });

    clickSkip();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toBe(texts.th.skipNotOnBooking);
    });
    expect(s.reserves()).toHaveLength(0);
    expect(s.boardReads()).toHaveLength(0);
    expect(store.step()).toBe(1);

    setLang('en');
    clickSkip();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toBe(texts.en.skipNotOnBooking);
    });
    expect(s.reserves()).toHaveLength(0);
    expect(store.step()).toBe(1);
  });

  it('row 3: entry carries bookingId -> no reserveCustomer call; skipNotOnBooking shown; still step 1', async () => {
    const s = stubServer({});
    const store = atCustomerStep(s);
    store.setEntry({
      kind: 'kiosk',
      id: 'bk-entry-1',
      locked: true,
      banner: () => '',
      identityConfirmed: () => false,
      setIdentityConfirmed: () => undefined,
      payloadExtras: () => ({ bookingId: 'bk-entry-1' }),
      refusal: () => undefined,
    });

    clickSkip();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toBe(texts.th.skipNotOnBooking);
    });
    expect(s.reserves()).toHaveLength(0);
    expect(s.boardReads()).toHaveLength(0);
    expect(store.step()).toBe(1);
  });

  it('row 4: held-ticket read answers kind: booking -> no reserveCustomer call; skipNotOnBooking shown; still step 1; booking kept in store.setHeldBooking', async () => {
    const s = stubServer({ called: [HELD_BOOKING], bookings: [BK9] });
    const store = atCustomerStep(s);

    clickSkip();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toBe(texts.th.skipNotOnBooking);
    });
    expect(s.reserves()).toHaveLength(0);
    expect(store.step()).toBe(1);
    expect(store.heldBooking()).toEqual({ bookingId: 'bk-9', ticketId: 't-9', displayNumber: 'B003' });
  });

  it('row 5: errorText(bookingNotThisCustomer, bookingNotThisCustomer) in Thai -> the Thai sentence, not the code', () => {
    expect(errorText('bookingNotThisCustomer', 'bookingNotThisCustomer')).toBe(
      'พาสปอร์ตนี้ไม่ใช่ของลูกค้าที่จองคิวนี้ กรุณาตรวจพาสปอร์ตอีกครั้ง',
    );
    expect(errorText('bookingNotThisCustomer', 'bookingNotThisCustomer')).not.toBe('bookingNotThisCustomer');
    expect(errorText('bookingNotUsable', 'bookingNotUsable')).toBe(
      'ใช้การจองนี้ไม่ได้ กรุณาตรวจคิวจองอีกครั้ง',
    );

    setLang('en');
    expect(errorText('bookingNotThisCustomer', 'bookingNotThisCustomer')).toBe(
      "This passport is not the booked customer's. Please check the passport again.",
    );
    expect(errorText('bookingNotUsable', 'bookingNotUsable')).toBe(
      "This booking can't be used here. Please check the booking ticket again.",
    );
  });

  it('edge case: held-ticket read fails (network) -> skip is refused with the worded error, not silently allowed', async () => {
    const s = stubServer({ boardNetworkFailure: true });
    const store = atCustomerStep(s);

    clickSkip();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toContain(sharedTh['error.network']);
    });
    expect(s.reserves()).toHaveLength(0);
    expect(store.step()).toBe(1);

    setLang('en');
    clickSkip();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="alert"]')?.textContent).toContain(sharedEn['error.network']);
    });
    expect(s.reserves()).toHaveLength(0);
    expect(store.step()).toBe(1);
  });

  it('edge case: store.meId() empty (no read possible) -> behave as today (skip allowed)', async () => {
    const s = stubServer({ called: [HELD_BOOKING], bookings: [BK9] });
    const store = atCustomerStep(s, NO_ME);

    clickSkip();
    await vi.waitFor(() => expect(store.step()).toBe(2));
    expect(s.boardReads()).toHaveLength(0);
    expect(s.reserves()).toHaveLength(1);
    expect(s.reserves()[0]!.body.payload).toMatchObject({ passportNo: 'WALKIN' });
    expect(s.reserves()[0]!.body.payload).not.toHaveProperty('bookingId');
  });
});
