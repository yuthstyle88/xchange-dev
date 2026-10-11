import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { locale, t } from '../i18n';
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

const CLAIMING_ROLES = ['OWNER', 'FRONT', 'BACKEND', 'MANAGER'];

export function QueueCard(props: { session: SessionStore }): JSX.Element {
  const [isFolded, setIsFolded] = createSignal(checkInitialFold());
  const toggleFold = () => setIsFolded((f) => !f);

  const store = useQueueStore(queueBranch);
  const [hasAnsweredAny, setHasAnsweredAny] = createSignal(false);
  let lastStatusLine = '';

  const badgeText = () => {
    const err = store().error();
    if (err && err.kind === 'offline') {
      return '!';
    }
    const s = store().state();
    if (s) {
      return String(s.count);
    }
    return '—';
  };

  const isWaiting = () => {
    if (store().error()) return false;
    const s = store().state();
    return Boolean(s && s.count > 0);
  };

  const titleText = () => {
    const s = store().state();
    if (s?.branchName) {
      return `XC · ${s.branchName}`;
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
    const s = store().state();
    if (s) {
      if (s.count > 0) {
        return t('queue.card.hasWaiting', { count: s.count, branchName: s.branchName });
      }
      return t('queue.card.noWaiting', { branchName: s.branchName });
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
    const s = store().state();
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
    const s = store().state();
    const u = props.session.me();
    const b = queueBranch();
    const on = soundOn();
    if (isEnded() || props.session.ended?.()) return;
    if (on && u && s && s.count > 0 && s.latest && b) {
      void announce(s, u.id, b);
    }
  });

  const canClaim = () => {
    const r = props.session.me()?.role;
    return r ? CLAIMING_ROLES.includes(r) : false;
  };

  const showWorkers = () => {
    if (store().error()?.kind === 'offline') return false;
    const s = store().state();
    if (!s || s.working.length === 0) return false;
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
                <For each={store().state()?.working ?? []}>
                  {(w) => {
                    const stamp = () =>
                      w.startedAt
                        ? new Date(w.startedAt).toLocaleString(locale(), {
                            timeZone: 'Asia/Bangkok',
                          })
                        : t('queue.card.noData');
                    return (
                      <div class="queue-card-worker">
                        {t('queue.card.workerLine', {
                          staff: w.staff,
                          queue: w.queue,
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
