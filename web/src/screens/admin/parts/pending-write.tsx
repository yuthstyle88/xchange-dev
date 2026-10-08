// V2-parts (#476): pending write manager with retry, inspect and abandon (ticket §7, design §12.2, P4)

import { createSignal } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { closeDialog, openDialog, setDialogLocked } from './Dialog';
import { en, th } from './i18n';
import { requestRefresh } from './refresh';
import { toast } from './toast';

interface PendingRequest {
  api: ApiClient;
  branch: string;
  action: string;
  payload: Record<string, unknown>;
  onSaved: () => void;
}

const [busy, setBusy] = createSignal(false);
let currentPending: PendingRequest | null = null;
let beforeUnloadActive = false;

function beforeUnloadHandler(e: BeforeUnloadEvent): void {
  e.preventDefault();
  e.returnValue = '';
}

function enableBeforeUnload(): void {
  if (!beforeUnloadActive && typeof window !== 'undefined') {
    window.addEventListener('beforeunload', beforeUnloadHandler);
    beforeUnloadActive = true;
  }
}

function disableBeforeUnload(): void {
  if (beforeUnloadActive && typeof window !== 'undefined') {
    window.removeEventListener('beforeunload', beforeUnloadHandler);
    beforeUnloadActive = false;
  }
}

function disableDialogControls(): void {
  if (typeof document !== 'undefined') {
    const dialogEl = document.querySelector('dialog');
    if (dialogEl) {
      const controls = dialogEl.querySelectorAll<
        HTMLInputElement | HTMLSelectElement | HTMLButtonElement
      >('form input, form select, form button');
      for (const c of controls) {
        c.disabled = true;
      }
    }
  }
}

function showNotConfirmedDialog(errorMessage: string): void {
  const t = screenT('admin.parts', { th, en });
  openDialog(t('notConfirmed'), () => (
    <div class="pending-write-dialog">
      <p class="error-message">{errorMessage}</p>
      <p class="note-message">{t('notConfirmedNote')}</p>
      <div class="dialog-actions">
        <button type="button" onClick={() => void pendingWrite.retry()}>
          {t('retry')}
        </button>
        <button type="button" onClick={() => void pendingWrite.inspect()}>
          {t('inspect')}
        </button>
      </div>
    </div>
  ));
  setDialogLocked(true);
}

function showNotFoundDialog(): void {
  const t = screenT('admin.parts', { th, en });
  openDialog(t('notFoundInServer'), () => (
    <div class="pending-write-dialog">
      <p class="note-message">{t('notFoundInServerText')}</p>
      <div class="dialog-actions">
        <button type="button" onClick={() => void pendingWrite.retry()}>
          {t('retry')}
        </button>
        <button type="button" onClick={() => void pendingWrite.abandon()}>
          {t('discard')}
        </button>
      </div>
    </div>
  ));
  setDialogLocked(true);
}

export const pendingWrite = {
  busy(): boolean {
    return busy();
  },

  async run(
    api: ApiClient,
    branch: string,
    action: string,
    payload: Record<string, unknown>,
    onSaved: () => void,
  ): Promise<void> {
    if (busy() || currentPending !== null) {
      return;
    }

    const requestId = crypto.randomUUID();
    const fullPayload = {
      ...payload,
      branchId: branch,
      requestId,
    };

    const req: PendingRequest = {
      api,
      branch,
      action,
      payload: fullPayload,
      onSaved,
    };
    currentPending = req;
    enableBeforeUnload();
    setBusy(true);
    setDialogLocked(true);
    disableDialogControls();

    const t = screenT('admin.parts', { th, en });

    try {
      await api.post('/api/rpc', { action, payload: fullPayload });
      setBusy(false);
      currentPending = null;
      setDialogLocked(false);
      disableBeforeUnload();
      closeDialog();
      toast(t('serverSaved'));
      requestRefresh();
      onSaved();
    } catch (err: unknown) {
      setBusy(false);
      enableBeforeUnload();
      setDialogLocked(true);
      const msg = err instanceof Error ? err.message : String(err);
      showNotConfirmedDialog(msg);
    }
  },

  async retry(): Promise<void> {
    if (!currentPending || busy()) {
      return;
    }

    setBusy(true);
    setDialogLocked(true);
    disableDialogControls();

    const req = currentPending;
    const t = screenT('admin.parts', { th, en });

    try {
      await req.api.post('/api/rpc', {
        action: req.action,
        payload: req.payload,
      });
      setBusy(false);
      currentPending = null;
      setDialogLocked(false);
      disableBeforeUnload();
      closeDialog();
      toast(t('serverSaved'));
      requestRefresh();
      req.onSaved();
    } catch (err: unknown) {
      setBusy(false);
      enableBeforeUnload();
      setDialogLocked(true);
      const msg = err instanceof Error ? err.message : String(err);
      showNotConfirmedDialog(msg);
    }
  },

  async inspect(): Promise<void> {
    if (!currentPending || busy()) {
      return;
    }

    setBusy(true);
    const req = currentPending;
    const t = screenT('admin.parts', { th, en });

    try {
      const answer = await req.api.view<{ applied: boolean }>(
        'admin_request_status',
        {
          branch: req.branch,
          filter: { requestId: req.payload.requestId as string },
        },
      );
      setBusy(false);

      if (answer.data?.applied) {
        currentPending = null;
        setDialogLocked(false);
        disableBeforeUnload();
        closeDialog();
        toast(t('foundSaved'));
        requestRefresh();
      } else {
        showNotFoundDialog();
      }
    } catch (err: unknown) {
      setBusy(false);
      const msg = err instanceof Error ? err.message : String(err);
      toast(msg);
    }
  },

  async abandon(): Promise<void> {
    if (!currentPending) {
      return;
    }

    const req = currentPending;
    const t = screenT('admin.parts', { th, en });

    try {
      const res = await req.api.post<
        unknown,
        { requestResolution?: string }
      >('/api/rpc', {
        action: 'adminAbandonRequest',
        payload: {
          requestId: req.payload.requestId,
          branchId: req.branch,
        },
      });

      currentPending = null;
      setDialogLocked(false);
      disableBeforeUnload();
      closeDialog();

      if (res.requestResolution === 'APPLIED') {
        toast(t('appliedToast'));
      } else {
        toast(t('cancelledToast'));
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      toast(msg);
    }
  },
};
