import { createEffect, createSignal, For, on, Show, type JSX } from 'solid-js';
import { t } from '../i18n';
import type { Key } from '../i18n/th';

export type StaffCancelReason = 'CUSTOMER_LEFT' | 'CUSTOMER_CHANGED_MIND' | 'DUPLICATE' | 'OTHER';
export type KioskCancelReason = StaffCancelReason | 'CUSTOMER_CANCELLED' | 'DAY_CLOSED';

export const STAFF_CANCEL_REASONS: readonly StaffCancelReason[] = [
  'CUSTOMER_LEFT',
  'CUSTOMER_CHANGED_MIND',
  'DUPLICATE',
  'OTHER',
];

const REASON_KEY: Record<StaffCancelReason, Key> = {
  CUSTOMER_LEFT: 'kioskCancel.reason.customerLeft',
  CUSTOMER_CHANGED_MIND: 'kioskCancel.reason.customerChangedMind',
  DUPLICATE: 'kioskCancel.reason.duplicate',
  OTHER: 'kioskCancel.reason.other',
};

export interface CancelReasonDialogProps {
  open: boolean;
  onConfirm: (choice: { reason: StaffCancelReason; note?: string }) => void;
  onCancel: () => void;
}

export function CancelReasonDialog(props: CancelReasonDialogProps): JSX.Element {
  const [reason, setReason] = createSignal<StaffCancelReason>('CUSTOMER_LEFT');
  const [note, setNote] = createSignal('');

  // Each open starts empty so choices from a previous item never leak into the next (#2248).
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) {
          setReason('CUSTOMER_LEFT');
          setNote('');
        }
      },
    ),
  );

  const trimmedNote = () => note().trim();
  const isValid = () => {
    const len = trimmedNote().length;
    if (len > 200) return false;
    if (reason() === 'OTHER') return len >= 1;
    return true;
  };

  const confirm = () => {
    if (!isValid()) return;
    const trimmed = trimmedNote();
    props.onConfirm(trimmed ? { reason: reason(), note: trimmed } : { reason: reason() });
  };

  return (
    <Show when={props.open}>
      <dialog open class="xc-dialog">
        <div class="xc-dialog-content">
          <h3>{t('kioskCancel.title')}</h3>
          <select
            name="reason"
            value={reason()}
            onChange={(e) => setReason(e.currentTarget.value as StaffCancelReason)}
          >
            <For each={STAFF_CANCEL_REASONS}>
              {(code) => (
                <option value={code} selected={reason() === code}>
                  {t(REASON_KEY[code])}
                </option>
              )}
            </For>
          </select>
          <textarea
            name="note"
            value={note()}
            placeholder={t('kioskCancel.note')}
            onInput={(e) => setNote(e.currentTarget.value)}
          />
          <div class="xc-dialog-actions">
            <button type="button" data-action="dismiss" onClick={() => props.onCancel()}>
              {t('kioskCancel.dismiss')}
            </button>
            <button
              type="button"
              data-action="confirm-cancel-request"
              disabled={!isValid()}
              onClick={confirm}
            >
              {t('kioskCancel.confirm')}
            </button>
          </div>
        </div>
      </dialog>
    </Show>
  );
}
