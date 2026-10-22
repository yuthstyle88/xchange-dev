import { createEffect, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../../shared/api/client';
import { screenT } from '../../../../shared/i18n';
import { sharedSession } from '../../../../shared/session/session';
import { PermissionsUnavailable, permissionsUnavailableText } from '../../../../shared/ui/PermissionsUnavailable';
import * as texts from './i18n';
import { createRefundsStore, type RefundsStore } from './refunds-store';
import { RefundsStep } from './RefundsStep';
import './refunds.css';

const t = screenT('desk.steps.refunds', texts);

export interface TaskHandle {
  nav(): JSX.Element;
  panel(): JSX.Element;
  isOpen(): boolean;
}

export function createTask<S>(props: { store: S }): TaskHandle {
  const storeAny = props.store as any;
  const path = storeAny?.path;

  // Stub frame store (P7) or STAFF_DESK (O-8): never read me or desk_refunds
  if (path?.kind !== 'owner') {
    return {
      nav: () => null,
      panel: () => null,
      isOpen: () => false,
    };
  }

  const api: ApiClient = storeAny?.api ?? new ApiClient();
  const session = sharedSession();
  const refundsStore: RefundsStore = createRefundsStore(api);

  const permitted = () => session.me()?.permissions?.includes('ReversalRequest') ?? false;
  const unavailableError = () => (session.me() === undefined ? session.meError() : null);

  let readOnce = false;
  createEffect(() => {
    if (!readOnce && permitted()) {
      readOnce = true;
      void refundsStore.readRefunds();
    }
  });

  return {
    nav: () => (
      <>
        <Show when={permitted() || unavailableError() !== null}>
          <button
            type="button"
            data-desk-task="refunds"
            class="desk-task-nav-btn"
            disabled={!permitted()}
            title={unavailableError() ? permissionsUnavailableText(unavailableError()) : ''}
            onClick={() => permitted() && refundsStore.setOpen(true)}
          >
            {t('nav')} ({refundsStore.rows().length})
          </button>
        </Show>
        <PermissionsUnavailable error={unavailableError()} onRetry={() => session.retryMe()} />
      </>
    ),
    panel: () => (
      <Show when={permitted() && refundsStore.open()}>
        <RefundsStep store={refundsStore} />
      </Show>
    ),
    isOpen: () => permitted() && refundsStore.open(),
  };
}
