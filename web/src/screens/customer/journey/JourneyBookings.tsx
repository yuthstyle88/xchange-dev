// C09: the journey's upcoming bookings with their check-in QR, from my_bookings (design customer-app.md §4.5,
// §9.1). SKELETON STUB (feat/fe-customer): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';

export interface JourneyBookingsProps {
  /** The journey screen's client. */
  api: ApiClient;
}

export function JourneyBookings(_props: JourneyBookingsProps): JSX.Element {
  return null;
}
