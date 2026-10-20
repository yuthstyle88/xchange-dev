// #2411: DeskBookingsScreen must not send a second write while the first is in flight.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
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

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'XC-B01',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const DESK_BOOKINGS_DATA = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
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
    }),
    makeDeskBookingItemDto({
      id: 'bk-2',
      bookingNo: 'B261015-0002',
      status: 'CONFIRMED',
      customerDisplayName: 'Malee Dee',
      phoneLast4: '1234',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: 'slot-2',
      startAt: '2026-10-15T10:00:00Z',
      endAt: '2026-10-15T10:30:00Z',
    }),
  ],
});

/**
 * Fake server. Every booking write is held open until the test resolves it.
 * Like api/src/infrastructure/idempotency/pg_store.rs, a write whose
 * Idempotency-Key is still PENDING is answered at once with 409 idempotencyConflict.
 */
function makeServer() {
  const writes: WriteCall[] = [];
  const held: Deferred[] = [];
  const pendingKeys = new Set<string>();
  let bookingReads = 0;

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/me') {
      return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      bookingReads += 1;
      return jsonResponse(200, { version: 1, changed: true, data: DESK_BOOKINGS_DATA });
    }

    if (method === 'POST' && /^\/api\/bookings\/[^/]+\/(check-in|cancel|reschedule)$/.test(u.pathname)) {
      const key = new Headers(init?.headers).get('idempotency-key');
      writes.push({ path: u.pathname, key, body: typeof init?.body === 'string' ? init.body : '' });
      if (key && pendingKeys.has(key)) {
        return jsonResponse(409, { error: { code: 'idempotencyConflict', message: 'Request in progress' } });
      }
      if (key) pendingKeys.add(key);
      const answer = await new Promise<Answer>((resolve) => held.push({ resolve }));
      if (key) pendingKeys.delete(key);
      if (answer.kind === 'network') throw new TypeError('Failed to fetch');
      return jsonResponse(answer.status, answer.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);

  return {
    writes,
    held,
    reads: () => bookingReads,
    /** Answer the oldest held write. */
    answer(a: Answer) {
      const d = held.shift();
      if (!d) throw new Error('no write is in flight');
      d.resolve(a);
    },
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen.tsx — writes are guarded while in flight (#2411)', () => {
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

  const q = <T extends Element>(sel: string): T | null => host.querySelector<T>(sel);
  const btn = (sel: string): HTMLButtonElement => {
    const b = q<HTMLButtonElement>(sel);
    expect(b, sel).not.toBeNull();
    return b!;
  };
  const banner = (): Element | null => q('.xc-error-banner');

  async function mount(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => {
      expect(q('[data-booking-id="bk-1"] button[data-action="check-in"]')).not.toBeNull();
    });
  }

  it('row 1: double click Check in sends one POST /api/bookings/{id}/check-in', async () => {
    const server = makeServer();
    await mount();

    const checkIn = btn('[data-booking-id="bk-1"] button[data-action="check-in"]');
    checkIn.click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(1));
    checkIn.click();
    await flush();

    expect(server.writes.filter((w) => w.path === '/api/bookings/bk-1/check-in')).toHaveLength(1);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CHECKED_IN', ticketNumber: 'A001' } });
    await vi.waitFor(() => expect(q('[data-booking-id="bk-1"] .xc-ticket-number')?.textContent).toBe('A001'));
    expect(server.writes).toHaveLength(1);
    expect(banner()).toBeNull();
  });

  it('row 2: clicking Confirm cancel while the cancel is in flight shows no error banner', async () => {
    const server = makeServer();
    await mount();

    btn('[data-booking-id="bk-1"] button[data-action="cancel"]').click();
    await vi.waitFor(() => expect(q('#cancel-reason-input')).not.toBeNull());
    const input = q<HTMLInputElement>('#cancel-reason-input')!;
    input.value = 'customer asked';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const confirm = btn('button[data-action="confirm-cancel"]');
    confirm.click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(1));
    confirm.click();
    await flush();

    expect(banner()).toBeNull();
    expect(server.writes).toHaveLength(1);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } });
    await vi.waitFor(() => expect(q('#cancel-reason-input')).toBeNull());
    expect(banner()).toBeNull();
    expect(q('[data-booking-id="bk-1"] .xc-status-CANCELLED')).not.toBeNull();
  });

  it('row 3: network error on reschedule (after a click while in flight), then retry, keeps the same key', async () => {
    const server = makeServer();
    await mount();

    btn('[data-booking-id="bk-1"] button[data-action="reschedule"]').click();
    await vi.waitFor(() => expect(q('button[data-action="confirm-reschedule"]')).not.toBeNull());

    const confirm = btn('button[data-action="confirm-reschedule"]');
    confirm.click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(1));
    confirm.click();
    await flush();

    server.answer({ kind: 'network' });
    await vi.waitFor(() => expect(q('button[data-action="retry"]')).not.toBeNull());

    btn('button[data-action="retry"]').click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));

    const sent = server.writes.filter((w) => w.path === '/api/bookings/bk-1/reschedule');
    expect(sent[0]!.key).toBeTruthy();
    expect(sent.at(-1)!.key).toBe(sent[0]!.key);
    expect(sent.at(-1)!.body).toBe(sent[0]!.body);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CONFIRMED', slotId: 'slot-2' } });
    await vi.waitFor(() => expect(q('button[data-action="confirm-reschedule"]')).toBeNull());
    expect(banner()).toBeNull();
  });

  it('edge: Refresh stays enabled and still reads while a check-in is in flight; the write buttons are disabled', async () => {
    const server = makeServer();
    await mount();

    btn('[data-booking-id="bk-1"] button[data-action="check-in"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(1));

    expect(btn('[data-booking-id="bk-1"] button[data-action="check-in"]').disabled).toBe(true);
    expect(btn('[data-booking-id="bk-2"] button[data-action="check-in"]').disabled).toBe(true);

    const refresh = btn('button[data-action="refresh"]');
    expect(refresh.disabled).toBe(false);
    const before = server.reads();
    refresh.click();
    await vi.waitFor(() => expect(server.reads()).toBe(before + 1));

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CONFIRMED', ticketNumber: 'A001' } });
    await vi.waitFor(() =>
      expect(btn('[data-booking-id="bk-1"] button[data-action="check-in"]').disabled).toBe(false),
    );
  });

  it('edge: writing is cleared after a refusal so Confirm cancel works again', async () => {
    const server = makeServer();
    await mount();

    btn('[data-booking-id="bk-1"] button[data-action="cancel"]').click();
    await vi.waitFor(() => expect(q('#cancel-reason-input')).not.toBeNull());
    const input = q<HTMLInputElement>('#cancel-reason-input')!;
    input.value = 'customer asked';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    btn('button[data-action="confirm-cancel"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(1));
    expect(btn('button[data-action="confirm-cancel"]').disabled).toBe(true);

    server.answer({ kind: 'http', status: 400, body: { error: { code: 'validationFailed', message: 'Invalid reason' } } });
    await vi.waitFor(() => expect(banner()).not.toBeNull());
    expect(btn('button[data-action="confirm-cancel"]').disabled).toBe(false);

    btn('button[data-action="confirm-cancel"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(2));
    // A 4xx drops the key, so the next send carries a new one.
    expect(server.writes[1]!.key).not.toBe(server.writes[0]!.key);
    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } });
    await vi.waitFor(() => expect(q('#cancel-reason-input')).toBeNull());
  });
});
