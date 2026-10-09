// V2-queue (#471): admin customer-queue bar (design §12.12, §12.18.8; v2 admin-queue.js)

import {
  createEffect,
  createSignal,
  For,
  onCleanup,
  Show,
  type Accessor,
  type JSX,
} from 'solid-js';
import { locale, screenT } from '../../../shared/i18n';
import { setQueueBranch, useQueueStore } from '../../../shared/queue/queue-store';
import { dockQueueCard } from '../../../shared/ui/QueueCard';
import { QueuePanel } from '../../../shared/ui/QueuePanel';
import { QueueSoundControls } from '../../../shared/ui/QueueSoundControls';
import * as table from './i18n';
import './queue.css';

const t = screenT('admin.queue', table);

export function QueueBar(props: {
  branch: Accessor<string>;
  branchLabel: Accessor<string>;
}): JSX.Element {
  const store = useQueueStore(props.branch);
  const state = () => store().state();
  const error = () => store().error();

  let openButtonRef: HTMLButtonElement | undefined;

  const [dialogOpen, setDialogOpen] = createSignal(false);
  const [toastVisible, setToastVisible] = createSignal(false);
  const [toastMessage, setToastMessage] = createSignal('');
  const [locked, setLocked] = createSignal(false);

  let prevBranch = props.branch();
  const [branchAwaitingAnswer, setBranchAwaitingAnswer] = createSignal(false);

  // Dock queue card while mounted
  const undock = dockQueueCard();
  onCleanup(() => {
    undock();
  });

  // Intercept /kiosk-queue.html links
  const handleDocumentClick = (e: MouseEvent) => {
    const target = e.target as Element | null;
    const link = target?.closest('a');
    if (!link) return;
    const href = link.getAttribute('href');
    if (
      href &&
      (href === '/kiosk-queue.html' ||
        href.startsWith('/kiosk-queue.html?') ||
        href.startsWith('/kiosk-queue.html#'))
    ) {
      e.preventDefault();
      if (!buttonDisabled()) {
        setDialogOpen(true);
      }
    }
  };

  if (typeof document !== 'undefined') {
    document.addEventListener('click', handleDocumentClick, true);
    onCleanup(() => {
      document.removeEventListener('click', handleDocumentClick, true);
    });
  }

  // Tell store its branch on mount and on change
  createEffect(() => {
    const current = props.branch();
    setQueueBranch(current);
    if (current !== prevBranch) {
      prevBranch = current;
      setBranchAwaitingAnswer(true);
      setDialogOpen(false);
      setToastVisible(false);
    }
  });

  // Clear branch awaiting answer once current branch answers
  createEffect(() => {
    const s = state();
    const current = props.branch();
    if (s && s.branchId === current) {
      setBranchAwaitingAnswer(false);
    }
  });

  // Track session end lock
  createEffect(() => {
    const err = error();
    if (err?.kind === 'ended') {
      setLocked(true);
      setDialogOpen(false);
    }
  });

  // Track toast notifications
  createEffect(() => {
    const s = state();
    if (!s) return;
    const currentBranch = props.branch();
    if (s.branchId !== currentBranch) return;

    const empId = s.employee?.id;
    const storageKey = empId ? `xc.queue.popup.${empId}.${currentBranch}` : null;
    const latest = s.latest;

    if (s.count === 0) {
      setToastVisible(false);
    }

    if (latest && s.count > 0 && storageKey) {
      const prev = sessionStorage.getItem(storageKey);
      if (prev !== latest) {
        sessionStorage.setItem(storageKey, latest);
        if (!dialogOpen()) {
          setToastMessage(t('toastNotice', { count: s.count, branch: s.branchName }));
          setToastVisible(true);
        }
      }
    }
  });

  const buttonDisabled = () => {
    if (locked()) return true;
    if (error()?.kind === 'offline') return true;
    if (branchAwaitingAnswer()) return true;
    return false;
  };

  const statusText = () => {
    if (error()?.kind === 'offline') {
      return t('offline');
    }
    if (branchAwaitingAnswer()) {
      return t('checkingBranch');
    }
    const s = state();
    if (!s) {
      return t('checkingInitial');
    }
    return t('status', {
      waiting: s.count,
      working: s.workingCount,
      branch: s.branchName,
    });
  };

  const barTitle = () => {
    if (branchAwaitingAnswer()) {
      return t('titleInitial');
    }
    const s = state();
    if (!s) {
      return t('titleInitial');
    }
    return t('titleWithBranch', { branch: s.branchName });
  };

  const showWorkers = () => {
    if (error()?.kind === 'offline') return false;
    if (branchAwaitingAnswer()) return false;
    const s = state();
    if (!s || s.working.length === 0) return false;
    return true;
  };

  return (
    <>
      <aside class="admin-queue-bar" aria-label={t('barAria')}>
        <div class="queue-bar-main">
          <h3 class="queue-bar-title">{barTitle()}</h3>
          <div role="status" class="queue-bar-status">
            {statusText()}
          </div>
          <button
            type="button"
            ref={openButtonRef}
            class="queue-bar-open-button"
            disabled={buttonDisabled()}
            onClick={() => setDialogOpen(true)}
          >
            {t('openQueue')}
          </button>
          <div class="queue-sound-area">
            <QueueSoundControls />
          </div>
        </div>

        <Show when={showWorkers()}>
          <div class="queue-workers-list">
            <For each={state()?.working ?? []}>
              {(w) => {
                const stamp = () =>
                  w.startedAt
                    ? new Date(w.startedAt).toLocaleString(locale(), {
                        timeZone: 'Asia/Bangkok',
                      })
                    : t('noData');
                return (
                  <div class="queue-worker-line">
                    {t('workerLine', {
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
      </aside>

      <Show
        when={toastVisible() && !branchAwaitingAnswer() && error()?.kind !== 'offline'}
      >
        <aside class="admin-queue-toast" role="alert">
          <span class="toast-text">{toastMessage()}</span>
          <button
            type="button"
            class="toast-view-button"
            onClick={() => {
              setToastVisible(false);
              setDialogOpen(true);
            }}
          >
            {t('viewQueue')}
          </button>
          <button
            type="button"
            class="toast-close-button"
            onClick={() => setToastVisible(false)}
          >
            {t('closeToast')}
          </button>
        </aside>
      </Show>

      <dialog
        open={dialogOpen()}
        aria-labelledby="queue-dialog-title"
        class="queue-dialog"
      >
        <div class="queue-dialog-header">
          <h2 id="queue-dialog-title">
            {t('titleWithBranch', { branch: props.branchLabel() })}
          </h2>
          <button
            type="button"
            class="queue-dialog-close"
            onClick={() => {
              setDialogOpen(false);
              openButtonRef?.focus();
            }}
          >
            {t('closeDialog')}
          </button>
        </div>
        <Show when={dialogOpen()}>
          <QueuePanel branch={props.branch()} shown={dialogOpen()} />
        </Show>
      </dialog>
    </>
  );
}
