// QW-2e (kiosk-shared-phone §4.3a W4, W4a, W5; §5r F8; rows U3, U4 and the expiry of §5r.3; §5r.4a): the request's
// live status. Skeleton by QW-2.0; the body is QW-2e's.
//
// - One `ViewPoller` (shared/live/view-poller.ts) on `my_web_request` with params `{branch: branchId, filter: {id}}`
//   every `intervalMs`; every read after the first carries `since=<the last version>` (W4a: an opaque string such as
//   "12.0a1b2c3d4e5f6a7b", passed through as is). Stopped on unmount.
// - [data-request-status] carries the status in its attribute and shows it in words. The shown status is the latest
//   of the `status` prop (what the page itself knows, e.g. CANCELLED right after cancel) and the polled one: a change
//   of the prop shows at once, with no poll in between.
// - [data-display-number] holds the number once there is one (empty before); the place in line (`ticketsAhead`)
//   shows while it is a number.
// - A `404 notFound` read after `expiresAt` ⇒ `onExpired()` once (W5: there is no EXPIRED status). A 404 before
//   `expiresAt` is not an expiry.
// Classes: xc-queue-status, xc-queue-status-label, xc-queue-number, xc-queue-ahead.
// Texts: statusLabel, status_<W5 status>, number, ahead.
import { createEffect, onCleanup, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { ViewPoller } from '../../../shared/live/view-poller';
import * as table from './i18n';
import type { MyWebRequestData, WebRequestStatus } from './queue-api';

const t = screenT('customer.queue', table);

export interface RequestStatusProps {
  client: Pick<ApiClient, 'view'>;
  branchId: string;
  requestId: string;
  /** RFC 3339: a 404 after it is the expiry. */
  expiresAt: string;
  intervalMs: number;
  /** What QueueRequestState knows (CANCELLED after cancel()); null when it knows nothing. */
  status: WebRequestStatus | null;
  onExpired: () => void;
  onStatus?: (s: WebRequestStatus) => void;
}

export function RequestStatus(props: RequestStatusProps): JSX.Element {
  let expiredCalled = false;
  const client: Pick<ApiClient, 'view'> = {
    view: async (name, q) => {
      try {
        return await props.client.view(name, q);
      } catch (err) {
        if ((err as { status?: number })?.status === 404 && !expiredCalled && Date.now() >= Date.parse(props.expiresAt)) {
          expiredCalled = true;
          props.onExpired();
        }
        throw err;
      }
    },
  };

  const poller = new ViewPoller<MyWebRequestData>(client, 'my_web_request', {
    params: { branch: props.branchId, filter: { id: props.requestId } },
    intervalMs: props.intervalMs,
    maxIntervalMs: props.intervalMs,
  });
  poller.start();
  onCleanup(() => poller.stop());

  const status = () => props.status ?? poller.data()?.status ?? null;
  const displayNumber = () => poller.data()?.displayNumber ?? null;
  const ticketsAhead = () => poller.data()?.ticketsAhead ?? null;

  createEffect<WebRequestStatus | null>((prev) => {
    const s = status();
    if (s && s !== prev) props.onStatus?.(s);
    return s;
  }, null);

  return (
    <div class="xc-queue-status" data-request-status={status() ?? ''}>
      <p class="xc-queue-status-label">
        {t('statusLabel')}: {status() ? t(`status_${status()!}`) : ''}
      </p>
      <p class="xc-queue-number" data-display-number={displayNumber() ?? ''} aria-label={t('number')}>
        {displayNumber() ?? ''}
      </p>
      <Show when={typeof ticketsAhead() === 'number'}>
        <p class="xc-queue-ahead">{t('ahead', { count: ticketsAhead()! })}</p>
      </Show>
    </div>
  );
}
