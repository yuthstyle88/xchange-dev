// §9 PricingPolicy: one currency's override margins and min spread (pricingConfigure) (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import type {
  PricingConfigureInput,
  PricingConfigureOutput,
} from '../../../shared/api/generated/pricing_configure';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { RATE_PERCENT, atMost, checkDecimal, type Bound } from '../../../shared/ui/bounded';
import * as table from './i18n';
import { ROUTE_PRICING_CONFIGURE, type TierNo } from './pricing-labels';
import './pricing-policy.css';

export interface PricingPolicyProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  row: PricingRowDto;
  /** EXECUTIVE frame (pricing-tiers.md §3: override margins are frame); the editor is hidden otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: PricingConfigureOutput): void;
}

const TIERS: readonly TierNo[] = [1, 2, 3, 4];
const MIN_SPREAD_BOUND: Bound = { ...atMost(RATE_PERCENT, '5'), sign: 'nonNegative' };

export function PricingPolicy(props: PricingPolicyProps): JSX.Element {
  const t = screenT('admin.pricing', table);
  const configureWrite = keyedWrite<PricingConfigureInput, PricingConfigureOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_CONFIGURE,
  );

  const getMargin = (tierNo: TierNo, side: 'buy' | 'sell') => {
    const m = props.row.margins.find((item) => item.tierNo === tierNo);
    return side === 'buy' ? (m?.buyMarginPct ?? '') : (m?.sellMarginPct ?? '');
  };

  const [margins, setMargins] = createSignal<Record<string, string>>({
    'buy-1': getMargin(1, 'buy'),
    'sell-1': getMargin(1, 'sell'),
    'buy-2': getMargin(2, 'buy'),
    'sell-2': getMargin(2, 'sell'),
    'buy-3': getMargin(3, 'buy'),
    'sell-3': getMargin(3, 'sell'),
    'buy-4': getMargin(4, 'buy'),
    'sell-4': getMargin(4, 'sell'),
  });

  const [minSpread, setMinSpread] = createSignal(props.row.minSpreadPct ?? '');
  const [reason, setReason] = createSignal('');
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

  const labels = (): Record<string, string> => {
    const out: Record<string, string> = {
      reason: t('reason'),
      minSpreadPct: t('minSpread'),
    };
    for (const tierNo of TIERS) {
      out[`margins.buy-${tierNo}`] = `${t('tier', { n: tierNo })} ${t('buy')} (${t('margin')})`;
      out[`margins.sell-${tierNo}`] = `${t('tier', { n: tierNo })} ${t('sell')} (${t('margin')})`;
    }
    return out;
  };

  const localError = (field: string): string =>
    fieldError(
      new ApiError({ status: 400, code: 'validationFailed', field, message: '' }),
      labels(),
    ).message;

  const applyError = (err: unknown) => {
    const fe = fieldError(err, labels());
    if (fe.field) {
      setFieldErrors({ [fe.field]: fe.message });
    } else {
      setError(fe.message);
    }
  };

  createEffect(() => {
    setMargins({
      'buy-1': getMargin(1, 'buy'),
      'sell-1': getMargin(1, 'sell'),
      'buy-2': getMargin(2, 'buy'),
      'sell-2': getMargin(2, 'sell'),
      'buy-3': getMargin(3, 'buy'),
      'sell-3': getMargin(3, 'sell'),
      'buy-4': getMargin(4, 'buy'),
      'sell-4': getMargin(4, 'sell'),
    });
    setMinSpread(props.row.minSpreadPct ?? '');
    setReason('');
    setError(null);
    setFieldErrors({});
  });

  const updateMargin = (key: string, value: string) => {
    setMargins((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async (e: Event) => {
    e.preventDefault();
    if (!props.mayEdit || saving()) return;
    setError(null);
    setFieldErrors({});

    const cur = margins();
    const nextFieldErrors: Record<string, string> = {};
    const tiers = TIERS.map((n) => {
      const buyMarginPct = (cur[`buy-${n}`] ?? '').trim();
      const sellMarginPct = (cur[`sell-${n}`] ?? '').trim();
      if (!checkDecimal(buyMarginPct, RATE_PERCENT).ok) {
        nextFieldErrors[`margins.buy-${n}`] = localError(`margins.buy-${n}`);
      }
      if (!checkDecimal(sellMarginPct, RATE_PERCENT).ok) {
        nextFieldErrors[`margins.sell-${n}`] = localError(`margins.sell-${n}`);
      }
      return { tierNo: n, buyMarginPct, sellMarginPct };
    });

    const trimmedSpread = minSpread().trim();
    if (trimmedSpread !== '' && !checkDecimal(trimmedSpread, MIN_SPREAD_BOUND).ok) {
      nextFieldErrors.minSpreadPct = localError('minSpreadPct');
    }

    const trimmedReason = reason().trim();
    if (trimmedReason === '') {
      nextFieldErrors.reason = localError('reason');
    }

    if (Object.keys(nextFieldErrors).length > 0) {
      setFieldErrors(nextFieldErrors);
      return;
    }

    setSaving(true);
    try {
      const payload: PricingConfigureInput = {
        policyVersionId: props.policyVersionId,
        currency: props.row.currency,
        tiers,
        minSpreadPct: trimmedSpread === '' ? null : trimmedSpread,
        anchorSource: null,
        reason: trimmedReason,
      };
      const out = await configureWrite.send(payload);
      props.onSaved(out);
    } catch (err: unknown) {
      applyError(err);
    } finally {
      setSaving(false);
    }
  };

  const handleClearOverride = async (e: Event) => {
    e.preventDefault();
    if (!props.mayEdit || saving()) return;
    setError(null);
    setFieldErrors({});

    const trimmedReason = reason().trim();
    if (trimmedReason === '') {
      setFieldErrors({ reason: localError('reason') });
      return;
    }

    setSaving(true);
    try {
      const payload: PricingConfigureInput = {
        policyVersionId: props.policyVersionId,
        currency: props.row.currency,
        tiers: null,
        minSpreadPct: null,
        anchorSource: null,
        reason: trimmedReason,
      };
      const out = await configureWrite.send(payload);
      props.onSaved(out);
    } catch (err: unknown) {
      applyError(err);
    } finally {
      setSaving(false);
    }
  };

  return (
    <form class="pricing-policy-form" noValidate onSubmit={handleSave}>
      <Show when={error()}>
        <div role="alert" class="pricing-policy-alert">
          {error()}
        </div>
      </Show>

      <div class="pricing-policy-header">
        <span class="pricing-policy-title">{props.row.currency}</span>
        <Show when={props.row.overridden}>
          <span class="pricing-policy-badge">{t('overridden')}</span>
        </Show>
      </div>

      <div class="pricing-policy-summary">
        <For each={props.row.margins}>
          {(m) => (
            <span class="pricing-policy-readonly-margin">
              {t('tier', { n: m.tierNo })}: {m.buyMarginPct} / {m.sellMarginPct}
            </span>
          )}
        </For>
        <Show when={props.row.minSpreadPct}>
          <span class="pricing-policy-readonly-spread">
            {t('minSpread')}: {props.row.minSpreadPct}
          </span>
        </Show>
      </div>

      <div class="pricing-policy-grid">
        <For each={TIERS}>
          {(tierNo) => (
            <div class="pricing-policy-tier">
              <span class="pricing-policy-tier-label">{t('tier', { n: tierNo })}</span>
              <div class="pricing-policy-field">
                <label class="pricing-policy-label" for={`buy-margin-${tierNo}`}>
                  {t('buy')} ({t('margin')})
                </label>
                <input
                  id={`buy-margin-${tierNo}`}
                  class="pricing-policy-input"
                  name={`buy-margin-${tierNo}`}
                  type="text"
                  disabled={!props.mayEdit || saving()}
                  title={props.disabledReason}
                  value={margins()[`buy-${tierNo}`] ?? ''}
                  onInput={(e) => updateMargin(`buy-${tierNo}`, e.currentTarget.value)}
                />
                <Show when={fieldErrors()[`margins.buy-${tierNo}`]}>
                  {(msg) => <span data-field-error={`margins.buy-${tierNo}`}>{msg()}</span>}
                </Show>
              </div>
              <div class="pricing-policy-field">
                <label class="pricing-policy-label" for={`sell-margin-${tierNo}`}>
                  {t('sell')} ({t('margin')})
                </label>
                <input
                  id={`sell-margin-${tierNo}`}
                  class="pricing-policy-input"
                  name={`sell-margin-${tierNo}`}
                  type="text"
                  disabled={!props.mayEdit || saving()}
                  title={props.disabledReason}
                  value={margins()[`sell-${tierNo}`] ?? ''}
                  onInput={(e) => updateMargin(`sell-${tierNo}`, e.currentTarget.value)}
                />
                <Show when={fieldErrors()[`margins.sell-${tierNo}`]}>
                  {(msg) => <span data-field-error={`margins.sell-${tierNo}`}>{msg()}</span>}
                </Show>
              </div>
            </div>
          )}
        </For>
      </div>

      <div class="pricing-policy-spread">
        <label class="pricing-policy-label" for="minSpreadPct">
          {t('minSpread')}
        </label>
        <input
          id="minSpreadPct"
          class="pricing-policy-input"
          name="minSpreadPct"
          type="text"
          disabled={!props.mayEdit || saving()}
          title={props.disabledReason}
          value={minSpread()}
          onInput={(e) => setMinSpread(e.currentTarget.value)}
        />
        <Show when={fieldErrors().minSpreadPct}>
          {(msg) => <span data-field-error="minSpreadPct">{msg()}</span>}
        </Show>
      </div>

      <div class="pricing-policy-field">
        <label class="pricing-policy-label" for="reason">
          {t('reason')}
        </label>
        <input
          id="reason"
          class="pricing-policy-input"
          name="reason"
          type="text"
          required
          disabled={!props.mayEdit || saving()}
          title={props.disabledReason}
          value={reason()}
          onInput={(e) => setReason(e.currentTarget.value)}
        />
        <Show when={fieldErrors().reason}>
          {(msg) => <span data-field-error="reason">{msg()}</span>}
        </Show>
      </div>

      <div class="pricing-policy-actions">
        <button
          type="submit"
          class="pricing-policy-btn-save"
          name="save"
          disabled={!props.mayEdit || saving()}
          title={props.disabledReason}
        >
          {t('save')}
        </button>
        <button
          type="button"
          class="pricing-policy-btn-clear"
          name="clearOverride"
          disabled={!props.mayEdit || saving()}
          title={props.disabledReason}
          onClick={handleClearOverride}
        >
          {t('cancel')}
        </button>
      </div>
    </form>
  );
}
