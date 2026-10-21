// C03: a booking card's confirm button, QR re-issue and cancel deadline (design customer-app.md §4.5;
// commands bookingConfirm `POST /api/bookings/{id}/confirm`, bookingQrReissue `POST /api/bookings/{id}/qr`).
import { createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  BookingConfirmInput,
  BookingConfirmOutput,
  BookingQrReissueInput,
  BookingQrReissueOutput,
} from '../../../shared/api/generated/bookings';
import type { MyBookingItemDto } from '../../../shared/api/generated/my_bookings';
import { locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';

const t = screenT('customer.bookings', table);

const FINISHED = ['COMPLETED', 'CANCELLED', 'NO_SHOW', 'EXPIRED'];
const CAN_REISSUE = ['PENDING_CUSTOMER_CONFIRMATION', 'CONFIRMED', 'CHECKED_IN'];

export interface BookingExtrasProps {
  booking: MyBookingItemDto;
  /** The screen's client; writes go through `keyedWrite` so a retry reuses its Idempotency-Key. */
  api: ApiClient;
  /** Merges the fields a command changed into the screen's copy of this booking. */
  onChange: (patch: Partial<MyBookingItemDto>) => void;
  timeZone?: string;
}

export function BookingExtras(props: BookingExtrasProps): JSX.Element {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const confirmWrite = keyedWrite<BookingConfirmInput & { branch: string }, BookingConfirmOutput>(
    props.api,
    'POST',
    (b) => `/api/bookings/${b.id}/confirm`,
  );
  const qrWrite = keyedWrite<BookingQrReissueInput & { branch: string }, BookingQrReissueOutput>(
    props.api,
    'POST',
    (b) => `/api/bookings/${b.id}/qr`,
  );

  const handleConfirm = async () => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      const out = await confirmWrite.send({ id: props.booking.id, branch: props.booking.branchId });
      props.onChange({ status: out.status });
    } catch (err) {
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  const handleReissueQr = async () => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      const out = await qrWrite.send({ id: props.booking.id, branch: props.booking.branchId });
      props.onChange({ qrToken: out.qrToken });
    } catch (err) {
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  const deadlineText = () => {
    if (FINISHED.includes(props.booking.status) || !props.booking.cancelCutoffAt) return null;
    const cutoff = new Date(props.booking.cancelCutoffAt);
    if (cutoff.getTime() <= Date.now()) return null;
    const date = cutoff.toLocaleString(locale(), {
      dateStyle: 'medium',
      timeStyle: 'short',
      timeZone: props.timeZone,
    });
    return t('rescheduleDeadline', { date });
  };

  return (
    <div class="xc-booking-extras">
      <Show when={deadlineText()}>
        {(text) => (
          <p class="xc-cancel-deadline" data-testid="cancel-deadline">
            {text()}
          </p>
        )}
      </Show>
      <Show when={error()}>
        {(msg) => (
          <p class="xc-extras-error" data-testid="extras-error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
      <Show when={CAN_REISSUE.includes(props.booking.status)}>
        <div class="xc-booking-extras-actions">
          <Show when={props.booking.status === 'PENDING_CUSTOMER_CONFIRMATION'}>
            <button
              type="button"
              class="xc-btn xc-btn-primary"
              data-action="confirm"
              disabled={busy()}
              onClick={handleConfirm}
            >
              {t('confirm')}
            </button>
          </Show>
          <button
            type="button"
            class="xc-btn xc-btn-secondary"
            data-action="reissue-qr"
            disabled={busy()}
            onClick={handleReissueQr}
          >
            {t('reissueQr')}
          </button>
        </div>
      </Show>
    </div>
  );
}
