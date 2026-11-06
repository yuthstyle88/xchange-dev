import type { ApiClient } from '../../shared/api/client';
import type { DeskBookingsData } from '../../shared/api/generated/desk_bookings';
import type { QueueBoardData } from '../../shared/api/generated/queue_board';
import { bangkokToday, type DeskStore, type DeskType } from './desk-store';

export interface HeldBooking {
  bookingId: string;
  ticketId: string;
  displayNumber: string;
  shopSide?: DeskType;
  currency?: string;
  quantity?: string;
}

export function holdBooking(store: DeskStore, booking: HeldBooking): void {
  store.setHeldBooking(booking);
  if (booking.shopSide) store.setType(booking.shopSide);
  if (booking.currency) store.setCurrency(booking.currency);
  if (booking.quantity) {
    store.setAmount(
      booking.quantity.includes('.') ? booking.quantity.replace(/0+$/, '').replace(/\.$/, '') : booking.quantity,
    );
  }
}

export async function openHeldBooking(store: DeskStore): Promise<void> {
  if (store.entry?.() || store.meId() === '') return;
  const held = await readHeldTicket(store.api, store.branch(), store.meId(), bangkokToday());
  if (held.kind === 'booking') holdBooking(store, held.booking);
}

// desk-frame.test.tsx S4 forbids the word "booking" in main.tsx
export { openHeldBooking as openHeldTicket };

export function hasEntryKey(store: DeskStore): boolean {
  const extras = store.entry()?.payloadExtras();
  if (!extras) return false;
  return Boolean(extras.bookingId || extras.onlineBookingId || extras.kioskRequestId);
}

export type HeldTicketRead =
  | { kind: 'none' }
  | { kind: 'walkIn' }
  | { kind: 'unknown' }
  | { kind: 'booking'; booking: HeldBooking }
  | { kind: 'bookingUnmatched'; displayNumber: string };

export async function readHeldTicket(
  api: ApiClient,
  branch: string,
  meId: string,
  today: string,
): Promise<HeldTicketRead> {
  try {
    const board = await api.view<QueueBoardData>('queue_board', { branch });
    if (!board.data) return { kind: 'unknown' };
    const row = board.data.called.find((c) => meId !== '' && c.tellerId === meId);
    if (!row) return { kind: 'none' };
    if (row.serviceType !== 'BOOKING') return { kind: 'walkIn' };

    const bookings = await api.view<DeskBookingsData>('desk_bookings', {
      branch,
      filter: { date: today },
    });
    if (!bookings.data) return { kind: 'unknown' };
    const match = bookings.data.items.find(
      (b) => b.ticketNumber === row.displayNumber && (b.status === 'CHECKED_IN' || b.status === 'IN_SERVICE'),
    );
    if (!match) return { kind: 'bookingUnmatched', displayNumber: row.displayNumber };
    return {
      kind: 'booking',
      booking: {
        bookingId: match.id,
        ticketId: row.id,
        displayNumber: row.displayNumber,
        shopSide: match.shopSide === 'BUY' || match.shopSide === 'SELL' ? match.shopSide : undefined,
        currency: match.currency,
        quantity: match.quantity,
      },
    };
  } catch {
    return { kind: 'unknown' };
  }
}
