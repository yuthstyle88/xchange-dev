import { For, Index, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import {
  calcModesFor,
  emptyRuleForm,
  type BenefitType,
  type RuleError,
  type RuleForm,
} from './campaign-rules';
import * as table from './i18n';

const t = screenT('admin.campaigns', table);

export interface CampaignRulesEditorProps {
  rules: RuleForm[];
  readOnly: boolean;
  errors: RuleError[];
  onChange(rules: RuleForm[]): void;
}

export function CampaignRulesEditor(props: CampaignRulesEditorProps): JSX.Element {
  function updateRule(idx: number, patch: Partial<RuleForm>): void {
    const next = props.rules.map((r, i) => (i === idx ? { ...r, ...patch } : r));
    props.onChange(next);
  }

  function handleBenefitTypeChange(idx: number, raw: string): void {
    const current = props.rules[idx];
    if (!current) return;
    const benefitType: BenefitType = raw === 'FEE_WAIVER' ? 'FEE_WAIVER' : 'RATE_IMPROVEMENT';
    if (current.benefitType === benefitType) return;
    const modes = calcModesFor(benefitType);
    const calcMode = modes.includes(current.calcMode) ? current.calcMode : modes[0]!;
    updateRule(idx, { benefitType, calcMode });
  }

  function handleAddRule(): void {
    if (props.readOnly) return;
    props.onChange([...props.rules, emptyRuleForm()]);
  }

  function handleRemoveRule(idx: number): void {
    if (props.readOnly) return;
    props.onChange(props.rules.filter((_, i) => i !== idx));
  }

  function modesForRow(rule: RuleForm): string[] {
    const base = [...calcModesFor(rule.benefitType)];
    if (rule.keptCalculation && !base.includes(rule.calcMode)) {
      base.unshift(rule.calcMode);
    }
    return base;
  }

  return (
    <div class="xc-rules-editor" data-rules-editor>
      <div class="xc-detail-header">
        <h4>{t('rulesTitle')}</h4>
        <Show when={!props.readOnly}>
          <button
            type="button"
            class="xc-btn-secondary"
            disabled={props.readOnly}
            onClick={handleAddRule}
          >
            {t('addRule')}
          </button>
        </Show>
      </div>

      <Index each={props.rules}>
        {(rule, idx) => {
          const isRowReadOnly = () => props.readOnly || Boolean(rule().keptCalculation);
          return (
            <div class="xc-rule-row" data-rule-row={idx}>
              <div class="xc-form-grid">
                <div class="xc-form-group">
                  <label>{t('benefitType')}</label>
                  <select
                    name="benefitType"
                    class="xc-select"
                    disabled={isRowReadOnly()}
                    value={rule().benefitType}
                    onInput={(e) => handleBenefitTypeChange(idx, e.currentTarget.value)}
                    onChange={(e) => handleBenefitTypeChange(idx, e.currentTarget.value)}
                  >
                    <option value="RATE_IMPROVEMENT" selected={rule().benefitType === 'RATE_IMPROVEMENT'}>
                      RATE_IMPROVEMENT
                    </option>
                    <option value="FEE_WAIVER" selected={rule().benefitType === 'FEE_WAIVER'}>
                      FEE_WAIVER
                    </option>
                  </select>
                </div>

                <div class="xc-form-group">
                  <label>{t('calcMode')}</label>
                  <select
                    name="calcMode"
                    class="xc-select"
                    disabled={isRowReadOnly()}
                    value={rule().calcMode}
                    onInput={(e) => updateRule(idx, { calcMode: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { calcMode: e.currentTarget.value })}
                  >
                    <For each={modesForRow(rule())}>
                      {(mode) => (
                        <option value={mode} selected={rule().calcMode === mode}>
                          {mode}
                        </option>
                      )}
                    </For>
                  </select>
                </div>

                <div class="xc-form-group">
                  <label>{t('calcValue')}</label>
                  <input
                    type="text"
                    name="calcValue"
                    class="xc-input"
                    disabled={isRowReadOnly()}
                    value={rule().calcValue}
                    onInput={(e) => updateRule(idx, { calcValue: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { calcValue: e.currentTarget.value })}
                  />
                </div>

                <div class="xc-form-group">
                  <label>{t('currencies')}</label>
                  <input
                    type="text"
                    name="currencies"
                    class="xc-input"
                    disabled={isRowReadOnly()}
                    value={rule().currencies}
                    onInput={(e) => updateRule(idx, { currencies: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { currencies: e.currentTarget.value })}
                  />
                </div>

                <div class="xc-form-group">
                  <label>{t('shopSide')}</label>
                  <select
                    name="shopSide"
                    class="xc-select"
                    disabled={isRowReadOnly()}
                    value={rule().shopSide}
                    onInput={(e) =>
                      updateRule(idx, {
                        shopSide:
                          e.currentTarget.value === 'BUY' || e.currentTarget.value === 'SELL'
                            ? e.currentTarget.value
                            : '',
                      })
                    }
                    onChange={(e) =>
                      updateRule(idx, {
                        shopSide:
                          e.currentTarget.value === 'BUY' || e.currentTarget.value === 'SELL'
                            ? e.currentTarget.value
                            : '',
                      })
                    }
                  >
                    <option value="" selected={rule().shopSide === ''}></option>
                    <option value="BUY" selected={rule().shopSide === 'BUY'}>
                      BUY
                    </option>
                    <option value="SELL" selected={rule().shopSide === 'SELL'}>
                      SELL
                    </option>
                  </select>
                </div>

                <div class="xc-form-group">
                  <label>{t('minThb')}</label>
                  <input
                    type="text"
                    name="minThb"
                    class="xc-input"
                    disabled={isRowReadOnly()}
                    value={rule().minThb}
                    onInput={(e) => updateRule(idx, { minThb: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { minThb: e.currentTarget.value })}
                  />
                </div>

                <div class="xc-form-group">
                  <label>{t('perTransactionMaxThb')}</label>
                  <input
                    type="text"
                    name="perTransactionMaxThb"
                    class="xc-input"
                    disabled={isRowReadOnly()}
                    value={rule().perTransactionMaxThb}
                    onInput={(e) => updateRule(idx, { perTransactionMaxThb: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { perTransactionMaxThb: e.currentTarget.value })}
                  />
                </div>

                <div class="xc-form-group">
                  <label>{t('perCustomerUses')}</label>
                  <input
                    type="text"
                    name="perCustomerUses"
                    class="xc-input"
                    disabled={isRowReadOnly()}
                    value={rule().perCustomerUses}
                    onInput={(e) => updateRule(idx, { perCustomerUses: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { perCustomerUses: e.currentTarget.value })}
                  />
                </div>

                <div class="xc-form-group">
                  <label>{t('perCustomerPeriodDays')}</label>
                  <input
                    type="text"
                    name="perCustomerPeriodDays"
                    class="xc-input"
                    disabled={isRowReadOnly()}
                    value={rule().perCustomerPeriodDays}
                    onInput={(e) => updateRule(idx, { perCustomerPeriodDays: e.currentTarget.value })}
                    onChange={(e) => updateRule(idx, { perCustomerPeriodDays: e.currentTarget.value })}
                  />
                </div>

                <div class="xc-form-group">
                  <label>
                    <input
                      type="checkbox"
                      name="requiresCoupon"
                      disabled={isRowReadOnly()}
                      checked={rule().requiresCoupon}
                      onChange={(e) => updateRule(idx, { requiresCoupon: e.currentTarget.checked })}
                    />
                    {t('requiresCoupon')}
                  </label>
                </div>
              </div>

              <Show when={!props.readOnly}>
                <button
                  type="button"
                  class="xc-btn-secondary"
                  disabled={props.readOnly}
                  onClick={() => handleRemoveRule(idx)}
                >
                  {t('removeRule')}
                </button>
              </Show>

              <Show when={props.errors[idx]?.calculation}>
                <div class="xc-field-error" data-field-error="rules.calculation">
                  {t('calcInvalid')}
                </div>
              </Show>

              <Show when={props.errors[idx]?.caps}>
                <div class="xc-field-error" data-field-error="rules.caps">
                  {t('error.validationFailed')}
                </div>
              </Show>
            </div>
          );
        }}
      </Index>
    </div>
  );
}
