// F01 (#1068) the sheet's pending reversals, moved out of Reconciliation.tsx as a pure code move
// (FE-admin skeleton, #564). Survey row A4 adds approve / reject here (executive-finance.md §5).
import { For, Show, type JSX } from 'solid-js';
import type { ReversalPendingRow } from '../../../../shared/api/generated/finance_reconciliation';
import { Empty, stamp } from '../parts';

export interface ReversalsPartProps {
  rows: ReversalPendingRow[];
  /** The sheet's branch: the decide command's seam branch. */
  branch: string;
  /** Re-reads the sheet after a decision. */
  onDecided: () => Promise<void>;
}

export function ReversalsPart(props: ReversalsPartProps): JSX.Element {
  return (
    <Show when={props.rows.length > 0} fallback={<Empty />}>
      <ul>
        <For each={props.rows}>
          {(r) => (
            <li>
              {r.requestId} {r.transactionId} {r.requestedBy} {stamp(r.createdAt)}
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}
