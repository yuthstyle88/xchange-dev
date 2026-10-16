import { createEffect, createSignal, on, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { th, en } from '../i18n';

export interface ReasonDialogProps {
  open: boolean;
  title: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  onConfirm: (reason: string) => void;
  onCancel: () => void;
}

export function ReasonDialog(props: ReasonDialogProps): JSX.Element {
  const t = screenT('staff', { th, en });
  const [reason, setReason] = createSignal('');

  // Each open starts with an empty reason so a note typed for one item is
  // never pre-filled into the next one (#2248).
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) setReason('');
      },
    ),
  );

  return (
    <Show when={props.open}>
      <dialog open class="xc-dialog">
        <div class="xc-dialog-content">
          <h3>{props.title}</h3>
          <input
            type="text"
            value={reason()}
            placeholder={props.placeholder ?? t('reasonPlaceholder')}
            onInput={(e) => setReason(e.currentTarget.value)}
          />
          <div class="xc-dialog-actions">
            <button type="button" onClick={() => props.onCancel()}>
              {props.cancelLabel ?? t('cancel')}
            </button>
            <button type="button" onClick={() => props.onConfirm(reason())}>
              {props.confirmLabel ?? t('confirm')}
            </button>
          </div>
        </div>
      </dialog>
    </Show>
  );
}
