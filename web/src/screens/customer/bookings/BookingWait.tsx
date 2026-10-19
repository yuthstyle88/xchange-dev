// C03: a booking's place in the queue and its wait (design booking-slots.md:241; BE contract #3404,
// the same names as `my_queue_ticket`).
// SKELETON STUB (feat/fe-customer-2): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { MyBookingItemDto } from '../../../shared/api/generated/my_bookings';

/** The fields #3404 adds to `MyBookingItemDto`. Optional until the generated type has them. */
export interface BookingWaitFields {
  ticketsAhead?: number | null;
  estimateSeconds?: number | null;
  estimateLowSeconds?: number | null;
  estimateHighSeconds?: number | null;
  estimateAsOf?: string | null;
}

export interface BookingWaitProps {
  booking: MyBookingItemDto & BookingWaitFields;
}

export function BookingWait(_props: BookingWaitProps): JSX.Element {
  return null;
}
