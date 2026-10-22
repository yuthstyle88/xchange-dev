// QW-2.0 (kiosk-shared-phone §5r F7–F9, §5r.4a): the /app/queue screen, glue only. It builds the parts and wires
// them; every rule lives in a part: wire calls in queue-api.ts (QW-2a), screen state in queue-state.ts (QW-2b),
// the form in RequestForm.tsx (QW-2c), the QR in RequestQr.tsx (QW-2d), the live status in RequestStatus.tsx
// (QW-2e), the buttons in RequestActions.tsx (QW-2f), the borrowed tablet in queue-tablet.ts (QW-2g).
// The whole-screen tests are QueueRequest.test.tsx; its header pins this file's export.
import { createSignal, Match, onMount, Show, Switch, type JSX } from 'solid-js';
import type { Fetch } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';
import { QueueApi } from './queue-api';
import { createQueueRequestState, type QueuePhase } from './queue-state';
import { useQueueTablet } from './queue-tablet';
import { RequestActions } from './RequestActions';
import { RequestForm } from './RequestForm';
import { RequestQr } from './RequestQr';
import { RequestStatus } from './RequestStatus';
import './queue.css';

const t = screenT('customer.queue', table);

/** The status poll's default interval. */
const POLL_MS = 5_000;

export interface QueueRequestScreenProps {
  /** Injected as in BookingsScreen, so tests mock at the fetch level. */
  fetch?: Fetch;
  /** The page's query string; defaults to `location.search`. */
  search?: string;
  intervalMs?: number;
}

type Open = Extract<QueuePhase, { kind: 'open' }>;

export function QueueRequestScreen(props: QueueRequestScreenProps = {}): JSX.Element {
  const fetchFn: Fetch = props.fetch ?? ((input, init) => fetch(input, init));
  const search = props.search ?? window.location.search;
  const api = new QueueApi(fetchFn);
  const state = createQueueRequestState(api, search);
  const tablet = useQueueTablet(state.branchCode, api.client);
  const [signedOut, setSignedOut] = createSignal(false);
  let scanDone = false;

  onMount(() => {
    void state.load().then((r) => setSignedOut(r.signedOut));
  });

  const signInHref = `/app/sign-in?next=${encodeURIComponent(`/app/queue${search}`)}`;
  const failed = () => {
    const p = state.phase();
    return p.kind === 'failed' ? p : undefined;
  };
  const form = () => {
    const p = state.phase();
    return p.kind === 'form' ? p : undefined;
  };
  const open = () => {
    const p = state.phase();
    return p.kind === 'open' ? (p as Open) : undefined;
  };

  return (
    <section class="xc-queue">
      <h1 class="xc-queue-title">{t('title')}</h1>
      <p class="xc-queue-subtitle">{t('subtitle')}</p>
      <Show
        when={!signedOut()}
        fallback={
          <a class="xc-queue-signin" href={signInHref}>
            {t('signIn')}
          </a>
        }
      >
        <Switch>
          <Match when={state.phase().kind === 'loading'}>
            <p class="xc-queue-loading">{t('loading')}</p>
          </Match>
          <Match when={failed()}>
            {(f) => (
              <p class="xc-queue-error" role="alert">
                {f().message}
              </p>
            )}
          </Match>
          <Match when={form()}>
            {(f) => (
              <>
                <Show when={f().expired}>
                  <p class="xc-queue-notice" role="status">
                    {t('expired')}
                  </p>
                </Show>
                <RequestForm rates={state.rates()} busy={state.busy()} onSubmit={(v) => void state.submit(v)} />
              </>
            )}
          </Match>
          <Match when={open()}>
            {(o) => (
              <>
                <RequestQr token={o().request.token} expiresAt={o().request.expiresAt} />
                <RequestStatus
                  client={api.client}
                  branchId={state.branchId() ?? ''}
                  requestId={o().request.requestId}
                  expiresAt={o().request.expiresAt}
                  intervalMs={props.intervalMs ?? POLL_MS}
                  status={o().status}
                  onExpired={() => state.expire()}
                  onStatus={(s) => {
                    if (tablet.shared && !scanDone && s !== 'QUOTED') {
                      scanDone = true;
                      void tablet.done();
                    }
                  }}
                />
                <RequestActions
                  emailCopy={state.emailCopy()}
                  status={o().status}
                  onCancel={() => state.cancel()}
                  onDone={async () => {
                    if (tablet.shared) await tablet.done();
                    else window.location.assign('/app/bookings');
                  }}
                  onEmailCopy={() => api.emailCopy(o().request.requestId, o().request.token)}
                />
              </>
            )}
          </Match>
        </Switch>
        <Show when={state.error()}>
          {(message) => (
            <p class="xc-queue-error" role="alert">
              {message()}
            </p>
          )}
        </Show>
        <Show when={tablet.error()}>
          {(message) => (
            <p class="xc-queue-error" role="alert">
              {message()}
            </p>
          )}
        </Show>
      </Show>
    </section>
  );
}
