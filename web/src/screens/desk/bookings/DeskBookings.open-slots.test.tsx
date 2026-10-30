// #5568 (DESK-R4 T3, docs/design/fe-desk-round4.md §3 T3 rows D1–D6, D9): the desk dialogs offer the branch's open
// slots from view `booking_slots`, not the slots of today's bookings (desk check round 4, P5).
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingSlotItemDto,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
  makeQuoteDto,
} from '../../../shared/testing/fixtures';
import type { BookingSlotItemDto } from '../../../shared/api/generated/booking_slots';
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

// Today's bookings sit in past slots (round 4: 21–26 Oct); the dialogs must not offer them.
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
      slotId: 'past-slot',
      startAt: '2026-11-02T03:00:00Z',
      endAt: '2026-11-02T03:30:00Z',
    }),
  ],
});

const slot = (id: string, start: string, available: number): BookingSlotItemDto =>
  makeBookingSlotItemDto({ id, start, end: start.replace(':00:00Z', ':30:00Z'), available });

interface SlotsRead {
  branch: string | null;
  date: string | null;
}

/** A fake server. `slots(branch, date)` answers `booking_slots`; return `'fail'` for a 500, or a Promise to hold it. */
function makeServer(slots: (branch: string | null, date: string | null) => BookingSlotItemDto[] | 'fail' | Promise<BookingSlotItemDto[]>) {
  const reads: SlotsRead[] = [];
  const bookingWrites: string[] = [];
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
        const branch = u.searchParams.get('branch');
        const date = u.searchParams.get('date');
        reads.push({ branch, date });
        const a = slots(branch, date);
        if (a === 'fail') return json(500, { error: { code: 'serverError', message: 'boom' } });
        const list = await a;
        return json(200, { version: 1, changed: true, data: { asOf: '2026-10-30T20:00:00Z', slots: list } });
      }
      if (method === 'POST' && u.pathname === '/api/quotes') return json(200, makeQuoteDto({ id: 'q-1', channel: 'BOOKING' }));
      if (method === 'POST' && u.pathname.startsWith('/api/bookings')) {
        bookingWrites.push(u.pathname);
        return json(200, {});
      }
      return json(404, { error: { code: 'notFound', message: 'Not found' } });
    }),
  );
  return { reads, bookingWrites };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

function setValue(el: HTMLInputElement | HTMLSelectElement, v: string): void {
  el.value = v;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T>(sel);
const options = (sel: string) => Array.from(host.querySelectorAll<HTMLOptionElement>(`${sel} option`));
const enabledValues = (sel: string) => options(sel).filter((o) => !o.disabled).map((o) => o.value);

async function mount(): Promise<void> {
  dispose = render(() => <DeskBookingsScreen />, host);
  await vi.waitFor(() => expect(q('[data-booking-id="bk-1"]')).not.toBeNull());
}

async function openNewBooking(): Promise<void> {
  q<HTMLButtonElement>('button[data-action="new-booking"]')!.click();
  await flush();
}

beforeEach(() => {
  // 2026-10-30 20:00 UTC is 2026-10-31 03:00 in Bangkok: "today" must be Bangkok's day, not UTC's.
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date('2026-10-30T20:00:00Z'));
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

describe('#5568 desk dialogs offer open slots from booking_slots (P5)', () => {
  it("D1 opening the new-booking dialog sets the date to Bangkok's today and reads that day's slots for the branch", async () => {
    const server = makeServer(() => [slot('s-a', '2026-10-31T03:00:00Z', 2)]);
    await mount();
    await openNewBooking();
    const date = q<HTMLInputElement>('#new-booking-date');
    expect(date, '#new-booking-date').not.toBeNull();
    expect(date!.type).toBe('date');
    expect(date!.name).toBe('serviceDate');
    expect(date!.value).toBe('2026-10-31');
    expect(q('label[for="new-booking-date"]')?.textContent?.trim()).toBe('วันที่');
    expect(server.reads).toContainEqual({ branch: 'XC-B01', date: '2026-10-31' });
  });

  it('D2 the select offers exactly the answered slots with available > 0, in answer order; the first is selected', async () => {
    makeServer(() => [
      slot('s-a', '2026-10-31T03:00:00Z', 2),
      slot('s-full', '2026-10-31T04:00:00Z', 0),
      slot('s-b', '2026-10-31T05:00:00Z', 1),
    ]);
    await mount();
    await openNewBooking();
    await vi.waitFor(() => expect(enabledValues('#new-booking-slot')).toEqual(['s-a', 's-b']));
    expect(options('#new-booking-slot').map((o) => o.value)).not.toContain('past-slot');
    expect(q<HTMLSelectElement>('#new-booking-slot')!.value).toBe('s-a');
    expect(options('#new-booking-slot')[0]!.textContent?.trim()).toBe('31 ต.ค. 10:00 - 10:30');
  });

  it('D3 a new date reads again; an older answer that arrives last is dropped', async () => {
    let releaseFirst!: (l: BookingSlotItemDto[]) => void;
    const server = makeServer((_b, date) => {
      if (date === '2026-11-01') return new Promise((r) => (releaseFirst = r));
      if (date === '2026-11-03') return [slot('s-nov3', '2026-11-03T03:00:00Z', 1)];
      return [slot('s-a', '2026-10-31T03:00:00Z', 1)];
    });
    await mount();
    await openNewBooking();
    await vi.waitFor(() => expect(enabledValues('#new-booking-slot')).toEqual(['s-a']));

    setValue(q<HTMLInputElement>('#new-booking-date')!, '2026-11-01');
    await flush();
    setValue(q<HTMLInputElement>('#new-booking-date')!, '2026-11-03');
    await vi.waitFor(() => expect(enabledValues('#new-booking-slot')).toEqual(['s-nov3']));
    releaseFirst([slot('s-nov1', '2026-11-01T03:00:00Z', 1)]);
    await flush();

    expect(server.reads.map((r) => r.date)).toEqual(['2026-10-31', '2026-11-01', '2026-11-03']);
    expect(enabledValues('#new-booking-slot')).toEqual(['s-nov3']);
    expect(q<HTMLSelectElement>('#new-booking-slot')!.value).toBe('s-nov3');
  });

  it('D3b a new branch reads again for that branch', async () => {
    const server = makeServer((branch) =>
      branch === 'XC-B02' ? [slot('s-b02', '2026-10-31T06:00:00Z', 1)] : [slot('s-a', '2026-10-31T03:00:00Z', 1)],
    );
    await mount();
    await openNewBooking();
    await vi.waitFor(() => expect(enabledValues('#new-booking-slot')).toEqual(['s-a']));
    setValue(q<HTMLInputElement>('#new-booking-branch')!, 'XC-B02');
    await vi.waitFor(() => expect(enabledValues('#new-booking-slot')).toEqual(['s-b02']));
    expect(server.reads).toContainEqual({ branch: 'XC-B02', date: '2026-10-31' });
  });

  it('D4 with no slot available, one disabled option ไม่มีช่วงเวลาว่าง, and submit books nothing', async () => {
    const server = makeServer(() => [slot('s-full', '2026-10-31T04:00:00Z', 0)]);
    await mount();
    await openNewBooking();
    await vi.waitFor(() => expect(options('#new-booking-slot')).toHaveLength(1));
    const only = options('#new-booking-slot')[0]!;
    expect(only.disabled).toBe(true);
    expect(only.textContent?.trim()).toBe('ไม่มีช่วงเวลาว่าง');

    setValue(q<HTMLInputElement>('input[name="quantity"]')!, '100');
    setValue(q<HTMLInputElement>('input[name="displayName"]')!, 'Somchai');
    setValue(q<HTMLInputElement>('input[name="phone"]')!, '0812345678');
    q<HTMLButtonElement>('button[data-action="submit-booking"]')!.click();
    await flush();
    expect(server.bookingWrites).toEqual([]);
  });

  it('D5 a failed slots read shows โหลดช่วงเวลาไม่สำเร็จ at the slot field and offers no slot', async () => {
    makeServer(() => 'fail');
    await mount();
    await openNewBooking();
    await vi.waitFor(() => expect(q('[data-row="slots-error"]')?.textContent?.trim()).toBe('โหลดช่วงเวลาไม่สำเร็จ'));
    expect(enabledValues('#new-booking-slot')).toEqual([]);
  });

  it("D6 reschedule reads the booking's own Bangkok day and offers its open slots except the booking's own", async () => {
    const server = makeServer((_b, date) =>
      date === '2026-11-02'
        ? [
            slot('past-slot', '2026-11-02T03:00:00Z', 3),
            slot('s-nov2-b', '2026-11-02T04:00:00Z', 1),
            slot('s-nov2-full', '2026-11-02T05:00:00Z', 0),
          ]
        : [],
    );
    await mount();
    q<HTMLButtonElement>('[data-booking-id="bk-1"] button[data-action="reschedule"]')!.click();
    await flush();
    expect(q<HTMLInputElement>('#reschedule-date')?.value).toBe('2026-11-02');
    await vi.waitFor(() => expect(enabledValues('#reschedule-slot-select')).toEqual(['s-nov2-b']));
    expect(q<HTMLSelectElement>('#reschedule-slot-select')!.value).toBe('s-nov2-b');
    expect(server.reads).toContainEqual({ branch: 'XC-B01', date: '2026-11-02' });
  });

  it("D9 the board still groups today's bookings by their slot", async () => {
    makeServer(() => []);
    await mount();
    expect(q('.xc-slot-group [data-booking-id="bk-1"]')).not.toBeNull();
  });
});
