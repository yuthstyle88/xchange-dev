// §9 BulkApprove: count, confirm, calls pricingApprove with the selected rows (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { Show, createMemo, createSignal, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type {
  PricingApproveInput,
  PricingApproveOutput,
  PricingApproveRowInput,
} from '../../../shared/api/generated/pricing_approve';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import * as table from './i18n';
import { BLOCKING_FLAGS, ROUTE_PRICING_APPROVE, type PricingFlagCode } from './pricing-labels';
import './bulk-approve.css';

const t = screenT('admin.pricing', table);

export interface BulkApproveProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  /** The rows the user selected; rows with a blocking flag are never sent. */
  rows: readonly PricingRowDto[];
  onApproved(out: PricingApproveOutput): void;
}

function canApprove(row: PricingRowDto): boolean {
  return (
    row.suggestion !== null &&
    !row.flags.some((f) => BLOCKING_FLAGS.includes(f as PricingFlagCode))
  );
}

export function BulkApprove(props: BulkApproveProps): JSX.Element {
  const [confirming, setConfirming] = createSignal(false);
  const [reason, setReason] = createSignal('');
  const [reasonError, setReasonError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const write = keyedWrite<PricingApproveInput, PricingApproveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_APPROVE,
  );

  const approvable = createMemo(() => props.rows.filter(canApprove));

  const submit = async () => {
    const list = approvable();
    if (list.length === 0 || busy()) return;
    setError(null);
    setReasonError(null);
    const trimmedReason = reason().trim();
    if (trimmedReason === '') {
      setReasonError(
        fieldError(
          new ApiError({ status: 400, code: 'validationFailed', field: 'reason', message: '' }),
          { reason: t('reason') },
        ).message,
      );
      return;
    }
    setBusy(true);
    try {
      const rows: PricingApproveRowInput[] = list.map((r) => ({
        currency: r.currency,
        tiers: r.suggestion!.map((tier) => ({
          tierNo: tier.tierNo,
          buy: tier.buy ?? '',
          sell: tier.sell ?? '',
        })),
      }));
      const out = await write.send({
        policyVersionId: props.policyVersionId,
        rows,
        reason: trimmedReason,
      });
      setConfirming(false);
      props.onApproved(out);
    } catch (err: unknown) {
      const fe = fieldError(err, { reason: t('reason') });
      if (fe.field === 'reason') {
        setReasonError(fe.message);
      } else {
        setError(fe.message);
      }
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
          setReasonError(null);
          setConfirming(true);
        }}
      >
        {t('bulkApprove')}
      </button>
      <Show when={confirming() && approvable().length > 0}>
        <div class="bulk-approve-confirm">
          <div>
            <input
              type="text"
              name="reason"
              placeholder={t('reason')}
              aria-label={t('reason')}
              required
              disabled={busy()}
              value={reason()}
              onInput={(e) => setReason(e.currentTarget.value)}
            />
            <Show when={reasonError()}>
              {(msg) => <span data-field-error="reason">{msg()}</span>}
            </Show>
          </div>
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
