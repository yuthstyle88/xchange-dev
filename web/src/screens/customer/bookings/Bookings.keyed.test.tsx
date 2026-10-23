// #2023 KW-4 Phase 1: BookingsScreen idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import {
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';

interface WriteCall {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepAnswer =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const BOOKINGS_DATA = makeMyBookingsData({
  bookings: [
    makeMyBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261015-0001',
      branchId: 'bkk-1',
      slotId: 'slot-1',
      startAt: '2026-10-15T10:00:00Z',
      endAt: '2026-10-15T10:30:00Z',
      status: 'CONFIRMED',
      cancelCutoffAt: '2026-10-15T09:00:00Z',
      qrToken: 'QR-BK-1',
      ticketNumber: null,
      ticketsAhead: null,
      estimateSeconds: null,
      estimateLowSeconds: null,
      estimateHighSeconds: null,
      estimateAsOf: null,
      quoteSummary: '',
      readiness: 'unknown',
    }),
  ],
});

const SLOTS_DATA = makeBookingSlotsData({
  asOf: '2026-10-15T08:00:00Z',
  slots: [
    makeBookingSlotItemDto({
      id: 'slot-1',
      start: '2026-10-15T10:00:00Z',
      end: '2026-10-15T10:30:00Z',
      available: 3,
    }),
    makeBookingSlotItemDto({
      id: 'slot-2',
      start: '2026-10-15T11:00:00Z',
      end: '2026-10-15T11:30:00Z',
      available: 4,
    }),
    makeBookingSlotItemDto({
      id: 'slot-3',
      start: '2026-10-15T13:00:00Z',
      end: '2026-10-15T13:30:00Z',
      available: 5,
    }),
  ],
});

function makeHarness(steps: StepAnswer[]) {
  const writes: WriteCall[] = [];
  let stepIdx = 0;

  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');

    if (method === 'GET' && u.pathname === '/api/public/host') {
      return jsonResponse(200, { org: 'XC' });
    }
    if (method === 'GET' && u.pathname === '/api/public/orgs/XC/branches') {
      return jsonResponse(200, {
        branches: [
          {
            id: 'bkk-1',
            code: 'bkk-1',
            name: 'bkk-1',
            address: null,
            openingHours: null,
            timezone: 'Asia/Bangkok',
            bookingEnabled: true,
            queueOpen: true,
          },
        ],
      });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/my_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: BOOKINGS_DATA });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/booking_slots') {
      return jsonResponse(200, { version: 1, changed: true, data: SLOTS_DATA });
    }

    if (method === 'POST' && u.pathname === '/api/bookings/bk-1/reschedule') {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({
        method,
        path: u.pathname,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = steps[stepIdx++] ?? {
        kind: 'http',
        status: 200,
        body: { id: 'bk-1', slotId: 'slot-2', status: 'CONFIRMED', bookingNo: 'B261015-0001' },
      };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);

  return {
    api: new ApiClient(fetchFn),
    writes,
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('Bookings.tsx — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date('2026-10-15T08:00:00Z'));
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function mountAndOpenReschedule(api: ApiClient): Promise<void> {
    dispose = render(() => <BookingsScreen client={api} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-1"] button[data-action="reschedule"]')).not.toBeNull();
    });
    const rescheduleBtn = host.querySelector(
      '[data-booking-id="bk-1"] button[data-action="reschedule"]',
    ) as HTMLButtonElement;
    rescheduleBtn.click();
    await vi.waitFor(() => {
      expect(host.querySelector('.xc-slot-option[data-slot-id="slot-2"] input[type="radio"]')).not.toBeNull();
    });
  }

  function selectSlot(slotId: string): void {
    const radio = host.querySelector(`.xc-slot-option[data-slot-id="${slotId}"] input[type="radio"]`) as HTMLInputElement;
    expect(radio).not.toBeNull();
    radio.click();
    radio.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function clickConfirmReschedule(): void {
    const btn = host.querySelector('button[data-action="confirm-reschedule"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    btn.click();
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { id: 'bk-1', slotId: 'slot-2', status: 'CONFIRMED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenReschedule(api);
    selectSlot('slot-2');

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: { id: 'bk-1', slotId: 'slot-2', status: 'CONFIRMED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenReschedule(api);
    selectSlot('slot-2');

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 409, body: { error: { code: 'bookingSlotFull', message: 'Slot is full' } } },
      { kind: 'http', status: 200, body: { id: 'bk-1', slotId: 'slot-2', status: 'CONFIRMED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenReschedule(api);
    selectSlot('slot-2');

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { id: 'bk-1', slotId: 'slot-3', status: 'CONFIRMED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenReschedule(api);
    selectSlot('slot-2');

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    selectSlot('slot-3');
    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 200, body: { id: 'bk-1', slotId: 'slot-2', status: 'CONFIRMED', bookingNo: 'B261015-0001' } },
      { kind: 'http', status: 200, body: { id: 'bk-1', slotId: 'slot-2', status: 'CONFIRMED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenReschedule(api);
    selectSlot('slot-2');

    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await vi.waitFor(() => {
      expect(host.querySelector('.xc-reschedule-dialog')).toBeNull();
    });

    const rescheduleBtn = host.querySelector(
      '[data-booking-id="bk-1"] button[data-action="reschedule"]',
    ) as HTMLButtonElement;
    rescheduleBtn.click();
    await vi.waitFor(() => {
      expect(host.querySelector('.xc-slot-option[data-slot-id="slot-2"] input[type="radio"]')).not.toBeNull();
    });
    selectSlot('slot-2');
    clickConfirmReschedule();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
