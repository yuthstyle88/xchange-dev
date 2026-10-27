// V.9 branch default basis points (pricing-tiers.md §15.4): the `"*"` row of pricingBranchPolicySave (PV-d2), used by
// every currency of this branch that has no row of its own.
import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingBpDto } from '../../../shared/api/generated/pricing_board';
import type {
  PricingBranchPolicySaveInput,
  PricingBranchPolicySaveOutput,
} from '../../../shared/api/generated/pricing_branch_policy_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import * as table from './i18n';
import { BRANCH_BP_MAX, BRANCH_BP_MIN, ROUTE_PRICING_BRANCH_POLICY } from './pricing-labels';
import './pricing-policy.css';

export interface BranchDefaultProps {
  api: ApiClient;
  branch: string;
  /** `pricing_board.branchDefault`. */
  value: PricingBpDto;
  /** EXECUTIVE at this branch; read-only otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: PricingBranchPolicySaveOutput): void;
}

function parseBp(raw: string): number | null {
  const s = raw.trim();
  if (!/^-?\d+$/.test(s)) return null;
  const n = Number(s) || 0;
  return n >= BRANCH_BP_MIN && n <= BRANCH_BP_MAX ? n : null;
}

export function BranchDefault(props: BranchDefaultProps): JSX.Element {
  const t = screenT('admin.pricing', table);
  const saveWrite = keyedWrite<PricingBranchPolicySaveInput, PricingBranchPolicySaveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_BRANCH_POLICY,
  );

  const [buyBp, setBuyBp] = createSignal(String(props.value.buyBp));
  const [sellBp, setSellBp] = createSignal(String(props.value.sellBp));
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

  createEffect(() => {
    setBuyBp(String(props.value.buyBp));
    setSellBp(String(props.value.sellBp));
    setError(null);
    setFieldErrors({});
  });

  const handleSave = async (e: Event) => {
    e.preventDefault();
    if (!props.mayEdit || saving() || props.disabledReason) return;
    setError(null);
    setFieldErrors({});

    const buy = parseBp(buyBp());
    const sell = parseBp(sellBp());
    const rangeMsg = t('bpRange', { min: BRANCH_BP_MIN, max: BRANCH_BP_MAX });
    const nextErrors: Record<string, string> = {};
    if (buy === null) nextErrors.buyBp = rangeMsg;
    if (sell === null) nextErrors.sellBp = rangeMsg;
    if (Object.keys(nextErrors).length > 0) {
      setFieldErrors(nextErrors);
      return;
    }

    setSaving(true);
    try {
      const out = await saveWrite.send({
        currency: '*',
        buyBp: buy!,
        sellBp: sell!,
      });
      props.onSaved(out);
    } catch (err: unknown) {
      const fe = fieldError(err, {
        buyBp: t('buyBp'),
        sellBp: t('sellBp'),
      });
      if (fe.field) {
        setFieldErrors({ [fe.field]: fe.message });
      } else {
        setError(fe.message);
      }
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
        <span class="pricing-policy-title">{t('branchDefaultTitle')}</span>
      </div>

      <div class="pricing-policy-field">
        <label class="pricing-policy-label">{t('buyBp')}</label>
        <input
          class="pricing-policy-input"
          name="buyBp"
          type="text"
          disabled={!props.mayEdit || saving()}
          title={props.disabledReason}
          value={buyBp()}
          onInput={(e) => setBuyBp(e.currentTarget.value)}
        />
        <Show when={fieldErrors().buyBp}>
          {(msg) => <span data-field-error="buyBp">{msg()}</span>}
        </Show>
      </div>

      <div class="pricing-policy-field">
        <label class="pricing-policy-label">{t('sellBp')}</label>
        <input
          class="pricing-policy-input"
          name="sellBp"
          type="text"
          disabled={!props.mayEdit || saving()}
          title={props.disabledReason}
          value={sellBp()}
          onInput={(e) => setSellBp(e.currentTarget.value)}
        />
        <Show when={fieldErrors().sellBp}>
          {(msg) => <span data-field-error="sellBp">{msg()}</span>}
        </Show>
      </div>

      <Show when={props.mayEdit || Boolean(props.disabledReason)}>
        <div class="pricing-policy-actions">
          <button
            type="submit"
            class="pricing-policy-btn-save"
            name="save-default"
            disabled={!props.mayEdit || saving() || Boolean(props.disabledReason)}
            title={props.disabledReason}
          >
            {t('save')}
          </button>
        </div>
      </Show>
    </form>
  );
}
