import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { locale, t } from '../i18n';
import { summarize } from '../queue/board';
import { queueBranch, useQueueStore } from '../queue/queue-store';
import { announce, soundOn } from '../queue/queue-sound';
import type { SessionStore } from '../session/session';
import { QueueSoundControls } from './QueueSoundControls';
import './queue-card.css';

function checkInitialFold(): boolean {
  if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
    try {
      return window.matchMedia('(max-width: 700px)').matches;
    } catch {
      return false;
    }
  }
  return false;
}

/** The server's row for `queue_board` (QueueOperate): only a caller who holds it gets the card (QU-6, #1354 ruling). */
function holdsQueueOperate(session: SessionStore): boolean {
  return session.me()?.permissions.includes('QueueOperate') ?? false;
}

export function QueueCard(props: { session: SessionStore }): JSX.Element {
  return (
    <Show when={holdsQueueOperate(props.session)}>
      <OperatorCard session={props.session} />
    </Show>
  );
}

function OperatorCard(props: { session: SessionStore }): JSX.Element {
  const [isFolded, setIsFolded] = createSignal(checkInitialFold());
  const toggleFold = () => setIsFolded((f) => !f);

  const store = useQueueStore(queueBranch);
  const [hasAnsweredAny, setHasAnsweredAny] = createSignal(false);
  let lastStatusLine = '';

  const summary = () => {
    const s = store().state();
    return s ? summarize(s) : undefined;
  };

  /** The name of the branch the card follows, from the signed-in user's branches. */
  const branchName = () => {
    const me = props.session.me();
    const b = queueBranch();
    if (!me || !b) return undefined;
    return me.branches.find((x) => x.id === b)?.name ?? (me.branch === b ? me.branchName : undefined);
  };

  const badgeText = () => {
    const err = store().error();
    if (err && err.kind === 'offline') {
      return '!';
    }
    const s = summary();
    if (s) {
      return String(s.count);
    }
    return '—';
  };

  const isWaiting = () => {
    if (store().error()) return false;
    const s = summary();
    return Boolean(s && s.count > 0);
  };

  const titleText = () => {
    const name = store().state() ? branchName() : undefined;
    if (name) {
      return `XC · ${name}`;
    }
    return t('queue.card.title');
  };

  const statusText = () => {
    const err = store().error();
    if (err) {
      if (err.kind === 'offline') {
        return t('queue.card.offline');
      }
      if (err.kind === 'ended') {
        if (err.reason === 'expired') {
          return t('queue.card.endedExpired');
        }
        if (err.reason === 'switched') {
          return t('queue.card.endedSwitched');
        }
      }
    }
    const s = summary();
    if (s) {
      const name = branchName() ?? '';
      if (s.count > 0) {
        return t('queue.card.hasWaiting', { count: s.count, branchName: name });
      }
      return t('queue.card.noWaiting', { branchName: name });
    }
    if (hasAnsweredAny()) {
      return t('queue.card.checkingBranch');
    }
    return t('queue.card.checking');
  };

  createEffect(() => {
    const err = store().error();
    if (err) {
      lastStatusLine = '';
      return;
    }
    const s = summary();
    if (s) {
      setHasAnsweredAny(true);
      const line = statusText();
      if (line !== lastStatusLine) {
        lastStatusLine = line;
        if (s.count > 0) {
          setIsFolded(false);
        }
      }
    }
  });

  const isEnded = () => store().error()?.kind === 'ended';

  createEffect(() => {
    const s = summary();
    const u = props.session.me();
    const b = queueBranch();
    const on = soundOn();
    if (isEnded() || props.session.ended?.()) return;
    if (on && u && s && s.count > 0 && s.latest && b) {
      void announce(s, u.id, b);
    }
  });

  // The server's row for the queue's accept/get (#1030 §3.10): claim iff `me.permissions` holds QueueOperate.
  const canClaim = () => props.session.me()?.permissions.includes('QueueOperate') ?? false;

  const showWorkers = () => {
    if (store().error()?.kind === 'offline') return false;
    const s = store().state();
    if (!s || s.called.length === 0) return false;
    return true;
  };

  return (
    <Show when={props.session.me()}>
      <Portal>
        <aside
          aria-label={t('queue.card.aria')}
          class="queue-card"
          classList={{ waiting: isWaiting(), minimized: isFolded() }}
        >
          <div class="queue-card-top">
            <span class="queue-card-badge">{badgeText()}</span>
            <span class="queue-card-title">{titleText()}</span>
            <button
              type="button"
              class="queue-card-toggle"
              aria-label={t('queue.card.foldAria')}
              aria-expanded={isFolded() ? 'false' : 'true'}
              onClick={toggleFold}
            >
              {isFolded() ? '+' : '−'}
            </button>
          </div>
          <div role="status" aria-live="polite" class="queue-card-status">
            {statusText()}
          </div>
          <div hidden={isFolded()} class="queue-card-body">
            <Show when={isEnded()}>
              <a href="/login" class="queue-card-action">{t('queue.card.signInAgain')}</a>
            </Show>
            <Show when={!isEnded() && canClaim() && queueBranch()}>
              {(b) => (
                <a
                  href={`/kiosk-queue.html?branch=${encodeURIComponent(b())}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="queue-card-action"
                >
                  {t('queue.card.openQueue')}
                </a>
              )}
            </Show>
            <Show when={showWorkers()}>
              <div class="queue-card-workers">
                <For each={store().state()?.called ?? []}>
                  {(w) => {
                    const stamp = () =>
                      w.calledAt
                        ? new Date(w.calledAt).toLocaleString(locale(), {
                            timeZone: 'Asia/Bangkok',
                          })
                        : t('queue.card.noData');
                    return (
                      <div class="queue-card-worker">
                        {t('queue.card.workerLine', {
                          staff: w.tellerId ?? t('queue.card.noData'),
                          queue: w.displayNumber,
                          time: stamp(),
                        })}
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>
            <QueueSoundControls />
          </div>
        </aside>
      </Portal>
    </Show>
  );
}
