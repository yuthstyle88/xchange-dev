// V2-parts (#476): new transaction button (ticket §7, design §12.2, P5)

import { createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { closeDialog, openDialog } from './Dialog';
import { en, th } from './i18n';
import { pendingWrite } from './pending-write';
import { toast } from './toast';
import './parts.css';

export interface NewTransactionButtonProps {
  api: ApiClient;
  branch: string;
}

export function NewTransactionButton(
  props: NewTransactionButtonProps,
): JSX.Element {
  const t = screenT('admin.parts', { th, en });
  const [readError, setReadError] = createSignal<string | null>(null);

  async function handleClick(): Promise<void> {
    if (pendingWrite.busy()) {
      return;
    }

    try {
      const res = await props.api.view<{
        shiftOpen: boolean;
        deskUrl: string;
      }>('admin_shift_state', { branch: props.branch });

      setReadError(null);

      if (!res.data?.shiftOpen) {
        toast(t('openShiftFirst'));
        location.hash = '#cash';
      } else {
        const deskUrl = res.data.deskUrl;
        openDialog(t('newTransactionTitle'), () => (
          <div class="new-transaction-dialog">
            <p>{t('newTransactionSentence')}</p>
            <div class="dialog-actions">
              <a href={deskUrl} target="_blank" rel="noopener">
                {t('openDeskLink')}
              </a>
              <button type="button" onClick={closeDialog}>
                {t('close')}
              </button>
            </div>
          </div>
        ));
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setReadError(msg);
    }
  }

  return (
    <div class="new-transaction-button-wrapper">
      <button
        type="button"
        class="new-transaction-button"
        disabled={pendingWrite.busy() || readError() !== null}
        onClick={() => void handleClick()}
      >
        {t('newTransactionBtn')}
      </button>
      <Show when={readError()}>
        {(err) => (
          <span class="read-error" role="alert">
            {err()}
          </span>
        )}
      </Show>
    </div>
  );
}
