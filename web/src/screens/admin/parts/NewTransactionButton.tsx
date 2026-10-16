// V2-parts (#476): new transaction button (ticket §7, design §12.2, P5)

import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
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
  let reading = false;

  createEffect(() => {
    props.branch;
    setReadError(null);
  });

  async function handleClick(): Promise<void> {
    if (pendingWrite.busy() || reading) {
      return;
    }

    reading = true;
    setReadError(null);

    try {
      const res = await props.api.view<{
        shiftOpen: boolean;
        deskUrl: string;
      }>('admin_shift_state', { branch: props.branch });

      if (!res.data?.shiftOpen) {
        toast(t('openShiftFirst'));
        location.hash = '#cash';
      } else {
        const deskUrl = res.data.deskUrl;
        openDialog(t('newTransactionTitle'), () => (
          <>
            <p>{t('newTransactionSentence')}</p>
            <div class="dialog-actions">
              <button type="button" class="secondary" onClick={closeDialog}>
                {t('close')}
              </button>
              <a class="primary" href={deskUrl} target="_blank" rel="noopener">
                {t('openDeskLink')}
              </a>
            </div>
          </>
        ));
      }
    } catch (err: unknown) {
      setReadError(wordedError(err));
    } finally {
      reading = false;
    }
  }

  return (
    <div class="new-transaction-button-wrapper">
      <button
        type="button"
        class="new-transaction-button"
        disabled={pendingWrite.busy()}
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
