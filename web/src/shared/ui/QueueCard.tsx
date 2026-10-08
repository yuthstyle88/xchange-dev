import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { t } from '../i18n';
import { queueBranch, useQueueStore } from '../queue/queue-store';
import { announce, soundOn } from '../queue/queue-sound';
import type { SessionStore } from '../session/session';
import { QueueSoundControls } from './QueueSoundControls';

const [dockCount, setDockCount] = createSignal(0);

export function dockQueueCard(): () => void {
  setDockCount((c) => c + 1);
  let released = false;
  return () => {
    if (!released) {
      released = true;
      setDockCount((c) => Math.max(0, c - 1));
    }
  };
}

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

  createEffect(() => {
    const s = store().state();
    const u = props.session.me();
    const b = queueBranch();
    const on = soundOn();
    if (on && u && s && s.count > 0 && s.latest) {
      void announce(s, u.id, b);
    }
  });

  const isEnded = () => store().error()?.kind === 'ended';

  const canClaim = () => {
    const r = props.session.me()?.role;
    return r ? CLAIMING_ROLES.includes(r) : false;
  };

  return (
    <Show when={props.session.me() && dockCount() === 0}>
      <Portal>
        <aside aria-label={t('queue.card.aria')} classList={{ waiting: isWaiting() }}>
          <div class="queue-card-top">
            <span>{badgeText()}</span>
            <span>{titleText()}</span>
            <button
              type="button"
              aria-label={t('queue.card.foldAria')}
              aria-expanded={isFolded() ? 'false' : 'true'}
              onClick={toggleFold}
            >
              {isFolded() ? '+' : '−'}
            </button>
          </div>
          <div role="status" aria-live="polite">
            {statusText()}
          </div>
          <div hidden={isFolded()}>
            <Show when={isEnded()}>
              <a href="/login">{t('queue.card.signInAgain')}</a>
            </Show>
            <Show when={!isEnded() && canClaim()}>
              <a
                href={`/kiosk-queue.html?branch=${encodeURIComponent(queueBranch())}`}
                target="_blank"
                rel="noopener noreferrer"
              >
                {t('queue.card.openQueue')}
              </a>
            </Show>
            <QueueSoundControls />
          </div>
        </aside>
      </Portal>
    </Show>
  );
}
