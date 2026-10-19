// F01 (#1068) the sheet's transfers part, moved out of Reconciliation.tsx as a pure code move
// (FE-admin skeleton, #564). Survey row A5 adds the way into reconcile here.
import { For, type JSX } from 'solid-js';
import type { TransferSummary } from '../../../../shared/api/generated/finance_reconciliation';

export function TransfersPart(props: { data: TransferSummary }): JSX.Element {
  return (
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
  );
}
