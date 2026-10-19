// C09 up-front tests (feat/fe-customer skeleton): JourneyBookings lists the customer's upcoming bookings
// with their check-in QR, from my_bookings. The implementer makes these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import type { MyBookingItemDto } from '../../../shared/api/generated/my_bookings';
import { setLang } from '../../../shared/i18n';
import { JourneyBookings } from './JourneyBookings';

type Reply = { status: number; body: unknown } | 'network';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function bk(id: string, status: string, startAt: string, qrToken: string | null): MyBookingItemDto {
  return {
    id, bookingNo: `NO-${id}`, status, branchId: 'bkk-1', slotId: 's', startAt, endAt: startAt, qrToken,
    quoteSummary: '', cancelCutoffAt: null, ticketNumber: null, ticketsAhead: null, estimateSeconds: null, estimateLowSeconds: null, estimateHighSeconds: null, estimateAsOf: null, readiness: 'unknown',
  };
}

const LIST = [
  bk('late', 'CONFIRMED', '2099-10-20T10:00:00Z', 'QR-LATE'),
  bk('done', 'COMPLETED', '2099-10-01T10:00:00Z', 'QR-DONE'),
  bk('early', 'PENDING_CUSTOMER_CONFIRMATION', '2099-10-10T10:00:00Z', null),
  bk('gone', 'CANCELLED', '2099-10-05T10:00:00Z', 'QR-GONE'),
  bk('in', 'CHECKED_IN', '2099-10-15T10:00:00Z', 'QR-IN'),
];

function server(reply?: Reply) {
  let reads = 0;
  const fetchFn: Fetch = vi.fn(async (input: string) => {
    const path = new URL(input, 'http://localhost').pathname;
    if (path !== '/api/v3/views/my_bookings') return json(404, { error: { code: 'notFound', message: 'Not found' } });
    reads += 1;
    const r = reply ?? { status: 200, body: { version: 1, changed: true, data: { bookings: LIST } } };
    if (r === 'network') throw new TypeError('Failed to fetch');
    return json(r.status, r.body);
  });
  return { api: new ApiClient(fetchFn), reads: () => reads };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
function mount(api: ApiClient): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <JourneyBookings api={api} />, host);
}
const cards = () => Array.from(host.querySelectorAll<HTMLElement>('[data-testid="journey-booking"]'));

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('JourneyBookings (C09)', () => {
  it('reads my_bookings once and lists only open bookings, soonest first', async () => {
    const s = server();
    mount(s.api);
    await flush();
    expect(s.reads()).toBe(1);
    expect(cards().map((c) => c.dataset.bookingId)).toEqual(['early', 'in', 'late']);
  });

  it('each card shows its booking number, and a QR when it has a token', async () => {
    mount(server().api);
    await flush();
    const [early, checkedIn] = cards();
    expect(early.textContent).toContain('NO-early');
    expect(early.querySelector('[data-qr-token]')).toBeNull();
    expect(checkedIn.querySelector('[data-qr-token]')?.getAttribute('data-qr-token')).toBe('QR-IN');
  });

  it('no open booking shows the empty line', async () => {
    mount(server({ status: 200, body: { version: 1, changed: true, data: { bookings: [LIST[1], LIST[3]] } } }).api);
    await flush();
    expect(cards()).toHaveLength(0);
    expect(host.querySelector('[data-testid="journey-bookings-empty"]')).not.toBeNull();
  });

  it('a failed read shows the error line', async () => {
    mount(server('network').api);
    await flush();
    expect(host.querySelector('[data-testid="journey-bookings-error"]')?.textContent?.trim()).toBeTruthy();
  });

  it('a 401 shows nothing: the queue section above already asks the customer to sign in', async () => {
    mount(server({ status: 401, body: { error: { code: 'unauthenticated', message: 'no' } } }).api);
    await flush();
    expect(host.textContent?.trim()).toBe('');
  });
});
