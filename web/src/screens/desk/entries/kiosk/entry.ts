// B03 the desk's `?kiosk=<id>` entry (FE-desk skeleton stub, #564; design xchange-v3-wave3-desk.md §3.6 row K1, §5.3
// `DeskEntryState`, §6 `entries/<name>/entry.ts` found by `entries.ts`'s glob). Pre-fill and identity only: the receipt
// publish half waits on O-1 (`xchange-v3-wave3-desk.md:809`), and completing the request needs its own quote (G7,
// api/src/application/workflows/exchange/create.rs:228-244), which `/api/kiosk get` does not return yet. Filled by its
// own ticket.
import { createComponent, createEffect, createSignal, type JSX } from 'solid-js';
import { cancelKioskRequest } from '../../../../shared/api/kiosk-cancel';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import { CancelReasonDialog } from '../../../../shared/ui/CancelReasonDialog';
import type { DeskType } from '../../desk-store';
import type { DeskEntry } from '../../entries';
import { en, th } from './i18n';

const t = screenT('desk.entries.kiosk', { th, en });

interface KioskGetAnswer {
  paymentMethod: string;
  type: DeskType;
  currency: string;
  foreign: number;
  queue: string;
}

export const entry: DeskEntry = {
  param: 'kiosk',
  async load(store, id, api) {
    const answer = await api.post<
      { action: 'get'; payload: { id: string; branchId: string } },
      KioskGetAnswer
    >('/api/kiosk', {
      action: 'get',
      payload: { id, branchId: store.branch() },
    });

    store.setType(answer.type);
    store.setCurrency(answer.currency);
    store.setAmount(String(answer.foreign));
    store.setMethod(answer.paymentMethod);

    const [identityConfirmed, setIdentityConfirmed] = createSignal(false);

    return {
      kind: 'kiosk',
      id,
      locked: true,
      banner: () => t('banner', { queue: answer.queue }),
      actions: () => {
        const [dialogOpen, setDialogOpen] = createSignal(false);
        const [error, setError] = createSignal<string | undefined>();

        const btn = document.createElement('button');
        btn.type = 'button';
        btn.dataset.action = 'cancel-kiosk-request';
        createEffect(() => {
          btn.textContent = t('cancelRequest');
        });
        btn.addEventListener('click', () => {
          setError(undefined);
          setDialogOpen(true);
        });

        return [
          (() => (store.outcome() !== 'saved' ? btn : null)) as unknown as JSX.Element,
          (() => {
            const msg = error();
            if (!msg) return null;
            const alertEl = document.createElement('p');
            alertEl.setAttribute('role', 'alert');
            alertEl.className = 'desk-error';
            alertEl.textContent = msg;
            return alertEl;
          }) as unknown as JSX.Element,
          createComponent(CancelReasonDialog, {
            get open() {
              return dialogOpen();
            },
            onCancel: () => setDialogOpen(false),
            onConfirm: async ({ reason, note }) => {
              setDialogOpen(false);
              setError(undefined);
              try {
                await cancelKioskRequest(api, {
                  id,
                  branchId: store.branch(),
                  reason,
                  ...(note !== undefined ? { note } : {}),
                });
                window.location.assign(`/desk?branch=${store.branch()}`);
              } catch (err) {
                setError(wordedError(err));
              }
            },
          }),
        ];
      },
      identityLabel: t('identityLabel'),
      identityConfirmed,
      setIdentityConfirmed,
      payloadExtras: () => ({ kioskRequestId: id }),
      beforeReserve: async (s) => {
        await api.post('/api/kiosk', {
          action: 'identityChecked',
          payload: { id, branchId: s.branch(), confirmed: s.identityChecked() },
        });
      },
      refusal: () => (identityConfirmed() ? undefined : t('identityRequired')),
    };
  },
};
