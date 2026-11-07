// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// The slot options of both dialogs read `dd/mm/yyyy HH:mm - HH:mm` in either language (never `th-TH` `2 พ.ย.`); the
// board's slot header stays time-only, since the board is today's list; the date inputs keep `YYYY-MM-DD`.
// Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingSlotItemDto,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
  makeQuoteDto,
  type BookingSlotItemDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'XC-B01',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261102-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Somchai Jaidee',
      phoneLast4: '5678',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: 'own-slot',
      startAt: '2026-11-02T03:00:00Z',
      endAt: '2026-11-02T03:30:00Z',
    }),
  ],
});

const slot = (id: string, start: string, available: number): BookingSlotItemDto =>
  makeBookingSlotItemDto({ id, start, end: start.replace(':00:00Z', ':30:00Z'), available });

function stubServer(slots: BookingSlotItemDto[]): void {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && u.pathname === '/api/v3/views/me') return json(200, { version: 1, changed: true, data: TELLER });
      if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
        return json(200, { version: 1, changed: true, data: BOOKINGS });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/booking_slots') {
        return json(200, { version: 1, changed: true, data: { asOf: '2026-11-01T20:00:00Z', slots } });
      }
      if (method === 'POST' && u.pathname === '/api/quotes') return json(200, makeQuoteDto({ id: 'q-1', channel: 'BOOKING' }));
      return json(404, { error: { code: 'notFound', message: 'Not found' } });
    }),
  );
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T>(sel);
const optionText = (sel: string, value: string) =>
  host.querySelector<HTMLOptionElement>(`${sel} option[value="${value}"]`)?.textContent?.trim();

async function mount(): Promise<void> {
  dispose = render(() => <DeskBookingsScreen />, host);
  await vi.waitFor(() => expect(q('[data-booking-id="bk-1"]')).not.toBeNull());
}

beforeEach(() => {
  // 2026-11-01 20:00 UTC is 2026-11-02 03:00 in Bangkok.
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date('2026-11-01T20:00:00Z'));
  setAudience('staff');
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

describe('F55 desk bookings: one date format', () => {
  it('F55-BK1 th: the reschedule option reads "02/11/2026 11:00 - 11:30"; the date input keeps 2026-11-02', async () => {
    stubServer([slot('s-b', '2026-11-02T04:00:00Z', 1)]);
    await mount();
    q<HTMLButtonElement>('[data-booking-id="bk-1"] button[data-action="reschedule"]')!.click();
    await flush();
    expect(q<HTMLInputElement>('#reschedule-date')?.value).toBe('2026-11-02');
    await vi.waitFor(() => expect(optionText('#reschedule-slot-select', 's-b')).toBe('02/11/2026 11:00 - 11:30'));
  });

  it('F55-BK2 en: the new-booking option reads "02/11/2026 11:00 - 11:30" too (no Thai month, no Buddhist year)', async () => {
    setLang('en');
    stubServer([slot('s-b', '2026-11-02T04:00:00Z', 1)]);
    await mount();
    q<HTMLButtonElement>('button[data-action="new-booking"]')!.click();
    await flush();
    expect(q<HTMLInputElement>('#new-booking-date')?.value).toBe('2026-11-02');
    await vi.waitFor(() => expect(optionText('#new-booking-slot', 's-b')).toBe('02/11/2026 11:00 - 11:30'));
  });

  it("F55-BK3 the board's slot header stays time-only 24h Bangkok: \"10:00 - 10:30\"", async () => {
    stubServer([]);
    await mount();
    const header = q('.xc-slot-group .xc-slot-header')?.textContent?.trim();
    expect(header).toBe('10:00 - 10:30');
  });
});
