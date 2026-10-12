// V2-queue (#471): admin customer-queue bar (design §12.12, §12.18.8; v2 admin-queue.js)

import {
  createEffect,
  createSignal,
  onCleanup,
  Show,
  type Accessor,
  type JSX,
} from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { summarize } from '../../../shared/queue/board';
import { setQueueBranch, useQueueStore } from '../../../shared/queue/queue-store';
import { sharedSession } from '../../../shared/session/session';
import { QueuePanel } from '../../../shared/ui/QueuePanel';
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

  let opener: HTMLElement | undefined;
  let dialogRef: HTMLDialogElement | undefined;

  const [dialogOpen, setDialogOpen] = createSignal(false);
  const [toastVisible, setToastVisible] = createSignal(false);
  const [toastMessage, setToastMessage] = createSignal('');
  const [locked, setLocked] = createSignal(false);

  createEffect(() => {
    const open = dialogOpen();
    if (!dialogRef) return;
    if (open) {
      if (typeof dialogRef.showModal === 'function') {
        if (!dialogRef.open) {
          dialogRef.showModal();
        }
      } else {
        dialogRef.open = true;
        dialogRef.setAttribute('open', '');
      }
    } else {
      if (typeof dialogRef.close === 'function') {
        if (dialogRef.open) {
          dialogRef.close();
        }
      } else {
        dialogRef.open = false;
        dialogRef.removeAttribute('open');
      }
    }
  });

  let prevBranch = props.branch();
  const [branchAwaitingAnswer, setBranchAwaitingAnswer] = createSignal(false);

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
        opener = link;
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

  // Clear branch awaiting answer once current branch answers: the store followed is that branch's (`useQueueStore`)
  createEffect(() => {
    if (state()) {
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
    const board = state();
    if (!board) return;
    const s = summarize(board);
    const currentBranch = props.branch();

    // QU-6 (#1354): `queue_board` names no employee; the key is the signed-in user's, as the card's sound key is.
    const empId = sharedSession().me()?.id;
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
          setToastMessage(t('toastNotice', { count: s.count, branch: props.branchLabel() }));
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

  return (
    <>
      <Show
        when={toastVisible() && !branchAwaitingAnswer() && error()?.kind !== 'offline'}
      >
        <aside class="admin-queue-toast" role="alert">
          <span class="toast-text">{toastMessage()}</span>
          <button
            type="button"
            class="toast-view-button"
            onClick={(e) => {
              opener = e.currentTarget;
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
        ref={dialogRef}
        aria-labelledby="queue-dialog-title"
        class="queue-dialog"
        onCancel={() => {
          setDialogOpen(false);
          opener?.focus();
        }}
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
              opener?.focus();
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
