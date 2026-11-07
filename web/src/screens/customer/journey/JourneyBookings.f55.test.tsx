// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// Do not edit, delete or weaken these tests.
//
// A booking's start is an instant a person reads and the card gives no day, so it is shown with
// formatBangkokDateTime, not toLocaleString(locale()) in the device's time zone.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { makeMyBookingItemDto } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { JourneyBookings } from './JourneyBookings';

function bk(id: string, startAt: string) {
  return makeMyBookingItemDto({
    id, bookingNo: `NO-${id}`, status: 'CONFIRMED', branchId: 'bkk-1', slotId: 's', startAt, endAt: startAt, qrToken: null,
    quoteSummary: '', cancelCutoffAt: null, ticketNumber: null, ticketsAhead: null, estimateSeconds: null, estimateLowSeconds: null, estimateHighSeconds: null, estimateAsOf: null, readiness: 'unknown',
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;

async function times(): Promise<string[]> {
  // 06:23:45Z = 13:23 ICT on 04/11; 17:05Z on 04/11 = 00:05 ICT on 05/11.
  const list = [bk('a', '2099-11-04T06:23:45Z'), bk('b', '2099-11-04T17:05:00Z')];
  const fetchFn: Fetch = vi.fn(async () =>
    new Response(JSON.stringify({ version: 1, changed: true, data: { bookings: list } }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <JourneyBookings api={new ApiClient(fetchFn)} />, host);
  await flush();
  return Array.from(host.querySelectorAll('[data-testid="journey-booking"] .xc-booking-time')).map((e) => e.textContent ?? '');
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  setLang('th');
});

describe('F55 JourneyBookings start time', () => {
  it('Thai: each start reads dd/mm/yyyy HH:mm, CE year, Bangkok', async () => {
    setLang('th');
    expect(await times()).toEqual(['04/11/2099 13:23', '05/11/2099 00:05']);
  });

  it('English: the same text', async () => {
    setLang('en');
    expect(await times()).toEqual(['04/11/2099 13:23', '05/11/2099 00:05']);
  });
});
