import type { ApiClient } from '../../shared/api/client';
import type { DeskBookingsData } from '../../shared/api/generated/desk_bookings';
import type { QueueBoardData } from '../../shared/api/generated/queue_board';

export interface HeldBooking {
  bookingId: string;
  ticketId: string;
  displayNumber: string;
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
      booking: { bookingId: match.id, ticketId: row.id, displayNumber: row.displayNumber },
    };
  } catch {
    return { kind: 'unknown' };
  }
}
