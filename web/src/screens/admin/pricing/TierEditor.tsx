// §9 TierEditor: 4 tiers buy/sell prefilled from the suggestion, the margin each makes, approve one currency (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { For, Show, createSignal, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { en, th } from './i18n';
import {
  ROUTE_PRICING_APPROVE,
  type PricingApproveInput,
  type PricingApproveOutput,
  type PricingRowView,
  type TierBandView,
  type TierNo,
  type TierPriceView,
} from './pricing-board.types';
import './tier-editor.css';

export interface TierEditorProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  row: PricingRowView;
  bands: readonly TierBandView[];
  onApproved(out: PricingApproveOutput): void;
}

const TIER_NOS: readonly TierNo[] = [1, 2, 3, 4];
const tr = screenT('admin.pricing', { th, en });

export function TierEditor(props: TierEditorProps): JSX.Element {
  const [editedBuy, setEditedBuy] = createSignal<Partial<Record<TierNo, string>>>({});
  const [editedSell, setEditedSell] = createSignal<Partial<Record<TierNo, string>>>({});
  const [submitting, setSubmitting] = createSignal(false);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);

  const approveCmd = keyedWrite<PricingApproveInput, PricingApproveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_APPROVE,
  );
  let lastPayloadKey = '';
  let lastRequestId = '';

  const suggestedBuy = (tierNo: TierNo): string =>
    props.row.suggestion?.tiers.find((t) => t.tierNo === tierNo)?.buy ?? '';

  const suggestedSell = (tierNo: TierNo): string =>
    props.row.suggestion?.tiers.find((t) => t.tierNo === tierNo)?.sell ?? '';

  const buyValue = (tierNo: TierNo): string => editedBuy()[tierNo] ?? suggestedBuy(tierNo);
  const sellValue = (tierNo: TierNo): string => editedSell()[tierNo] ?? suggestedSell(tierNo);

  const buyMarginText = (tierNo: TierNo): string => {
    if (!props.row.suggestion) return '';
    const edited = editedBuy()[tierNo];
    if (edited !== undefined && edited !== suggestedBuy(tierNo)) {
      return tr('marginOnApprove');
    }
    return props.row.suggestion.margins.find((m) => m.tierNo === tierNo)?.buyMarginPct ?? '';
  };

  const sellMarginText = (tierNo: TierNo): string => {
    if (!props.row.suggestion) return '';
    const edited = editedSell()[tierNo];
    if (edited !== undefined && edited !== suggestedSell(tierNo)) {
      return tr('marginOnApprove');
    }
    return props.row.suggestion.margins.find((m) => m.tierNo === tierNo)?.sellMarginPct ?? '';
  };

  const bandText = (tierNo: TierNo): string => {
    const b = props.bands.find((band) => band.tierNo === tierNo);
    if (!b) return '';
    return b.upperInclusiveThb ? `${b.lowerThb} – ${b.upperInclusiveThb}` : `${b.lowerThb}+`;
  };

  const handleApprove = async () => {
    if (!props.row.suggestion || submitting()) return;
    setErrorMsg(null);
    setSubmitting(true);

    const tiers: TierPriceView[] = TIER_NOS.map((tierNo) => ({
      tierNo,
      buy: buyValue(tierNo),
      sell: sellValue(tierNo),
    }));
    const payloadKey = JSON.stringify({
      branch: props.branch,
      policyVersionId: props.policyVersionId,
      currency: props.row.currency,
      tiers,
    });
    if (payloadKey !== lastPayloadKey || !lastRequestId) {
      lastPayloadKey = payloadKey;
      lastRequestId = crypto.randomUUID();
    }

    try {
      const out = await approveCmd.send({
        branch: props.branch,
        requestId: lastRequestId,
        policyVersionId: props.policyVersionId,
        currencies: [{ currency: props.row.currency, tiers }],
      });
      props.onApproved(out);
    } catch (err: unknown) {
      setErrorMsg(wordedError(err));
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

