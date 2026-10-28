// XC-BOOKING-ESTIMATE FE-2: C03 /app/bookings shows a booking's indicative benefit when the customer opens it.
// Design docs/design/blueprint/marketing-campaigns.md §6.4.3, E-9, E-10, §6.4.5 rows 23 and 24, §6.4.6 FE-2.
// Written before the implementation by the FE lane controller claude/e92db6. Do not edit, delete or weaken these
// tests; add your own elsewhere.
//
// [required] by these tests:
// - Each CONFIRMED booking card has `button[data-action="booking-estimate"]`. Pressing it sends exactly one
//   POST /api/v3/compute/planner_preview with `{ branch: <booking.branchId>, currency, customerSide,
//   amount: { quantity } }` from the booking item, and renders the shared `IndicativeBenefit`
//   (`screens/customer/benefit-estimate/`) inside that card with `slotStartAt = booking.startAt`.
// - While an estimate is open, the screen polls the booking branch's public rates route
//   (`/api/public/orgs/<org>/branches/<code>/rates`) with `If-None-Match`, every `ratesIntervalMs` (a new
//   `BookingsScreenProps` field). A 304 changes nothing. A 200 with a new ETag sends one more planner_preview for that
//   booking. Nothing else re-sends it on a timer.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeMyBookingItemDto,
  makeMyBookingsData,
  makePlannerBenefitItemDto,
  makePlannerPreviewAnswer,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';

const BOOKING = makeMyBookingItemDto({
  id: 'bk-1',
  bookingNo: 'B261101-0001',
  branchId: 'bkk-1',
  slotId: 'slot-1',
  startAt: '2026-11-05T03:00:00Z',
  endAt: '2026-11-05T03:30:00Z',
  status: 'CONFIRMED',
  currency: 'USD',
  customerSide: 'CUSTOMER_SELL',
  quantity: '100',
  ticketNumber: null,
  quoteSummary: '',
});

const ESTIMATED = (validUntil: string) =>
  makePlannerPreviewAnswer({
    thb: '3542.00',
    rate: '35.42000000',
    unitSize: '1',
    benefit: {
      state: 'ESTIMATED',
      basis: 'INDICATIVE',
      channel: 'APP',
      evaluatedAt: '2026-11-01T00:00:00Z',
      validUntil,
      publicationId: 'pub-1',
      finalRate: '35.52000000',
      thbAfter: '3552.00',
      benefitThb: '10.00',
      items: [makePlannerBenefitItemDto({ label: 'Gold week', benefitClass: 'PRICE', rateBefore: '35.42', rateAfter: '35.52' })],
    },
  });

function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(status === 304 ? null : JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

function makeServer(previewAnswer: () => unknown) {
  const previews: Array<Record<string, unknown>> = [];
  const ratesCalls: Array<{ path: string; ifNoneMatch: string | null }> = [];
  let ratesEtag = '"e1"';
  const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
    const u = new URL(input, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (u.pathname === '/api/public/host') return json(200, { org: 'XC' });
    if (u.pathname === '/api/public/orgs/XC/branches') {
      return json(200, {
        branches: [
          {
            id: 'bkk-1',
            code: 'BKK1',
            name: 'Bangkok 1',
            address: null,
            openingHours: null,
            timezone: 'Asia/Bangkok',
            bookingEnabled: true,
            queueOpen: true,
          },
        ],
      });
    }
    if (u.pathname === '/api/public/orgs/XC/branches/BKK1/rates') {
      const inm = new Headers(init?.headers).get('if-none-match');
      ratesCalls.push({ path: u.pathname, ifNoneMatch: inm });
      if (inm === ratesEtag) return json(304, null, { etag: ratesEtag });
      return json(200, { asOf: ratesEtag, rates: [] }, { etag: ratesEtag });
    }
    if (u.pathname === '/api/v3/views/my_bookings') {
      return json(200, { version: 1, changed: true, data: makeMyBookingsData({ bookings: [BOOKING] }) });
    }
    if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
      previews.push(typeof init?.body === 'string' ? JSON.parse(init.body) : {});
      return json(200, previewAnswer());
    }
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  });
  return {
    fetchFn,
    previews,
    ratesCalls,
    newRates() {
      ratesEtag = '"e2"';
    },
  };
}

describe('C03 my bookings: a booking opened shows its indicative benefit (FE-2, rows 23 and 24)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function mount(server: ReturnType<typeof makeServer>): Promise<HTMLElement> {
    dispose = render(() => <BookingsScreen fetch={server.fetchFn} ratesIntervalMs={20} />, host);
    await vi.waitFor(() => expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull());
    return host.querySelector<HTMLElement>('[data-booking-id="bk-1"]')!;
  }

  const openEstimate = (card: HTMLElement) =>
    card.querySelector<HTMLButtonElement>('button[data-action="booking-estimate"]')!.click();

  it('E1 a CONFIRMED booking card offers the estimate; nothing is asked before it is opened', async () => {
    const server = makeServer(() => ESTIMATED('2026-11-30T00:00:00Z'));
    const card = await mount(server);
    expect(card.querySelector('button[data-action="booking-estimate"]'), 'the open-estimate button').not.toBeNull();
    await flush();
    expect(server.previews).toEqual([]);
  });

  it('E2 opening sends one planner_preview from the booking intent and shows the estimate in the card', async () => {
    const server = makeServer(() => ESTIMATED('2026-11-30T00:00:00Z'));
    const card = await mount(server);
    openEstimate(card);
    await vi.waitFor(() => expect(card.querySelector('[data-row="benefit-estimate"]')).not.toBeNull());
    expect(server.previews).toEqual([
      { branch: 'bkk-1', currency: 'USD', customerSide: 'CUSTOMER_SELL', amount: { quantity: '100' } },
    ]);
    expect(card.querySelector('[data-row="benefit-item"]')?.textContent).toContain('Gold week: 35.42 → 35.52');
    expect(card.querySelector('[data-row="benefit-ends-before-slot"]')).toBeNull();
  });

  it('E3 (row 23) the slot starts at or after validUntil: the card warns that the promotion ends first', async () => {
    const server = makeServer(() => ESTIMATED('2026-11-05T03:00:00Z'));
    const card = await mount(server);
    openEstimate(card);
    await vi.waitFor(() => expect(card.querySelector('[data-row="benefit-ends-before-slot"]')).not.toBeNull());
  });

  it('E4 NONE shows no estimate block and no amount', async () => {
    const server = makeServer(() =>
      makePlannerPreviewAnswer({
        benefit: { state: 'NONE', basis: 'INDICATIVE', channel: 'APP', evaluatedAt: '2026-11-01T00:00:00Z', reason: 'notEligible' },
      }),
    );
    const card = await mount(server);
    openEstimate(card);
    await vi.waitFor(() => expect(server.previews).toHaveLength(1));
    await flush();
    expect(card.querySelector('[data-row="benefit-estimate"]')).toBeNull();
  });

  it('E5 (row 24) no further call on a timer while the rates route answers 304; one more after a new ETag', async () => {
    const server = makeServer(() => ESTIMATED('2026-11-30T00:00:00Z'));
    const card = await mount(server);
    openEstimate(card);
    await vi.waitFor(() => expect(card.querySelector('[data-row="benefit-estimate"]')).not.toBeNull());
    await vi.waitFor(() => expect(server.ratesCalls.length).toBeGreaterThanOrEqual(3), { timeout: 2000 });
    expect(
      server.ratesCalls.slice(1).every((c) => c.ifNoneMatch === '"e1"'),
      'the rates route is polled with If-None-Match',
    ).toBe(true);
    await new Promise((r) => setTimeout(r, 150));
    expect(server.previews, 'no planner_preview on a timer').toHaveLength(1);

    server.newRates();
    await vi.waitFor(() => expect(server.previews).toHaveLength(2), { timeout: 2000 });
    await new Promise((r) => setTimeout(r, 150));
    expect(server.previews, 'exactly one more after the new ETag').toHaveLength(2);
  });
});
