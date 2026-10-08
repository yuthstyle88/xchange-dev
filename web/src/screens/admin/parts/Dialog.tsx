// V2-parts (#476): dialog host and controller (ticket §7, design §12.2)

import { createSignal, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { en, th } from './i18n';

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

  return (
    <dialog
      open={state() !== null}
      onCancel={(e) => {
        if (locked()) {
          e.preventDefault();
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
