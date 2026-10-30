// #5568 (DESK-R4 T3, docs/design/fe-desk-round4.md §3 T3 rows D7, D8): the new-booking dialog shows the BOOKING
// quote's THB amount ("ประมาณการยอดเงิน", desk check round 4 P4), and the dialogs' close buttons and the reschedule
// slot label read Thai. Written before the implementation by the ticket author claude/studio-claude-15.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingSlotItemDto,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
  makeQuoteDto,
  makeQuoteLineDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { en, th } from './i18n';

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
      bookingNo: 'B261031-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Somchai Jaidee',
      phoneLast4: '5678',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-31T03:00:00Z',
      endAt: '2026-10-31T03:30:00Z',
    }),
  ],
});

const QUOTE = makeQuoteDto({
  id: 'q-booking',
  channel: 'BOOKING',
  lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'SELL', quantity: '100', settleThb: '3367.37' })],
  benefits: [],
});

function stub(): void {
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
        const slots = [
          makeBookingSlotItemDto({ id: 'slot-1', start: '2026-10-31T03:00:00Z', end: '2026-10-31T03:30:00Z', available: 1 }),
          makeBookingSlotItemDto({ id: 'slot-2', start: '2026-10-31T04:00:00Z', end: '2026-10-31T04:30:00Z', available: 1 }),
        ];
        return json(200, { version: 1, changed: true, data: { asOf: '2026-10-30T20:00:00Z', slots } });
      }
      if (method === 'POST' && u.pathname === '/api/quotes') return json(200, QUOTE);
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
const closeButtons = () =>
  Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-modal-actions button.xc-btn-secondary')).map((b) =>
    b.textContent?.trim(),
  );

async function mount(): Promise<void> {
  dispose = render(() => <DeskBookingsScreen />, host);
  await vi.waitFor(() => expect(q('[data-booking-id="bk-1"]')).not.toBeNull());
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date('2026-10-30T20:00:00Z'));
  setAudience('staff');
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  stub();
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

describe('#5568 the BOOKING estimate shows its THB amount (P4)', () => {
  it('D7 a shown quote has a ประมาณการยอดเงิน row with each line settleThb, inside booking-estimate', async () => {
    await mount();
    q<HTMLButtonElement>('button[data-action="new-booking"]')!.click();
    await flush();
    const qty = q<HTMLInputElement>('input[name="quantity"]')!;
    qty.value = '100';
    qty.dispatchEvent(new Event('input', { bubbles: true }));
    qty.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(q('[data-row="booking-estimate"] [data-row="booking-estimate-thb"]')).not.toBeNull());
    const row = q('[data-row="booking-estimate"] [data-row="booking-estimate-thb"]')!;
    expect(row.textContent).toContain('ประมาณการยอดเงิน');
    expect(row.textContent).toContain('3,367.37 THB');
  });
});

describe('#5568 the dialogs read Thai (labels)', () => {
  it('D8 the i18n keys are Thai, and en carries the same Thai', () => {
    const t = th as Record<string, string>;
    const e = en as Record<string, string>;
    const want: Record<string, string> = {
      close: 'ปิด',
      'dialog.date': 'วันที่',
      'dialog.noSlots': 'ไม่มีช่วงเวลาว่าง',
      'dialog.slotsLoadFailed': 'โหลดช่วงเวลาไม่สำเร็จ',
      'dialog.estimatedThb': 'ประมาณการยอดเงิน',
    };
    for (const [k, v] of Object.entries(want)) {
      expect(t[k], `th.${k}`).toBe(v);
      expect(e[k], `en.${k}`).toBe(v);
    }
  });

  it('D8 the new-booking, reschedule and cancel dialogs close with ปิด; the reschedule slot label is ช่วงเวลา', async () => {
    await mount();
    q<HTMLButtonElement>('button[data-action="new-booking"]')!.click();
    await flush();
    expect(closeButtons()).toEqual(['ปิด']);
    q<HTMLButtonElement>('.xc-modal-actions button.xc-btn-secondary')!.click();
    await flush();

    q<HTMLButtonElement>('[data-booking-id="bk-1"] button[data-action="reschedule"]')!.click();
    await flush();
    expect(closeButtons()).toEqual(['ปิด']);
    expect(q('label[for="reschedule-slot-select"]')?.textContent?.trim()).toBe('ช่วงเวลา');
    q<HTMLButtonElement>('.xc-modal-actions button.xc-btn-secondary')!.click();
    await flush();

    q<HTMLButtonElement>('[data-booking-id="bk-1"] button[data-action="cancel"]')!.click();
    await flush();
    expect(closeButtons()).toEqual(['ปิด']);
    expect(host.textContent).not.toMatch(/\bClose\b/);
  });
});
