// V.9 branch policy for one currency (pricing-tiers.md §15.4, §15.8a): buy/sell basis points, "own" (the branch prices
// this currency itself) and "follow" (enrol in MAIN's FOLLOW), saved with pricingBranchPolicySave (PV-d2).
import { createEffect, createSignal, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import type {
  PricingBranchPolicySaveInput,
  PricingBranchPolicySaveOutput,
} from '../../../shared/api/generated/pricing_branch_policy_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import * as table from './i18n';
import {
  BRANCH_BP_MAX,
  BRANCH_BP_MIN,
  policyFromLabel,
  ROUTE_PRICING_BRANCH_POLICY,
} from './pricing-labels';
import './pricing-policy.css';

export interface BranchPolicyProps {
  api: ApiClient;
  branch: string;
  row: PricingRowDto;
  /** `pricing_board.isMain`: MAIN pushes to the branches, so "own" and "follow" mean nothing there. */
  isMain: boolean;
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

export function BranchPolicy(props: BranchPolicyProps): JSX.Element {
  const t = screenT('admin.pricing', table);
  const saveWrite = keyedWrite<PricingBranchPolicySaveInput, PricingBranchPolicySaveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_BRANCH_POLICY,
  );

  const [buyBp, setBuyBp] = createSignal(String(props.row.branchPolicy.buyBp));
  const [sellBp, setSellBp] = createSignal(String(props.row.branchPolicy.sellBp));
  const [own, setOwn] = createSignal(props.row.branchPolicy.own);
  const [follow, setFollow] = createSignal(props.row.branchPolicy.follow);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

  createEffect(() => {
    const bp = props.row.branchPolicy;
    setBuyBp(String(bp.buyBp));
    setSellBp(String(bp.sellBp));
    setOwn(bp.own);
    setFollow(bp.follow);
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
      const payload: PricingBranchPolicySaveInput = props.isMain
        ? { currency: props.row.currency, buyBp: buy!, sellBp: sell! }
        : {
            currency: props.row.currency,
            buyBp: buy!,
            sellBp: sell!,
            own: own(),
            follow: follow(),
          };
      const out = await saveWrite.send(payload);
      props.onSaved(out);
    } catch (err: unknown) {
      const fe = fieldError(err, {
        buyBp: t('buyBp'),
        sellBp: t('sellBp'),
        own: t('ownPrice'),
        follow: t('followMain'),
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
        <span class="pricing-policy-title">{t('policyTitle')}</span>
        <span class="pricing-policy-badge">{policyFromLabel(props.row.branchPolicy.from)}</span>
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

      <Show when={!props.isMain}>
        <div class="pricing-policy-field">
          <label class="pricing-policy-label">
            <input
              name="own"
              type="checkbox"
              disabled={!props.mayEdit || saving()}
              title={props.disabledReason}
              checked={own()}
              onChange={(e) => setOwn(e.currentTarget.checked)}
            />
            {t('ownPrice')}
          </label>
          <Show when={fieldErrors().own}>
            {(msg) => <span data-field-error="own">{msg()}</span>}
          </Show>
        </div>

        <div class="pricing-policy-field">
          <label class="pricing-policy-label">
            <input
              name="follow"
              type="checkbox"
              disabled={!props.mayEdit || saving()}
              title={props.disabledReason}
              checked={follow()}
              onChange={(e) => setFollow(e.currentTarget.checked)}
            />
            {t('followMain')}
          </label>
          <Show when={fieldErrors().follow}>
            {(msg) => <span data-field-error="follow">{msg()}</span>}
          </Show>
        </div>
      </Show>

      <Show when={props.mayEdit || Boolean(props.disabledReason)}>
        <div class="pricing-policy-actions">
          <button
            type="submit"
            class="pricing-policy-btn-save"
            name="save-policy"
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
