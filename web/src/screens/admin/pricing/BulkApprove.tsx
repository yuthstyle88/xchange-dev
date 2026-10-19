// §9 BulkApprove: count, confirm, calls pricingApprove with the selected rows (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { Show, createMemo, createSignal, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import {
  BLOCKING_FLAGS,
  ROUTE_PRICING_APPROVE,
  type PricingApproveInput,
  type PricingApproveOutput,
  type PricingRowView,
} from './pricing-board.types';
import './bulk-approve.css';

const t = screenT('admin.pricing', table);

export interface BulkApproveProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  /** The rows the user selected; rows with a blocking flag are never sent. */
  rows: readonly PricingRowView[];
  onApproved(out: PricingApproveOutput): void;
}

function canApprove(row: PricingRowView): boolean {
  return row.suggestion !== null && !row.flags.some((f) => BLOCKING_FLAGS.includes(f));
}

export function BulkApprove(props: BulkApproveProps): JSX.Element {
  const [confirming, setConfirming] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const write = keyedWrite<PricingApproveInput, PricingApproveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_APPROVE,
  );

  const approvable = createMemo(() => props.rows.filter(canApprove));
  // One requestId per distinct payload: a retry of the same body reuses it; any change (other rows, new suggested
  // prices after a market refresh, another policy version) gets a new one (design §6: requestId is the idempotency key).
  let lastPayload = '';
  let lastRequestId = '';

  const submit = async () => {
    const list = approvable();
    if (list.length === 0 || busy()) return;
    setError(null);
    setBusy(true);
    try {
      const currencies = list.map((r) => ({
        currency: r.currency,
        tiers: r.suggestion!.tiers,
      }));
      const payload = JSON.stringify({ branch: props.branch, policyVersionId: props.policyVersionId, currencies });
      if (payload !== lastPayload || lastRequestId === '') {
        lastPayload = payload;
        lastRequestId = crypto.randomUUID();
      }
      const out = await write.send({
        branch: props.branch,
        requestId: lastRequestId,
        policyVersionId: props.policyVersionId,
        currencies,
      });
      setConfirming(false);
      props.onApproved(out);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="bulk-approve">
      <span class="bulk-approve-count" data-count={String(approvable().length)}>
        {t('bulkCount', { count: approvable().length })}
      </span>
      <button
        type="button"
        name="bulkApprove"
        class="bulk-approve-btn"
        disabled={approvable().length === 0 || busy()}
        onClick={() => {
          if (approvable().length === 0) return;
          setError(null);
          setConfirming(true);
        }}
      >
        {t('bulkApprove')}
      </button>
      <Show when={confirming() && approvable().length > 0}>
        <div class="bulk-approve-confirm">
          <button
            type="button"
            name="confirmBulk"
            class="bulk-approve-btn"
            disabled={busy()}
            onClick={() => void submit()}
          >
            {t('confirmBulk')}
          </button>
          <button
            type="button"
            name="cancel"
            class="bulk-approve-cancel"
            disabled={busy()}
            onClick={() => setConfirming(false)}
          >
            {t('cancel')}
          </button>
        </div>
      </Show>
      <Show when={error()}>
        {(msg) => (
          <p class="bulk-approve-error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
    </div>
  );
}
