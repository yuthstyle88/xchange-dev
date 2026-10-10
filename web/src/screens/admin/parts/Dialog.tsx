// V2-parts (#476): dialog host and controller (ticket §7, design §12.2)

import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { en, th } from './i18n';
import './parts.css';

interface DialogState {
  title: string;
  body: () => JSX.Element;
}

const [state, setState] = createSignal<DialogState | null>(null);
const [locked, setLocked] = createSignal<boolean>(false);

export function openDialog(title: string, body: () => JSX.Element): void {
  setState({ title, body });
}

export function closeDialog(): void {
  setState(null);
  setLocked(false);
}

export function setDialogLocked(val: boolean): void {
  setLocked(val);
}

export function isDialogLocked(): boolean {
  return locked();
}

export function Dialog(): JSX.Element {
  const t = screenT('admin.parts', { th, en });
  let dialogRef: HTMLDialogElement | undefined;

  createEffect(() => {
    const s = state();
    if (!dialogRef) return;
    if (s !== null) {
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

  return (
    <dialog
      ref={dialogRef}
      onCancel={(e) => {
        if (locked()) {
          e.preventDefault();
        } else {
          closeDialog();
        }
      }}
    >
      <Show when={state()}>
        {(current) => (
          <div class="dialog-content">
            <header>
              <h2>{current().title}</h2>
              <button
                type="button"
                aria-label={t('closeWindow')}
                disabled={locked()}
                onClick={() => {
                  if (!locked()) {
                    closeDialog();
                  }
                }}
              >
                ×
              </button>
            </header>
            <div class="dialog-body">{current().body()}</div>
          </div>
        )}
      </Show>
    </dialog>
  );
}
