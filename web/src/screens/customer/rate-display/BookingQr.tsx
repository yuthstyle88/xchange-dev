// RD-4 booking QR component (design rate-display.md D9, D9a).
import { Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { QrCode } from '../../../shared/ui/QrCode';
import { en, th, zh } from './i18n';

const t = screenT('customer.rate-display', { th, en, zh });

export interface BookingQrProps {
  branchCode?: string;
  bookingEnabled?: boolean;
}

export function BookingQr(props: BookingQrProps): JSX.Element {
  const targetUrl = () => `${window.location.origin}/app/book?branch=${props.branchCode ?? ''}`;
  const captionPath = () => `/app/book?branch=${props.branchCode ?? ''}`;

  return (
    <Show when={props.bookingEnabled && props.branchCode}>
      <div
        class="xc-rate-display-booking-qr"
        data-booking-qr
        data-qr-url={targetUrl()}
      >
        <QrCode value={targetUrl()} label={t('bookingQrLabel')} />
        <div class="xc-rate-display-booking-qr-caption">
          <span class="xc-rate-display-booking-qr-text">{t('bookingQr')}</span>
          <span class="xc-rate-display-booking-qr-path">{captionPath()}</span>
        </div>
      </div>
    </Show>
  );
}

export default BookingQr;
