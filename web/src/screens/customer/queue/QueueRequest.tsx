// QW-2.0 (kiosk-shared-phone §5r F7–F9, §5r.4a): the /app/queue screen, glue only. It builds the parts and wires
// them; every rule lives in a part: wire calls in queue-api.ts (QW-2a), screen state in queue-state.ts (QW-2b),
// the form in RequestForm.tsx (QW-2c), the QR in RequestQr.tsx (QW-2d), the live status in RequestStatus.tsx
// (QW-2e), the buttons in RequestActions.tsx (QW-2f), the branch tablet in queue-tablet.ts (BT-F2, §7).
// The whole-screen tests are QueueRequest.test.tsx; its header pins this file's export.
import { createEffect, createSignal, Match, Show, Switch, type JSX } from 'solid-js';
import type { Fetch, ViewParams } from '../../../shared/api/client';
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

function withBranch(rawSearch: string, branch: string): string {
  if (!branch) return rawSearch;
  const params = new URLSearchParams(rawSearch);
  params.set('branch', branch);
  return `?${params.toString()}`;
}

export function QueueRequestScreen(props: QueueRequestScreenProps = {}): JSX.Element {
  const fetchFn: Fetch = props.fetch ?? ((input, init) => fetch(input, init));
  const search = props.search ?? window.location.search;
  const api = new QueueApi(fetchFn);
  const tablet = useQueueTablet(search, fetchFn);
  const [state, setState] = createSignal(createQueueRequestState(api, search));
  const [signedOut, setSignedOut] = createSignal(false);

  let loaded = false;
  createEffect(() => {
    if (loaded || !tablet.mode()) return;
    loaded = true;
    if (tablet.signInFirst()) return;
    const nextState = createQueueRequestState(api, withBranch(search, tablet.branch()));
    setState(nextState);
    void nextState.load().then((r) => setSignedOut(r.signedOut));
  });

  const statusClient = {
    view: async <T, F = unknown>(name: string, q?: ViewParams<F>) => {
      try {
        return await api.client.view<T, F>(name, q);
      } catch (err) {
        if ((err as { status?: number })?.status === 401) {
          tablet.requestEnded();
        }
        throw err;
      }
    },
  };

  const signInHref = () => `/app/sign-in?next=${encodeURIComponent(tablet.signInNext())}`;
  const failed = () => {
    const p = state().phase();
    return p.kind === 'failed' ? p : undefined;
  };
  const reissueFailed = () => {
    const p = state().phase();
    return p.kind === 'reissueFailed' ? p : undefined;
  };
  const form = () => {
    const p = state().phase();
    return p.kind === 'form' ? p : undefined;
  };
  const open = () => {
    const p = state().phase();
    return p.kind === 'open' ? (p as Open) : undefined;
  };

  return (
    <section class="xc-queue">
      <h1 class="xc-queue-title">{t('title')}</h1>
      <p class="xc-queue-subtitle">{t('subtitle')}</p>
      <Show
        when={!signedOut() && !tablet.signInFirst()}
        fallback={
          <a class="xc-queue-signin" href={signInHref()}>
            {t('signIn')}
          </a>
        }
      >
        <Switch>
          <Match when={state().phase().kind === 'loading'}>
            <p class="xc-queue-loading">{t('loading')}</p>
          </Match>
          <Match when={failed()}>
            {(f) => (
              <p class="xc-queue-error" role="alert">
                {f().message}
              </p>
            )}
          </Match>
          <Match when={reissueFailed()}>
            {(f) => (
              <div class="xc-queue-reissue-failed">
                <p class="xc-queue-error" role="alert">
                  {f().message}
                </p>
                <button
                  type="button"
                  class="xc-queue-retry"
                  data-reissue-retry
                  onClick={() => void state().retryReissue().then((r) => setSignedOut(r.signedOut))}
                >
                  {t('retry')}
                </button>
              </div>
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
                <RequestForm rates={state().rates()} busy={state().busy()} onSubmit={(v) => void state().submit(v)} />
              </>
            )}
          </Match>
          <Match when={open()}>
            {(o) => (
              <>
                <RequestQr token={o().request.token} expiresAt={o().request.expiresAt} />
                <RequestStatus
                  client={statusClient}
                  branchId={state().branchId() ?? ''}
                  requestId={o().request.requestId}
                  expiresAt={o().request.expiresAt}
                  intervalMs={props.intervalMs ?? POLL_MS}
                  status={o().status}
                  signInHref={signInHref()}
                  onExpired={() => {
                    state().expire();
                    tablet.requestEnded();
                  }}
                  onStatus={(s) => {
                    if (s !== 'QUOTED') tablet.requestEnded();
                  }}
                />
                <RequestActions
                  emailCopy={state().emailCopy()}
                  status={o().status}
                  onCancel={() => state().cancel()}
                  onDone={async () => {
                    tablet.done();
                  }}
                  onEmailCopy={() => api.emailCopy(o().request.requestId, o().request.token)}
                />
              </>
            )}
          </Match>
        </Switch>
        <Show when={state().error()}>
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
