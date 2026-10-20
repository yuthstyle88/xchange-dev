// #2475: opening another booking's cancel / reschedule dialog while the first booking's
// write is in flight must not drop that write's Idempotency-Key, so its Retry is a replay.
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

/** Fake server. Every booking write is held open until the test answers it. */
function makeServer() {
  const writes: WriteCall[] = [];
  const held: Deferred[] = [];

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/me') {
      return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: DESK_BOOKINGS_DATA });
    }

    if (method === 'POST' && /^\/api\/bookings\/[^/]+\/(check-in|cancel|reschedule)$/.test(u.pathname)) {
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

describe('DeskBookingsScreen.tsx — Retry after a dialog switch reuses the first booking\'s key (#2475)', () => {
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
  const closeBtn = (): HTMLButtonElement => {
    const b = Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-modal-actions button')).find(
      (x) => x.textContent?.trim() === 'Close',
    );
    expect(b, 'Close button').toBeDefined();
    return b!;
  };
  const reasonInput = (): HTMLInputElement | null => q<HTMLInputElement>('#cancel-reason-input');

  async function mount(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => {
      expect(q('[data-booking-id="bk-2"] button[data-action="cancel"]')).not.toBeNull();
    });
  }

  async function typeReason(text: string): Promise<void> {
    await vi.waitFor(() => expect(reasonInput()).not.toBeNull());
    const input = reasonInput()!;
    input.value = text;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  /** Confirm cancel on bk-1, close while pending, open Cancel on bk-2 and type its reason. */
  async function cancelAThenOpenB(
    server: ReturnType<typeof makeServer>,
    reasonA: string,
    reasonB: string,
  ): Promise<void> {
    btn('[data-booking-id="bk-1"] button[data-action="cancel"]').click();
    await typeReason(reasonA);
    btn('button[data-action="confirm-cancel"]').click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));
    expect(server.writes[0]!.path).toBe('/api/bookings/bk-1/cancel');

    closeBtn().click();
    await vi.waitFor(() => expect(reasonInput()).toBeNull());

    btn('[data-booking-id="bk-2"] button[data-action="cancel"]').click();
    await typeReason(reasonB);
  }

  /** Confirm reschedule on bk-1, close while pending, open Reschedule on bk-2. */
  async function rescheduleAThenOpenB(server: ReturnType<typeof makeServer>): Promise<void> {
    btn('[data-booking-id="bk-1"] button[data-action="reschedule"]').click();
    await vi.waitFor(() => expect(q('button[data-action="confirm-reschedule"]')).not.toBeNull());
    btn('button[data-action="confirm-reschedule"]').click();
    await vi.waitFor(() => expect(server.held).toHaveLength(1));
    expect(server.writes[0]!.path).toBe('/api/bookings/bk-1/reschedule');

    closeBtn().click();
    await vi.waitFor(() => expect(q('button[data-action="confirm-reschedule"]')).toBeNull());

    btn('[data-booking-id="bk-2"] button[data-action="reschedule"]').click();
    await vi.waitFor(() => expect(q('button[data-action="confirm-reschedule"]')).not.toBeNull());
  }

  it("row 1: cancel A, close while pending, open Cancel on B, A fails with a network error, Retry A — same Idempotency-Key as A's first POST", async () => {
    const server = makeServer();
    await mount();
    await cancelAThenOpenB(server, 'reason for A', 'reason for B');

    server.answer({ kind: 'network' });
    await vi.waitFor(() => expect(q('button[data-action="retry"]')).not.toBeNull());

    btn('button[data-action="retry"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(2));

    const [first, retry] = server.writes;
    expect(retry!.path).toBe('/api/bookings/bk-1/cancel');
    expect(retry!.body).toBe(first!.body);
    expect(first!.key).toBeTruthy();
    expect(retry!.key, "A's Retry replays A's key").toBe(first!.key);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } });
    await vi.waitFor(() => expect(q('[data-booking-id="bk-1"] .xc-status-CANCELLED')).not.toBeNull());
  });

  it("row 2: reschedule A, close while pending, open Reschedule on B, A fails with a network error, Retry A — same Idempotency-Key as A's first POST", async () => {
    const server = makeServer();
    await mount();
    await rescheduleAThenOpenB(server);

    server.answer({ kind: 'network' });
    await vi.waitFor(() => expect(q('button[data-action="retry"]')).not.toBeNull());

    btn('button[data-action="retry"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(2));

    const [first, retry] = server.writes;
    expect(retry!.path).toBe('/api/bookings/bk-1/reschedule');
    expect(retry!.body).toBe(first!.body);
    expect(first!.key).toBeTruthy();
    expect(retry!.key, "A's Retry replays A's key").toBe(first!.key);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CONFIRMED', slotId: 'slot-2' } });
    await flush();
  });

  it("edge: after A's success, confirming B (same reason text) sends B with its own key", async () => {
    const server = makeServer();
    await mount();
    await cancelAThenOpenB(server, 'same reason', 'same reason');

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } });
    await vi.waitFor(() => expect(q('[data-booking-id="bk-1"] .xc-status-CANCELLED')).not.toBeNull());
    await flush();

    btn('button[data-action="confirm-cancel"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(2));

    const [a, b] = server.writes;
    expect(b!.path).toBe('/api/bookings/bk-2/cancel');
    expect(b!.body).toBe(a!.body);
    expect(b!.key).toBeTruthy();
    expect(b!.key).not.toBe(a!.key);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-2', status: 'CANCELLED', bookingNo: 'B261015-0002' } });
    await flush();
  });

  it("edge: A fails, then B is confirmed instead of Retry (same reason text) — B does not reuse A's key", async () => {
    const server = makeServer();
    await mount();
    await cancelAThenOpenB(server, 'same reason', 'same reason');

    server.answer({ kind: 'network' });
    await vi.waitFor(() => expect(q('button[data-action="retry"]')).not.toBeNull());

    btn('button[data-action="confirm-cancel"]').click();
    await vi.waitFor(() => expect(server.writes).toHaveLength(2));

    const [a, b] = server.writes;
    expect(b!.path).toBe('/api/bookings/bk-2/cancel');
    expect(b!.body).toBe(a!.body);
    expect(b!.key).toBeTruthy();
    expect(b!.key).not.toBe(a!.key);

    server.answer({ kind: 'http', status: 200, body: { id: 'bk-2', status: 'CANCELLED', bookingNo: 'B261015-0002' } });
    await flush();
  });
});
