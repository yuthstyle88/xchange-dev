// #5799 FE-W4a up-front tests (docs/design/walkin-queue.md §12.4 rows 1-6; ruling A on #564, issuecomment-6541732560).
// Written before the implementation by the ticket author claude/01TxaRJG. Do not edit, delete or weaken; add tests in a
// file of your own. step.walkin.test.tsx stubs no board read, so its rows exercise W4a.5 (read fails -> serve as today).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { makeDeskConfig, makeQueueTicketDto } from '../../../../shared/testing/fixtures';
import { OWNER_DESK } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import { step as bookingStep } from './step';
import * as texts from './i18n';

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

type Call = { method: string; path: string; key: string | null; body: unknown };
type Answer = { status: number; body?: unknown };

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

function stubServer(opts: { called?: CalledRow[]; boardStatus?: number; bookings?: BookingItem[] }) {
  const calls: Call[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const headers = new Headers(init?.headers);
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, key: headers.get('idempotency-key'), body });
    let a: Answer;
    if (method === 'GET' && path === '/api/v3/views/desk_config') {
      a = { status: 200, body: { version: 1, changed: true, data: makeDeskConfig({ shiftOpen: true }) } };
    } else if (method === 'GET' && path === '/api/v3/views/queue_board') {
      const status = opts.boardStatus ?? 200;
      a =
        status === 200
          ? {
              status,
              body: {
                version: 1,
                changed: true,
                data: {
                  waiting: [],
                  called: (opts.called ?? []).map((c) => ({ ...c, tellerName: null, calledAt: null })),
                  myCounter: { status: 'ACTIVE' },
                },
              },
            }
          : { status, body: { error: { code: 'serverError', message: 'serverError' } } };
    } else if (method === 'GET' && path === '/api/v3/views/desk_bookings') {
      a = {
        status: 200,
        body: {
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
        },
      };
    } else if (method === 'POST' && path === '/api/queue/tickets/serve') {
      a = { status: 200, body: makeQueueTicketDto({ id: 't-served', displayNumber: 'W020', status: 'SERVING' }) };
    } else {
      a = { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    }
    return new Response(JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': 'application/json' },
    });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    serveCalls: () => calls.filter((c) => c.method === 'POST' && c.path === '/api/queue/tickets/serve'),
  };
}

let disposers: (() => void)[] = [];
let host: HTMLDivElement;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal('confirm', vi.fn(() => true));
});

afterEach(() => {
  for (const d of disposers) d();
  disposers = [];
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function readyStore(s: ReturnType<typeof stubServer>): DeskStore {
  let store!: DeskStore;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path: OWNER_DESK, me: ME as never });
  });
  store.setField('name', 'JOHN DOE', 'user');
  store.setValueDate('2026-10-31');
  store.go(2);
  disposers.push(render(() => bookingStep.component({ store }), host));
  return store;
}

const submitBtn = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
const heldRow = () => host.querySelector('[data-row="held-booking"]');
const alertEl = () => host.querySelector('[role="alert"]');

describe('steps/booking W4a: the teller holds a BOOKING ticket (walkin-queue.md §12)', () => {
  it('row 1: held BOOKING B003 matched to booking bk-9 -> no serve, heldBooking set, row shows B003, step 3', async () => {
    const s = stubServer({
      called: [{ id: 't-b', displayNumber: 'B003', serviceType: 'BOOKING', status: 'SERVING', tellerId: 'u-1' }],
      bookings: [{ id: 'bk-9', bookingNo: 'BK-0009', status: 'IN_SERVICE', ticketNumber: 'B003' }],
    });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));

    expect(s.serveCalls(), 'no POST /api/queue/tickets/serve for a held BOOKING ticket (W4a.2)').toHaveLength(0);
    expect(store.heldBooking()).toEqual({ bookingId: 'bk-9', ticketId: 't-b', displayNumber: 'B003' });
    expect(store.walkInTicket()).toBeUndefined();

    store.go(2);
    await vi.waitFor(() => expect(heldRow()).not.toBeNull());
    expect(heldRow()!.textContent).toContain('B003');
  });

  it('row 1b: a CALLED (not yet started) BOOKING ticket counts as held too', async () => {
    const s = stubServer({
      called: [{ id: 't-b', displayNumber: 'B003', serviceType: 'BOOKING', status: 'CALLED', tellerId: 'u-1' }],
      bookings: [{ id: 'bk-9', bookingNo: 'BK-0009', status: 'CHECKED_IN', ticketNumber: 'B003' }],
    });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(s.serveCalls()).toHaveLength(0);
    expect(store.heldBooking()?.bookingId).toBe('bk-9');
  });

  it('row 2: held WALK_IN ticket -> serve once, as today; no heldBooking', async () => {
    const s = stubServer({
      called: [{ id: 't-w', displayNumber: 'W010', serviceType: 'WALK_IN', status: 'CALLED', tellerId: 'u-1' }],
    });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(s.serveCalls()).toHaveLength(1);
    expect(store.walkInTicket()?.displayNumber).toBe('W020');
    expect(store.heldBooking()).toBeUndefined();
  });

  it('row 3: nothing held -> serve once, as today', async () => {
    const s = stubServer({ called: [] });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(s.serveCalls()).toHaveLength(1);
    expect(store.heldBooking()).toBeUndefined();
  });

  it('row 3b: a BOOKING ticket held by another teller is ignored -> serve once', async () => {
    const s = stubServer({
      called: [{ id: 't-x', displayNumber: 'B007', serviceType: 'BOOKING', status: 'SERVING', tellerId: 'u-2' }],
      bookings: [{ id: 'bk-7', bookingNo: 'BK-0007', status: 'IN_SERVICE', ticketNumber: 'B007' }],
    });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(s.serveCalls()).toHaveLength(1);
    expect(store.heldBooking()).toBeUndefined();
  });

  it('row 4: the board read fails -> serve once, as today (W4a.5)', async () => {
    const s = stubServer({ boardStatus: 500 });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(s.serveCalls()).toHaveLength(1);
    expect(store.heldBooking()).toBeUndefined();
  });

  it('row 5: held BOOKING B004 with no matching booking -> worded refusal, no serve, stays on step 2 (W4a.6)', async () => {
    const s = stubServer({
      called: [{ id: 't-b4', displayNumber: 'B004', serviceType: 'BOOKING', status: 'SERVING', tellerId: 'u-1' }],
      bookings: [{ id: 'bk-1', bookingNo: 'BK-0001', status: 'COMPLETED', ticketNumber: 'B004' }],
    });
    const store = readyStore(s);
    const th = texts.th as Record<string, string>;

    expect(th['error.heldBookingUnmatched'], 'i18n key error.heldBookingUnmatched (th)').toBeTruthy();
    submitBtn().click();
    await vi.waitFor(() => expect(alertEl()).not.toBeNull());
    expect(alertEl()!.textContent).toContain('B004');
    expect(s.serveCalls()).toHaveLength(0);
    expect(store.step()).toBe(2);
    expect(store.heldBooking()).toBeUndefined();
  });

  it('row 6: startNew() clears heldBooking', async () => {
    const s = stubServer({
      called: [{ id: 't-b', displayNumber: 'B003', serviceType: 'BOOKING', status: 'SERVING', tellerId: 'u-1' }],
      bookings: [{ id: 'bk-9', bookingNo: 'BK-0009', status: 'IN_SERVICE', ticketNumber: 'B003' }],
    });
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.heldBooking()?.bookingId).toBe('bk-9'));
    store.startNew();
    expect(store.heldBooking()).toBeUndefined();
  });
});
