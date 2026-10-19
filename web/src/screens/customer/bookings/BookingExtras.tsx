// C03: a booking card's confirm button, QR re-issue and cancel deadline (design customer-app.md §4.5;
// commands bookingConfirm `POST /api/bookings/{id}/confirm`, bookingQrReissue `POST /api/bookings/{id}/qr`).
// SKELETON STUB (feat/fe-customer): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { MyBookingItemDto } from '../../../shared/api/generated/my_bookings';

export interface BookingExtrasProps {
  booking: MyBookingItemDto;
  /** The screen's client; writes go through `keyedWrite` so a retry reuses its Idempotency-Key. */
  api: ApiClient;
  /** Merges the fields a command changed into the screen's copy of this booking. */
  onChange: (patch: Partial<MyBookingItemDto>) => void;
}

export function BookingExtras(_props: BookingExtrasProps): JSX.Element {
  return null;
}
