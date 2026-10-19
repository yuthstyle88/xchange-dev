// §9 PricingPolicy: one currency's override margins and min spread (pricingConfigure) (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import {
  ROUTE_PRICING_CONFIGURE,
  type PricingConfigureInput,
  type PricingConfigureOutput,
  type PricingRowView,
  type TierNo,
} from './pricing-board.types';
import './pricing-policy.css';

export interface PricingPolicyProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  row: PricingRowView;
  /** EXECUTIVE frame (pricing-tiers.md §3: override margins are frame); the editor is hidden otherwise. */
  mayEdit: boolean;
  onSaved(out: PricingConfigureOutput): void;
}

const TIERS: readonly TierNo[] = [1, 2, 3, 4];

export function PricingPolicy(props: PricingPolicyProps): JSX.Element {
  const t = screenT('admin.pricing', table);
  const configureWrite = keyedWrite<PricingConfigureInput, PricingConfigureOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_CONFIGURE,
  );
  // One requestId per distinct payload, so a retry after a lost reply reuses it (design §6: requestId is the
  // idempotency key; a fresh id per click could write a second policy version).
  let lastPayload = '';
  let lastRequestId = '';
  const requestIdFor = (body: Omit<PricingConfigureInput, 'requestId'>): string => {
    const key = JSON.stringify(body);
    if (key !== lastPayload || lastRequestId === '') {
      lastPayload = key;
      lastRequestId = crypto.randomUUID();
    }
    return lastRequestId;
  };

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
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

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
    setError(null);
  });

  const updateMargin = (key: string, value: string) => {
    setMargins((prev) => ({ ...prev, [key]: value }));
  };

  const handleSave = async (e: Event) => {
    e.preventDefault();
    if (!props.mayEdit || saving()) return;
    setError(null);
    setSaving(true);
    try {
      const cur = margins();
      const body = {
        branch: props.branch,
        policyVersionId: props.policyVersionId,
        currency: props.row.currency,
        margins: TIERS.map((n) => ({
          tierNo: n,
          buyMarginPct: cur[`buy-${n}`] ?? '',
          sellMarginPct: cur[`sell-${n}`] ?? '',
        })),
        minSpreadPct: minSpread().trim() === '' ? null : minSpread().trim(),
      };
      const payload: PricingConfigureInput = { ...body, requestId: requestIdFor(body) };
      const out = await configureWrite.send(payload);
      props.onSaved(out);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setSaving(false);
    }
  };

  const handleClearOverride = async (e: Event) => {
    e.preventDefault();
    if (!props.mayEdit || saving()) return;
    setError(null);
    setSaving(true);
    try {
      const body = {
        branch: props.branch,
        policyVersionId: props.policyVersionId,
        currency: props.row.currency,
        margins: null,
        minSpreadPct: null,
      };
      const payload: PricingConfigureInput = { ...body, requestId: requestIdFor(body) };
      const out = await configureWrite.send(payload);
      props.onSaved(out);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <form class="pricing-policy-form" onSubmit={handleSave}>
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
                  value={margins()[`buy-${tierNo}`] ?? ''}
                  onInput={(e) => updateMargin(`buy-${tierNo}`, e.currentTarget.value)}
                />
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
                  value={margins()[`sell-${tierNo}`] ?? ''}
                  onInput={(e) => updateMargin(`sell-${tierNo}`, e.currentTarget.value)}
                />
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
          value={minSpread()}
          onInput={(e) => setMinSpread(e.currentTarget.value)}
        />
      </div>

      <div class="pricing-policy-actions">
        <button
          type="submit"
          class="pricing-policy-btn-save"
          name="save"
          disabled={!props.mayEdit || saving()}
        >
          {t('save')}
        </button>
        <button
          type="button"
          class="pricing-policy-btn-clear"
          name="clearOverride"
          disabled={!props.mayEdit || saving()}
          onClick={handleClearOverride}
        >
          {t('cancel')}
        </button>
      </div>
    </form>
  );
}
