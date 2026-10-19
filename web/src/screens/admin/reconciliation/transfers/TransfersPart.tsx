// F01 (#1068) the sheet's transfers part, moved out of Reconciliation.tsx as a pure code move
// (FE-admin skeleton, #564). Survey row A5 adds the way into reconcile here.
import { For, Show, type JSX } from 'solid-js';
import type { TransferSummary } from '../../../../shared/api/generated/finance_reconciliation';
import { screenT } from '../../../../shared/i18n';
import * as table from './i18n';

const t = screenT('admin.reconciliation.transfers', table);

export function TransfersPart(props: { data: TransferSummary }): JSX.Element {
  const canReconcile = () =>
    props.data.byStatus.some((b) => b.status === 'RECEIVED' && b.count > 0) ||
    props.data.shortReceipts.length > 0;

  return (
    <>
      <ul>
        <For each={props.data.byStatus}>
          {(b) => (
            <li>
              {b.status}: {b.count}
            </li>
          )}
        </For>
        <For each={props.data.inTransit}>
          {(i) => (
            <li>
              {i.transferId} {i.currency} {i.quantity}
            </li>
          )}
        </For>
        <For each={props.data.shortReceipts}>
          {(r) => (
            <li>
              {r.transferId} {r.currency} {r.sent} → {r.received}
            </li>
          )}
        </For>
      </ul>
      <Show when={canReconcile()}>
        <a data-action="reconcile" href="#cash">
          {t('reconcileInCash')}
        </a>
      </Show>
    </>
  );
}
