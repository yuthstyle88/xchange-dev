import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { QrCode } from '../../../shared/ui/QrCode';
import { th, en, zh } from './i18n';
import './copy-link.css';

export interface CopyDetails {
  subject?: string;
  status?: string;
  displayNumber?: string;
  number?: string;
  branch?: string;
  branchName?: string;
  currency?: string;
  amount?: number | string;
  rate?: number | string;
  date?: string;
  slot?: string;
  expiresAt?: string;
  [key: string]: unknown;
}

export interface CopyLinkProps {
  fetch?: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
  fragment?: string;
  url?: string;
}

function extractQrFromFragment(frag: string, origin: string): string {
  const parts = frag.split('.');
  if (parts.length >= 6) {
    const subject = parts[1];
    const qrPart = parts[4];
    if (subject === 'webRequest') {
      if (qrPart.startsWith('http://') || qrPart.startsWith('https://')) {
        return qrPart;
      }
      return origin ? `${origin}/q#${qrPart}` : `/q#${qrPart}`;
    }
    return qrPart;
  }
  return frag;
}

export function CopyLink(props: CopyLinkProps): JSX.Element {
  const t = screenT('customer.copy-link', { th, en, zh });

  const [loading, setLoading] = createSignal(true);
  const [expired, setExpired] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [data, setData] = createSignal<CopyDetails | null>(null);

  const getFragment = (): string => {
    if (props.fragment !== undefined) return props.fragment.replace(/^#/, '');
    if (props.url) {
      try {
        const u = new URL(props.url);
        return u.hash.replace(/^#/, '');
      } catch {
        const idx = props.url.indexOf('#');
        if (idx !== -1) return props.url.slice(idx + 1);
      }
    }
    if (typeof window !== 'undefined' && window.location?.hash) {
      return window.location.hash.replace(/^#/, '');
    }
    return '';
  };

  const getOrigin = (): string => {
    if (props.url) {
      try {
        return new URL(props.url).origin;
      } catch {
        // ignore
      }
    }
    return typeof window !== 'undefined' && window.location?.origin ? window.location.origin : '';
  };

  const qrValue = () => extractQrFromFragment(getFragment(), getOrigin());

  const statusLabel = (st?: string): string => {
    if (!st) return '';
    switch (st.toUpperCase()) {
      case 'QUOTED':
        return t('statusQuoted');
      case 'WAITING':
        return t('statusWaiting');
      case 'CALLED':
        return t('statusCalled');
      case 'SERVING':
        return t('statusServing');
      case 'COMPLETED':
        return t('statusCompleted');
      case 'CANCELLED':
        return t('statusCancelled');
      case 'NO_SHOW':
        return t('statusNoShow');
      case 'CONFIRMED':
        return t('statusConfirmed');
      default:
        return st;
    }
  };

  onMount(async () => {
    const frag = getFragment();
    if (!frag) {
      setLoading(false);
      setError('invalid');
      return;
    }

    const fetcher = props.fetch ?? fetch;
    try {
      const res = await fetcher('/api/public/copy-link', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ fragment: frag }),
      });

      if (res.status === 410) {
        setExpired(true);
        setLoading(false);
        return;
      }

      if (!res.ok) {
        let body: unknown;
        try {
          body = await res.json();
        } catch {
          // ignore
        }
        const b = body as { error?: string | { code?: string } } | null;
        if (b && (b.error === 'linkExpired' || (typeof b.error === 'object' && b.error?.code === 'linkExpired'))) {
          setExpired(true);
        } else {
          setError('loadFailed');
        }
        setLoading(false);
        return;
      }

      const json = await res.json();
      const details = (json.details ?? json.data ?? json) as CopyDetails;
      if (details.status === 'EXPIRED' || (json as { error?: string }).error === 'linkExpired') {
        setExpired(true);
      } else {
        setData(details);
      }
    } catch {
      setError('loadFailed');
    } finally {
      setLoading(false);
    }
  });

  return (
    <div class="xc-copy-link">
      <div class="xc-copy-link-card">
        <h1 class="xc-copy-link-title">{t('title')}</h1>

        <Show when={loading()}>
          <p class="xc-copy-link-loading">{t('loading')}</p>
        </Show>

        <Show when={!loading() && expired()}>
          <p class="xc-copy-link-error">{t('linkExpired')}</p>
        </Show>

        <Show when={!loading() && !expired() && error()}>
          <p class="xc-copy-link-error">{t(error() as 'invalid' | 'loadFailed')}</p>
        </Show>

        <Show when={!loading() && !expired() && !error() && data()}>
          {(d) => (
            <>
              <Show when={d().status}>
                <div class="xc-copy-link-status" data-status={d().status}>
                  {statusLabel(d().status)}
                </div>
              </Show>

              <div class="xc-copy-link-details">
                <Show when={d().displayNumber || d().number}>
                  <div class="xc-copy-link-row">
                    <span class="xc-copy-link-label">{t('number')}</span>
                    <span class="xc-copy-link-value">{d().displayNumber ?? d().number}</span>
                  </div>
                </Show>

                <Show when={d().branchName || d().branch}>
                  <div class="xc-copy-link-row">
                    <span class="xc-copy-link-label">{t('branch')}</span>
                    <span class="xc-copy-link-value">{d().branchName ?? d().branch}</span>
                  </div>
                </Show>

                <Show when={d().currency || d().amount !== undefined}>
                  <div class="xc-copy-link-row">
                    <span class="xc-copy-link-label">{t('amount')}</span>
                    <span class="xc-copy-link-value">
                      {d().currency ? `${d().currency} ` : ''}
                      {d().amount}
                    </span>
                  </div>
                </Show>

                <Show when={d().rate !== undefined}>
                  <div class="xc-copy-link-row">
                    <span class="xc-copy-link-label">{t('rate')}</span>
                    <span class="xc-copy-link-value">{d().rate}</span>
                  </div>
                </Show>

                <Show when={d().date || d().slot}>
                  <div class="xc-copy-link-row">
                    <span class="xc-copy-link-label">{t('date')}</span>
                    <span class="xc-copy-link-value">
                      {d().date}
                      {d().slot ? ` (${d().slot})` : ''}
                    </span>
                  </div>
                </Show>
              </div>

              <div class="xc-copy-link-code" data-copy-qr>
                <QrCode value={qrValue()} label={t('qrLabel')} />
              </div>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}

export default CopyLink;
