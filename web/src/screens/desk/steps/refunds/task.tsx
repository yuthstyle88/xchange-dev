import { createSignal, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../../shared/api/client';
import type { StaffMeView } from '../../../../shared/api/generated/staff_me';
import { screenT } from '../../../../shared/i18n';
import { can } from '../../../staff/parts/can';
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

  // Stub frame store (P7) or STAFF_DESK (O-8): never read staff_me or desk_refunds
  if (path?.kind !== 'owner') {
    return {
      nav: () => null,
      panel: () => null,
      isOpen: () => false,
    };
  }

  const api: ApiClient = storeAny?.api ?? new ApiClient();
  const [visible, setVisible] = createSignal(false);
  const refundsStore: RefundsStore = createRefundsStore(api);

  api
    .view<StaffMeView>('staff_me')
    .then((res) => {
      if (can(res.data, 'ReversalRequest')) {
        setVisible(true);
        void refundsStore.readRefunds();
      }
    })
    .catch(() => {
      // staff_me failed or 404: step stays hidden, desk_refunds is never called
    });

  return {
    nav: () => (
      <Show when={visible()}>
        <button
          type="button"
          data-desk-task="refunds"
          class="desk-task-nav-btn"
          onClick={() => refundsStore.setOpen(true)}
        >
          {t('nav')} ({refundsStore.rows().length})
        </button>
      </Show>
    ),
    panel: () => (
      <Show when={visible() && refundsStore.open()}>
        <RefundsStep store={refundsStore} />
      </Show>
    ),
    isOpen: () => visible() && refundsStore.open(),
  };
}
