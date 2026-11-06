// §9 TierEditor: 4 tier blocks ("ระดับ 1 ไม่เกิน 10,000 บาท", ร้านรับซื้อ / ร้านขาย, "แนะนำ x / y") like the shop board,
// buy/sell prefilled from the suggestion, the margin each makes, approve one currency (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
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
import { ApproveResults } from './ApproveResults';
import { ROUTE_PRICING_APPROVE, type TierNo } from './pricing-labels';
import { bandText, rateAt, rowDigits } from './pricing-format';
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
  const [approveOut, setApproveOut] = createSignal<PricingApproveOutput | null>(null);

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

  const digits = () => rowDigits(props.row);
  /** The prefilled box shows the suggestion without the server's zero padding (same number); an untouched box still
   *  sends the suggestion exactly as the board sent it (`buyValue` / `sellValue`). */
  const shown = (v: string): string => (v === '' ? '' : rateAt(v, digits()));
  const bandLine = (tierNo: TierNo): string =>
    bandText(
      props.bands.find((band) => band.tierNo === tierNo),
      {
        upTo: (upper) => tr('bandUpTo', { upper }),
        between: (lower, upper) => tr('bandBetween', { lower, upper }),
        above: (lower) => tr('bandAbove', { lower }),
      },
    );

  const handleApprove = async () => {
    if (!props.row.suggestion || submitting()) return;
    setErrorMsg(null);
    setFieldErrors({});
    setApproveOut(null);

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
        rows: [
          {
            currency: props.row.currency,
            tiers,
            ...(props.row.tierSet ? { tierSetId: props.row.tierSet.id } : {}),
          },
        ],
        reason: trimmedReason,
      });
      setApproveOut(out);
      props.onApproved(out);
    } catch (err: unknown) {
      if (err instanceof ApiError && err.code === 'staleVersion') {
        setErrorMsg(tr('approveStale'));
        return;
      }
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

  const fieldId = (side: 'buy' | 'sell', tierNo: TierNo) => `tier-${props.row.currency}-${side}-${tierNo}`;

  return (
    <section class="tier-editor" aria-label={`${tr('setPrice')} ${props.row.currency}`}>
      <Show when={!props.row.suggestion}>
        <p class="tier-editor-empty">{tr('noSuggestion')}</p>
      </Show>

      <Show when={errorMsg()}>
        {(msg) => (
          <div role="alert" class="tier-editor-alert">
            {msg()}
          </div>
        )}
      </Show>

      <div class="tier-editor-tiers">
        <For each={TIER_NOS}>
          {(tierNo) => (
            <fieldset class="tier-editor-tier" data-tier={tierNo}>
              <legend class="tier-editor-tier-title">
                <strong>{tr('tierLevel', { n: tierNo })}</strong>
                <Show when={bandLine(tierNo)}>
                  <span class="tier-editor-band">{bandLine(tierNo)}</span>
                </Show>
              </legend>
              <div class="tier-editor-fields">
                <div class="tier-editor-field">
                  <label class="tier-editor-label" for={fieldId('buy', tierNo)}>
                    {tr('shopBuy')}
                  </label>
                  <input
                    id={fieldId('buy', tierNo)}
                    class="tier-editor-input"
                    type="text"
                    inputMode="decimal"
                    autocomplete="off"
                    name={`buy-${tierNo}`}
                    value={editedBuy()[tierNo] ?? shown(suggestedBuy(tierNo))}
                    disabled={!props.row.suggestion}
                    aria-invalid={fieldErrors()[`tiers[${tierNo - 1}].buy`] ? 'true' : undefined}
                    onInput={(e) => {
                      const val = e.currentTarget.value;
                      setEditedBuy((prev) => ({ ...prev, [tierNo]: val }));
                    }}
                  />
                  <Show when={fieldErrors()[`tiers[${tierNo - 1}].buy`]}>
                    {(msg) => (
                      <span class="tier-editor-error" data-field-error={`tiers[${tierNo - 1}].buy`}>
                        {msg()}
                      </span>
                    )}
                  </Show>
                </div>
                <div class="tier-editor-field">
                  <label class="tier-editor-label" for={fieldId('sell', tierNo)}>
                    {tr('shopSell')}
                  </label>
                  <input
                    id={fieldId('sell', tierNo)}
                    class="tier-editor-input"
                    type="text"
                    inputMode="decimal"
                    autocomplete="off"
                    name={`sell-${tierNo}`}
                    value={editedSell()[tierNo] ?? shown(suggestedSell(tierNo))}
                    disabled={!props.row.suggestion}
                    aria-invalid={fieldErrors()[`tiers[${tierNo - 1}].sell`] ? 'true' : undefined}
                    onInput={(e) => {
                      const val = e.currentTarget.value;
                      setEditedSell((prev) => ({ ...prev, [tierNo]: val }));
                    }}
                  />
                  <Show when={fieldErrors()[`tiers[${tierNo - 1}].sell`]}>
                    {(msg) => (
                      <span class="tier-editor-error" data-field-error={`tiers[${tierNo - 1}].sell`}>
                        {msg()}
                      </span>
                    )}
                  </Show>
                </div>
              </div>
              <div class="tier-editor-hints">
                <Show when={props.row.suggestion}>
                  <span class="tier-editor-hint">
                    {tr('suggestedHint')}{' '}
                    <span class="tier-editor-hint-rates">
                      {rateAt(suggestedBuy(tierNo), digits())}
                      <span class="pricing-rate-sep" aria-hidden="true">
                        {' / '}
                      </span>
                      {rateAt(suggestedSell(tierNo), digits())}
                    </span>
                  </span>
                </Show>
                <span class="tier-editor-hint tier-editor-margins">
                  {tr('marginHint')}{' '}
                  <span class="tier-editor-margin" data-margin={`buy-${tierNo}`}>
                    {buyMarginText(tierNo)}
                  </span>
                  <span class="pricing-rate-sep" aria-hidden="true">
                    {' / '}
                  </span>
                  <span class="tier-editor-margin" data-margin={`sell-${tierNo}`}>
                    {sellMarginText(tierNo)}
                  </span>
                </span>
              </div>
            </fieldset>
          )}
        </For>
      </div>

      <div class="tier-editor-actions">
        <div class="tier-editor-field tier-editor-reason">
          <label class="tier-editor-label" for={`tier-${props.row.currency}-reason`}>
            {tr('reason')}
          </label>
          <input
            id={`tier-${props.row.currency}-reason`}
            type="text"
            name="reason"
            class="tier-editor-input tier-editor-input-text"
            placeholder={tr('reason')}
            aria-label={tr('reason')}
            required
            disabled={!props.row.suggestion || submitting()}
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
          />
          <Show when={fieldErrors().reason}>
            {(msg) => (
              <span class="tier-editor-error" data-field-error="reason">
                {msg()}
              </span>
            )}
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

      <Show when={approveOut()}>{(out) => <ApproveResults out={out()} />}</Show>
    </section>
  );
}
