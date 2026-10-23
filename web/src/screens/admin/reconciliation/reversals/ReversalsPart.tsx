// F01 (#1068) the sheet's pending reversals, moved out of Reconciliation.tsx as a pure code move
// (FE-admin skeleton, #564). Survey row A4 adds approve / reject here (executive-finance.md §5).
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../../shared/api/client';
import type { ReversalPendingRow } from '../../../../shared/api/generated/finance_reconciliation';
import { screenT } from '../../../../shared/i18n';
import { fieldError } from '../../../../shared/i18n/field-error';
import { decideRefusal, decideWrite, type DecideOutcome } from '../../parts/decide';
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
  const api = new ApiClient();
  const writer = decideWrite(api, props.row.requestId);

  const [reason, setReason] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function decide(outcome: DecideOutcome) {
    if (busy()) return;
    const text = reason();
    const refusal = decideRefusal(outcome, text);
    if (refusal === 'reasonRequired') {
      setError(t('reasonRequired'));
      return;
    }
    if (refusal === 'reasonTooLong') {
      setError(t('reasonTooLong'));
      return;
    }
    setError(null);
    setBusy(true);
    try {
      await writer.send({
        outcome,
        reason: text,
        branch: props.branch,
      });
      await props.onDecided();
    } catch (err: unknown) {
      setError(fieldError(err, { reason: t('reason') }).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <li data-request={props.row.requestId}>
      <span>
        {props.row.requestId} {props.row.transactionId} {props.row.requestedBy} {stamp(props.row.createdAt)}
      </span>
      <input
        type="text"
        name="reason"
        placeholder={t('reason')}
        maxLength={500}
        value={reason()}
        onInput={(e) => setReason(e.currentTarget.value)}
      />
      <div>{reason().length} / 500</div>
      <button
        type="button"
        data-action="approve"
        disabled={busy()}
        onClick={() => void decide('APPROVED')}
      >
        {t('approve')}
      </button>
      <button
        type="button"
        data-action="reject"
        disabled={busy()}
        onClick={() => void decide('REJECTED')}
      >
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
