// §9 TierEditor: 4 tiers buy/sell prefilled from the suggestion, the margin each makes, approve one currency (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { For, Show, createSignal, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type {
  PricingApproveInput,
  PricingApproveOutput,
  PricingTierPriceInput,
} from '../../../shared/api/generated/pricing_approve';
import type { PricingBandDto, PricingRowDto } from '../../../shared/api/generated/pricing_board';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { RATE, checkDecimal } from '../../../shared/ui/bounded';
import { en, th } from './i18n';
import { ROUTE_PRICING_APPROVE, type TierNo } from './pricing-labels';
import './tier-editor.css';

export interface TierEditorProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  row: PricingRowDto;
  bands: readonly PricingBandDto[];
  onApproved(out: PricingApproveOutput): void;
}

const TIER_NOS: readonly TierNo[] = [1, 2, 3, 4];
const tr = screenT('admin.pricing', { th, en });

export function TierEditor(props: TierEditorProps): JSX.Element {
  const [editedBuy, setEditedBuy] = createSignal<Partial<Record<TierNo, string>>>({});
  const [editedSell, setEditedSell] = createSignal<Partial<Record<TierNo, string>>>({});
  const [reason, setReason] = createSignal('');
  const [submitting, setSubmitting] = createSignal(false);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

  const labels = (): Record<string, string> => {
    const out: Record<string, string> = { reason: tr('reason') };
    for (const tierNo of TIER_NOS) {
      const idx = tierNo - 1;
      out[`tiers[${idx}].buy`] = `${tr('tier', { n: tierNo })} ${tr('buy')}`;
      out[`tiers[${idx}].sell`] = `${tr('tier', { n: tierNo })} ${tr('sell')}`;
    }
    return out;
  };

  const localError = (field: string): string =>
    fieldError(
      new ApiError({ status: 400, code: 'validationFailed', field, message: '' }),
      labels(),
    ).message;

  const approveCmd = keyedWrite<PricingApproveInput, PricingApproveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_APPROVE,
  );

  const suggestedBuy = (tierNo: TierNo): string =>
    props.row.suggestion?.find((t) => t.tierNo === tierNo)?.buy ?? '';

  const suggestedSell = (tierNo: TierNo): string =>
    props.row.suggestion?.find((t) => t.tierNo === tierNo)?.sell ?? '';

  const buyValue = (tierNo: TierNo): string => editedBuy()[tierNo] ?? suggestedBuy(tierNo);
  const sellValue = (tierNo: TierNo): string => editedSell()[tierNo] ?? suggestedSell(tierNo);

  const buyMarginText = (tierNo: TierNo): string => {
    if (!props.row.suggestion) return '';
    const edited = editedBuy()[tierNo];
    if (edited !== undefined && edited !== suggestedBuy(tierNo)) {
      return tr('marginOnApprove');
    }
    return props.row.margins.find((m) => m.tierNo === tierNo)?.buyMarginPct ?? '';
  };

  const sellMarginText = (tierNo: TierNo): string => {
    if (!props.row.suggestion) return '';
    const edited = editedSell()[tierNo];
    if (edited !== undefined && edited !== suggestedSell(tierNo)) {
      return tr('marginOnApprove');
    }
    return props.row.margins.find((m) => m.tierNo === tierNo)?.sellMarginPct ?? '';
  };

  const bandText = (tierNo: TierNo): string => {
    const b = props.bands.find((band) => band.tierNo === tierNo);
    if (!b) return '';
    const lower = b.lowerExclusiveThb ?? '0';
    return b.upperInclusiveThb ? `${lower} – ${b.upperInclusiveThb}` : `${lower}+`;
  };

  const handleApprove = async () => {
    if (!props.row.suggestion || submitting()) return;
    setErrorMsg(null);
    setFieldErrors({});

    const nextFieldErrors: Record<string, string> = {};
    const tiers: PricingTierPriceInput[] = TIER_NOS.map((tierNo) => {
      const idx = tierNo - 1;
      const buy = buyValue(tierNo).trim();
      const sell = sellValue(tierNo).trim();
      if (!checkDecimal(buy, RATE).ok) {
        nextFieldErrors[`tiers[${idx}].buy`] = localError(`tiers[${idx}].buy`);
      }
      if (!checkDecimal(sell, RATE).ok) {
        nextFieldErrors[`tiers[${idx}].sell`] = localError(`tiers[${idx}].sell`);
      }
      return { tierNo, buy, sell };
    });
    const trimmedReason = reason().trim();
    if (trimmedReason === '') {
      nextFieldErrors.reason = localError('reason');
    }
    if (Object.keys(nextFieldErrors).length > 0) {
      setFieldErrors(nextFieldErrors);
      return;
    }

    setSubmitting(true);
    try {
      const out = await approveCmd.send({
        policyVersionId: props.policyVersionId,
        rows: [{ currency: props.row.currency, tiers }],
        reason: trimmedReason,
      });
      props.onApproved(out);
    } catch (err: unknown) {
      const fe = fieldError(err, labels());
      if (fe.field) {
        setFieldErrors({ [fe.field]: fe.message });
      } else {
        setErrorMsg(fe.message);
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <section class="tier-editor">
      <header class="tier-editor-header">
        <strong class="tier-editor-currency">{props.row.currency}</strong>
        <Show when={!props.row.suggestion}>
          <span class="tier-editor-empty">{tr('noSuggestion')}</span>
        </Show>
      </header>

      <Show when={errorMsg()}>
        {(msg) => (
          <div role="alert" class="tier-editor-alert">
            {msg()}
          </div>
        )}
      </Show>

      <table class="tier-editor-table">
        <thead>
          <tr>
            <th>{tr('label')}</th>
            <th>{tr('buy')}</th>
            <th>{tr('margin')}</th>
            <th>{tr('sell')}</th>
            <th>{tr('margin')}</th>
          </tr>
        </thead>
        <tbody>
          <For each={TIER_NOS}>
            {(tierNo) => (
              <tr>
                <td>
                  <span>{tr('tier', { n: tierNo })}</span>
                  <Show when={bandText(tierNo)}>
                    <span class="tier-editor-band">{bandText(tierNo)}</span>
                  </Show>
                </td>
                <td>
                  <input
                    class="tier-editor-input"
                    type="text"
                    inputMode="decimal"
                    name={`buy-${tierNo}`}
                    value={buyValue(tierNo)}
                    disabled={!props.row.suggestion}
                    onInput={(e) => {
                      const val = e.currentTarget.value;
                      setEditedBuy((prev) => ({ ...prev, [tierNo]: val }));
                    }}
                  />
                  <Show when={fieldErrors()[`tiers[${tierNo - 1}].buy`]}>
                    {(msg) => <span data-field-error={`tiers[${tierNo - 1}].buy`}>{msg()}</span>}
                  </Show>
                </td>
                <td class="tier-editor-margin" data-margin={`buy-${tierNo}`}>
                  {buyMarginText(tierNo)}
                </td>
                <td>
                  <input
                    class="tier-editor-input"
                    type="text"
                    inputMode="decimal"
                    name={`sell-${tierNo}`}
                    value={sellValue(tierNo)}
                    disabled={!props.row.suggestion}
                    onInput={(e) => {
                      const val = e.currentTarget.value;
                      setEditedSell((prev) => ({ ...prev, [tierNo]: val }));
                    }}
                  />
                  <Show when={fieldErrors()[`tiers[${tierNo - 1}].sell`]}>
                    {(msg) => <span data-field-error={`tiers[${tierNo - 1}].sell`}>{msg()}</span>}
                  </Show>
                </td>
                <td class="tier-editor-margin" data-margin={`sell-${tierNo}`}>
                  {sellMarginText(tierNo)}
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>

      <div class="tier-editor-actions">
        <div>
          <input
            type="text"
            name="reason"
            class="tier-editor-input"
            placeholder={tr('reason')}
            aria-label={tr('reason')}
            required
            disabled={!props.row.suggestion || submitting()}
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
          />
          <Show when={fieldErrors().reason}>
            {(msg) => <span data-field-error="reason">{msg()}</span>}
          </Show>
        </div>
        <button
          type="button"
          name="approve"
          class="tier-editor-approve"
          disabled={!props.row.suggestion || submitting()}
          onClick={() => void handleApprove()}
        >
          {tr('approve')}
        </button>
      </div>
    </section>
  );
}
