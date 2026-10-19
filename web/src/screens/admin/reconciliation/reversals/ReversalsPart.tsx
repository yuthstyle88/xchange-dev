// F01 (#1068) the sheet's pending reversals, moved out of Reconciliation.tsx as a pure code move
// (FE-admin skeleton, #564). Survey row A4 adds approve / reject here (executive-finance.md §5).
import { createSignal, For, Show, type JSX } from 'solid-js';
import type { ReversalPendingRow } from '../../../../shared/api/generated/finance_reconciliation';
import { screenT } from '../../../../shared/i18n';
import { Empty, stamp } from '../parts';
import * as table from './i18n';

const t = screenT('admin.reconciliation.reversals', table);

export interface ReversalsPartProps {
  rows: ReversalPendingRow[];
  /** The sheet's branch: the decide command's seam branch. */
  branch: string;
  /** Re-reads the sheet after a decision. */
  onDecided: () => Promise<void>;
}

function ReversalRow(props: {
  row: ReversalPendingRow;
  branch: string;
  onDecided: () => Promise<void>;
}): JSX.Element {
  const [reason, setReason] = createSignal('');
  const [error] = createSignal<string | null>(null);

  return (
    <li data-request={props.row.requestId}>
      <span>
        {props.row.transactionId} {props.row.requestedBy} {stamp(props.row.createdAt)}
      </span>
      <input
        type="text"
        name="reason"
        placeholder={t('reason')}
        value={reason()}
        onInput={(e) => setReason(e.currentTarget.value)}
      />
      <button type="button" data-action="approve">
        {t('approve')}
      </button>
      <button type="button" data-action="reject">
        {t('reject')}
      </button>
      <Show when={error()}>
        <span data-error>{error()}</span>
      </Show>
    </li>
  );
}

export function ReversalsPart(props: ReversalsPartProps): JSX.Element {
  return (
    <Show when={props.rows.length > 0} fallback={<Empty />}>
      <ul>
        <For each={props.rows}>
          {(r) => <ReversalRow row={r} branch={props.branch} onDecided={props.onDecided} />}
        </For>
      </ul>
    </Show>
  );
}
