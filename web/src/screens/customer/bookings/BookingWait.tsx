// C03: a booking's place in the queue and its wait (design booking-slots.md:241; BE contract #3404,
// the same names as `my_queue_ticket`).
import { Show, type JSX } from 'solid-js';
import type { MyBookingItemDto } from '../../../shared/api/generated/my_bookings';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';

const t = screenT('customer.bookings', table);

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

export function BookingWait(props: BookingWaitProps): JSX.Element {
  const ahead = () => props.booking.ticketsAhead;
  const hasAhead = () => typeof ahead() === 'number' && ahead()! > 0;

  const low = () => props.booking.estimateLowSeconds;
  const high = () => props.booking.estimateHighSeconds;
  const hasEstimate = () =>
    hasAhead() &&
    low() !== null &&
    high() !== null &&
    typeof low() === 'number' &&
    typeof high() === 'number';

  return (
    <Show when={hasAhead()}>
      <div class="xc-booking-wait">
        <span class="xc-booking-wait-ahead" data-testid="booking-wait-ahead">
          {t('waitAhead', { count: ahead()! })}
        </span>
        <Show when={hasEstimate()}>
          <span class="xc-booking-wait-estimate" data-testid="booking-wait-estimate">
            {t('waitEstimate', {
              low: Math.ceil(low()! / 60),
              high: Math.ceil(high()! / 60),
            })}
          </span>
        </Show>
      </div>
    </Show>
  );
}
