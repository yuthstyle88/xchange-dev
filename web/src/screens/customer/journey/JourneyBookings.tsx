// C09: the journey's upcoming bookings with their check-in QR, from my_bookings (design customer-app.md §4.5,
// §9.1).
import { createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type { MyBookingItemDto, MyBookingsData } from '../../../shared/api/generated/my_bookings';
import { locale, screenT } from '../../../shared/i18n';
import { QrCode } from '../../../shared/ui/QrCode';
import { th, en, zh } from './i18n';

export interface JourneyBookingsProps {
  /** The journey screen's client. */
  api: ApiClient;
}

const FINISHED = new Set(['COMPLETED', 'CANCELLED', 'NO_SHOW', 'EXPIRED']);

export function JourneyBookings(props: JourneyBookingsProps): JSX.Element {
  const t = screenT('customer.journey', { th, en, zh });
  const [state, setState] = createSignal<'loading' | 'ready' | 'unauthorized' | 'error'>('loading');
  const [bookings, setBookings] = createSignal<MyBookingItemDto[]>([]);

  onMount(() => {
    void props.api
      .view<MyBookingsData>('my_bookings')
      .then((res) => {
        const list = (res.data?.bookings ?? [])
          .filter((b) => !FINISHED.has(b.status))
          .slice()
          .sort((a, b) => Date.parse(a.startAt) - Date.parse(b.startAt));
        setBookings(list);
        setState('ready');
      })
      .catch((err: unknown) => {
        if (err instanceof ApiError && err.status === 401) {
          setState('unauthorized');
        } else {
          setState('error');
        }
      });
  });

  const formatStart = (iso: string): string => {
    try {
      const d = new Date(iso);
      if (Number.isNaN(d.getTime())) return iso;
      return d.toLocaleString(locale());
    } catch {
      return iso;
    }
  };

  return (
    <Show when={state() === 'ready' || state() === 'error'}>
      <div class="xc-journey-section">
        <div class="xc-journey-section-header">
          <h2 class="xc-journey-section-title">{t('bookingsTitle')}</h2>
        </div>

        <Show when={state() === 'error'}>
          <div data-testid="journey-bookings-error">{t('errorBookings')}</div>
        </Show>

        <Show when={state() === 'ready' && bookings().length === 0}>
          <div class="xc-journey-empty" data-testid="journey-bookings-empty">
            {t('noBookings')}
          </div>
        </Show>

        <Show when={state() === 'ready' && bookings().length > 0}>
          <div class="xc-journey-bookings-list">
            <For each={bookings()}>
              {(b) => (
                <div class="xc-journey-card" data-testid="journey-booking" data-booking-id={b.id}>
                  <div class="xc-journey-card-header">
                    <span class="xc-booking-no">{b.bookingNo}</span>
                    <span class="xc-booking-time">{formatStart(b.startAt)}</span>
                  </div>
                  <Show when={b.qrToken !== null}>
                    <div class="xc-booking-qr" data-qr-token={b.qrToken!}>
                      <QrCode value={b.qrToken!} label={t('bookingQr')} />
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
}
