import { createEffect, createSignal, on, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { th, en } from '../i18n';

export interface ReasonDialogProps {
  open: boolean;
  title: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  required?: boolean;
  maxLength?: number;
  onConfirm: (reason: string) => void | Promise<void>;
  onCancel: () => void;
}

export function ReasonDialog(props: ReasonDialogProps): JSX.Element {
  const t = screenT('staff', { th, en });
  const [reason, setReason] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);

  // Each open starts with an empty reason so a note typed for one item is
  // never pre-filled into the next one (#2248).
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) {
          setReason('');
          setError(null);
        }
      },
    ),
  );

  const handleConfirm = async () => {
    if (props.required && reason().trim().length === 0) {
      setError(t('reasonRequired'));
      return;
    }
    await props.onConfirm(reason());
  };

  return (
    <Show when={props.open}>
      <dialog open class="xc-dialog">
        <div class="xc-dialog-content">
          <h3>{props.title}</h3>
          <input
            type="text"
            value={reason()}
            maxLength={props.maxLength}
            placeholder={props.placeholder ?? t('reasonPlaceholder')}
            onInput={(e) => {
              setReason(e.currentTarget.value);
              if (error() && e.currentTarget.value.trim().length > 0) {
                setError(null);
              }
            }}
          />
          <Show when={props.maxLength !== undefined}>
            <div>{reason().length} / {props.maxLength}</div>
          </Show>
          <Show when={error()}>
            <div role="alert">{error()}</div>
          </Show>
          <div class="xc-dialog-actions">
            <button type="button" onClick={() => props.onCancel()}>
              {props.cancelLabel ?? t('cancel')}
            </button>
            <button type="button" onClick={handleConfirm}>
              {props.confirmLabel ?? t('confirm')}
            </button>
          </div>
        </div>
      </dialog>
    </Show>
  );
}
