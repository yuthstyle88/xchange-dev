// QW-2d (kiosk-shared-phone §5r F8, R-10; row U2's QR; §5r.4a): the request QR. Skeleton by QW-2.0; the body is
// QW-2d's (class mechanical; its behaviour table is on the ticket).
//
// Renders [data-request-qr][data-qr-url="<location.origin>/q#<token>"] holding `QrCode` (shared/ui/QrCode.tsx) of
// that URL, with its download, and the expiry (`expiresAt`, in the page's language).
// Classes: xc-queue-qr, xc-queue-qr-caption, xc-queue-expiry. Texts: qrLabel, downloadQr, showToStaff, expiresAt.
import type { JSX } from 'solid-js';
import { locale, screenT } from '../../../shared/i18n';
import { QrCode } from '../../../shared/ui/QrCode';
import * as table from './i18n';

const t = screenT('customer.queue', table);

export interface RequestQrProps {
  /** The request token, in memory only (W3). */
  token: string;
  /** RFC 3339. */
  expiresAt: string;
}

function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleTimeString(locale(), {
      hour: '2-digit',
      minute: '2-digit',
    });
  } catch {
    return iso;
  }
}

export function RequestQr(props: RequestQrProps): JSX.Element {
  const qrUrl = () => `${location.origin}/q#${props.token}`;

  return (
    <div class="xc-queue-qr" data-request-qr data-qr-url={qrUrl()}>
      <QrCode
        value={qrUrl()}
        label={t('qrLabel')}
        downloadName="queue-request-qr"
        downloadLabel={t('downloadQr')}
      />
      <p class="xc-queue-qr-caption">{t('showToStaff')}</p>
      <p class="xc-queue-expiry">{t('expiresAt', { time: formatTime(props.expiresAt) })}</p>
    </div>
  );
}
