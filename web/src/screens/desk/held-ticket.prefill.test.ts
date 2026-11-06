// F34 (#564 issuecomment-6657891388, customer simulation on xc-v3 at 7caf1e7, 2026-11-05): after the teller starts
// booking B001, the desk opens on BUY and AUD and the teller retypes the booking. The kiosk link pre-fills
// (`entries/kiosk/entry.ts:35-38`); a started booking has no entry, and the desk reads it only at step 1-2
// (`held-ticket.ts:25-53`), after the rate step. Since F34a (#6333) the `desk_bookings` row carries the booking's
// `shopSide`, `currency` and `quantity`.
//
// Written before the implementation by the ticket's author (claude/33691c06, sub-controller of the xchange lane
// controller claude/022a7453). Do not edit, delete or weaken these tests; add your own in another file.
//
// [required] by these tests:
// - `readHeldTicket` answers a held booking with `shopSide`, `currency` and `quantity` copied from its row.
// - `holdBooking(store, booking)` (exported from `held-ticket.ts`) keeps the booking (`setHeldBooking`) and pre-fills
//   the desk: `setType(shopSide)` (the shop's side, F33's single meaning), `setCurrency(currency)`, and
//   `setAmount(quantity)` without trailing zeros ("300.0000" → "300", "12.5000" → "12.5").
// - `openHeldBooking(store)` (exported from `held-ticket.ts`; `main.tsx` calls it after `openEntry` when the URL named
//   no entry): when the teller has an id, it reads the held ticket and holds a booking it finds; a walk-in or no held
//   ticket changes nothing. The new `HeldBooking` fields are optional, so a booking held without them still works.
import { describe, expect, it } from 'vitest';
import type { ApiClient } from '../../shared/api/client';
import { makeDeskBookingItemDto, makeQueueBoardCalled, makeQueueBoardData } from '../../shared/testing/fixtures';
import type { DeskStore } from './desk-store';
import * as held from './held-ticket';

type Calls = { name: string; args: unknown[] }[];

/** A view-only api: `queue_board` and `desk_bookings`, as `readHeldTicket` reads them. */
function fakeApi(opts: { serviceType?: 'BOOKING' | 'WALK_IN'; held?: boolean; quantity?: string }): ApiClient {
  const called = opts.held === false
    ? []
    : [
        makeQueueBoardCalled({
          id: 'qt-1',
          displayNumber: 'B001',
          serviceType: opts.serviceType ?? 'BOOKING',
          status: 'SERVING',
          tellerId: 'u-1',
        }),
      ];
  return {
    async view(name: string) {
      if (name === 'queue_board') return { data: makeQueueBoardData({ called }) };
      if (name === 'desk_bookings') {
        return {
          data: {
            items: [
              makeDeskBookingItemDto({
                id: 'BK-1',
                ticketNumber: 'B001',
                status: 'IN_SERVICE',
                shopSide: 'SELL',
                currency: 'USD',
                quantity: opts.quantity ?? '300.0000',
              }),
            ],
          },
        };
      }
      throw new Error(`unexpected view ${name}`);
    },
  } as unknown as ApiClient;
}

/** The store calls `holdBooking` and `openEntry` may make, recorded in order. */
function fakeStore(api: ApiClient, meId = 'u-1'): { store: DeskStore; calls: Calls } {
  const calls: Calls = [];
  const rec = (name: string) => (...args: unknown[]) => {
    calls.push({ name, args });
  };
  const store = {
    api,
    branch: () => 'MAIN',
    meId: () => meId,
    setEntry: rec('setEntry'),
    setHeldBooking: rec('setHeldBooking'),
    setType: rec('setType'),
    setCurrency: rec('setCurrency'),
    setAmount: rec('setAmount'),
  } as unknown as DeskStore;
  return { store, calls };
}

const holdBooking = (held as Record<string, unknown>).holdBooking as
  | ((store: DeskStore, booking: unknown) => void)
  | undefined;
const openHeldBooking = (held as Record<string, unknown>).openHeldBooking as
  | ((store: DeskStore) => Promise<void>)
  | undefined;

describe('F34: a started booking pre-fills the desk', () => {
  it('F34-1 readHeldTicket carries the booking side, currency and quantity', async () => {
    const read = await held.readHeldTicket(fakeApi({}), 'MAIN', 'u-1', '2026-11-05');
    expect(read.kind).toBe('booking');
    expect(read.kind === 'booking' ? read.booking : undefined).toMatchObject({
      bookingId: 'BK-1',
      ticketId: 'qt-1',
      displayNumber: 'B001',
      shopSide: 'SELL',
      currency: 'USD',
      quantity: '300.0000',
    });
  });

  it('F34-2 holdBooking keeps the booking and sets type, currency and amount', () => {
    expect(typeof holdBooking, 'held-ticket.ts exports holdBooking').toBe('function');
    const { store, calls } = fakeStore(fakeApi({}));
    const booking = {
      bookingId: 'BK-1',
      ticketId: 'qt-1',
      displayNumber: 'B001',
      shopSide: 'SELL',
      currency: 'USD',
      quantity: '300.0000',
    };
    holdBooking!(store, booking);
    expect(calls.find((c) => c.name === 'setHeldBooking')?.args[0]).toEqual(booking);
    expect(calls.find((c) => c.name === 'setType')?.args[0]).toBe('SELL');
    expect(calls.find((c) => c.name === 'setCurrency')?.args[0]).toBe('USD');
    expect(calls.find((c) => c.name === 'setAmount')?.args[0]).toBe('300');
  });

  it('F34-3 a fractional quantity keeps its digits and drops only trailing zeros', () => {
    expect(typeof holdBooking, 'held-ticket.ts exports holdBooking').toBe('function');
    const { store, calls } = fakeStore(fakeApi({}));
    holdBooking!(store, {
      bookingId: 'BK-1',
      ticketId: 'qt-1',
      displayNumber: 'B001',
      shopSide: 'BUY',
      currency: 'JPY',
      quantity: '12.5000',
    });
    expect(calls.find((c) => c.name === 'setAmount')?.args[0]).toBe('12.5');
    expect(calls.find((c) => c.name === 'setType')?.args[0]).toBe('BUY');
  });

  it('F34-4 openHeldBooking holds the started booking and pre-fills', async () => {
    expect(typeof openHeldBooking, 'held-ticket.ts exports openHeldBooking').toBe('function');
    const { store, calls } = fakeStore(fakeApi({}));
    await openHeldBooking!(store);
    expect(calls.map((c) => c.name)).toEqual(
      expect.arrayContaining(['setHeldBooking', 'setType', 'setCurrency', 'setAmount']),
    );
    expect(calls.find((c) => c.name === 'setType')?.args[0]).toBe('SELL');
    expect(calls.find((c) => c.name === 'setAmount')?.args[0]).toBe('300');
  });

  it('F34-5 openHeldBooking with a held walk-in, or nothing held, changes nothing', async () => {
    for (const api of [fakeApi({ serviceType: 'WALK_IN' }), fakeApi({ held: false })]) {
      expect(typeof openHeldBooking, 'held-ticket.ts exports openHeldBooking').toBe('function');
      const { store, calls } = fakeStore(api);
      await openHeldBooking!(store);
      expect(calls).toEqual([]);
    }
  });

  it('F34-6 openHeldBooking for a teller with no id reads nothing', async () => {
    expect(typeof openHeldBooking, 'held-ticket.ts exports openHeldBooking').toBe('function');
    const { store, calls } = fakeStore(fakeApi({}), '');
    await openHeldBooking!(store);
    expect(calls).toEqual([]);
  });
});
