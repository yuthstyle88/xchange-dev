import { Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { th, en } from '../i18n';

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog(props: ConfirmDialogProps): JSX.Element {
  const t = screenT('staff', { th, en });

  return (
    <Show when={props.open}>
      <dialog open class="xc-dialog">
        <div class="xc-dialog-content">
          <h3>{props.title}</h3>
          {props.message && <p>{props.message}</p>}
          <div class="xc-dialog-actions">
            <button type="button" onClick={() => props.onCancel()}>
              {props.cancelLabel ?? t('cancel')}
            </button>
            <button type="button" onClick={() => props.onConfirm()}>
              {props.confirmLabel ?? t('confirm')}
            </button>
          </div>
        </div>
      </dialog>
    </Show>
  );
}
