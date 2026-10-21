import { createMemo, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { parseRequestQr } from '../../../shared/queue/request-qr';
import { QrCode } from '../../../shared/ui/QrCode';
import { th, en, zh } from './i18n';
import './request-qr.css';

export interface RequestQrProps {
  url?: string;
  fetch?: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
}

export function RequestQr(props: RequestQrProps): JSX.Element {
  const t = screenT('customer.request-qr', { th, en, zh });

  const qrData = createMemo(() => {
    const raw = props.url ?? (typeof window !== 'undefined' ? window.location.href : '');
    const token = parseRequestQr(raw);
    if (!token) return null;
    try {
      const u = new URL(raw);
      return { token, qrUrl: `${u.origin}/q#${token}` };
    } catch {
      return null;
    }
  });

  return (
    <div class="xc-request-qr">
      <div class="xc-request-qr-card">
        <h1 class="xc-request-qr-title">{t('title')}</h1>
        <Show
          when={qrData()}
          fallback={<p class="xc-request-qr-error">{t('invalid')}</p>}
        >
          {(data) => (
            <>
              <p class="xc-request-qr-instruction">{t('instruction')}</p>
              <div class="xc-request-qr-code" data-request-qr>
                <QrCode value={data().qrUrl} label={t('qrLabel')} />
              </div>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}

export default RequestQr;
