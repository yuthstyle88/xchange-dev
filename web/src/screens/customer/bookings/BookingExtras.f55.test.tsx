// F55 (docs/design/backdated-keying.md §16.7), written by the ticket author claude/3b81c9d5. Do not edit, delete or
// weaken these tests.
//
// Owner ruling 2026-11-07: customer booking times use the branch's own time zone, in the dd/mm/yyyy HH:mm shape with a
// CE year. The reschedule deadline is therefore formatDateTimeInZone(cancelCutoffAt, timeZone): 04/11/2099 13:23 for a
// Bangkok branch, 04/11/2099 15:23 for a Tokyo one. It is never toLocaleString(locale(), { dateStyle: 'medium' })
// (`4 พ.ย. 2642 13:23`).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { makeMyBookingItemDto } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { BookingExtras } from './BookingExtras';

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;

function deadlineText(timeZone = 'Asia/Bangkok'): string {
  const fetchFn: Fetch = vi.fn(async () => new Response('{}', { status: 404 }));
  const b = makeMyBookingItemDto({
    id: 'bk-f55',
    bookingNo: 'B991104-0001',
    status: 'CONFIRMED',
    branchId: 'bkk-1',
    slotId: 'slot-1',
    startAt: '2099-11-04T08:00:00Z',
    endAt: '2099-11-04T08:30:00Z',
    qrToken: 'QR',
    quoteSummary: '',
    // 06:23:45Z = 13:23 ICT on 04/11/2099.
    cancelCutoffAt: '2099-11-04T06:23:45Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  });
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <BookingExtras booking={b} api={new ApiClient(fetchFn)} timeZone={timeZone} onChange={vi.fn()} />,
    host,
  );
  return host.querySelector('[data-testid="cancel-deadline"]')?.textContent ?? '';
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  setLang('th');
});

describe('F55 BookingExtras reschedule deadline', () => {
  it('Thai: the deadline reads dd/mm/yyyy HH:mm, CE year, Bangkok', () => {
    setLang('th');
    expect(deadlineText()).toBe('เลื่อนนัดได้ถึง 04/11/2099 13:23');
  });

  it('English: the same date text', () => {
    setLang('en');
    expect(deadlineText()).toBe('Reschedule by 04/11/2099 13:23');
  });

  it("a Tokyo branch: the deadline is in the branch's own zone (owner ruling)", () => {
    setLang('th');
    expect(deadlineText('Asia/Tokyo')).toBe('เลื่อนนัดได้ถึง 04/11/2099 15:23');
  });
});
