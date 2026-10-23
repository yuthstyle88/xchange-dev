import { render } from 'solid-js/web';
import { describe, expect, it } from 'vitest';
import { makeCampaignRuleDetail } from '../../../shared/testing/fixtures';
import { CampaignRulesEditor } from './CampaignRulesEditor';
import { emptyRuleForm, ruleErrors, ruleFormOf, ruleInputOf } from './campaign-rules';

describe('M02 campaign-rules behaviour table (new rows)', () => {
  it('allows FEE_WAIVER percent 1 (range (0, 1])', () => {
    const form = {
      ...emptyRuleForm(),
      benefitType: 'FEE_WAIVER' as const,
      calcMode: 'percent',
      calcValue: '1',
    };
    expect(ruleErrors(form)).toEqual({});
    expect(ruleInputOf(form).calculation).toEqual({ percent: '1' });
  });

  it('refuses FEE_WAIVER amount 0 or fixed -5 with calculation error', () => {
    const zeroAmount = {
      ...emptyRuleForm(),
      benefitType: 'FEE_WAIVER' as const,
      calcMode: 'amount',
      calcValue: '0',
    };
    const negFixed = {
      ...emptyRuleForm(),
      benefitType: 'FEE_WAIVER' as const,
      calcMode: 'fixed',
      calcValue: '-5',
    };
    const threeDpAmount = {
      ...emptyRuleForm(),
      benefitType: 'FEE_WAIVER' as const,
      calcMode: 'amount',
      calcValue: '10.123',
    };
    expect(ruleErrors(zeroAmount)).toEqual({ calculation: true });
    expect(ruleErrors(negFixed)).toEqual({ calculation: true });
    expect(ruleErrors(threeDpAmount)).toEqual({ calculation: true });
  });

  it('refuses calculation values with more than 8 decimal places', () => {
    const eightDp = {
      ...emptyRuleForm(),
      benefitType: 'RATE_IMPROVEMENT' as const,
      calcMode: 'delta',
      calcValue: '0.12345678',
    };
    const nineDp = {
      ...emptyRuleForm(),
      benefitType: 'RATE_IMPROVEMENT' as const,
      calcMode: 'delta',
      calcValue: '0.123456789',
    };
    expect(ruleErrors(eightDp)).toEqual({});
    expect(ruleErrors(nineDp)).toEqual({ calculation: true });
  });

  it('normalises currencies " usd, JPY " to ["USD", "JPY"] and omits currencies when empty', () => {
    const withCurrencies = {
      ...emptyRuleForm(),
      calcValue: '0.05',
      currencies: ' usd, JPY ',
    };
    expect(ruleInputOf(withCurrencies).eligibility).toEqual({
      currencies: ['USD', 'JPY'],
    });

    const emptyCurrencies = {
      ...emptyRuleForm(),
      calcValue: '0.05',
      currencies: ' , ',
    };
    expect(ruleInputOf(emptyCurrencies).eligibility).toEqual({});
    expect('currencies' in ruleInputOf(emptyCurrencies).eligibility).toBe(false);
  });

  it('refuses perCustomerUses "2.5" or "-1" with data-field-error="rules.caps"', () => {
    for (const bad of ['2.5', '-1']) {
      const form = {
        ...emptyRuleForm(),
        calcValue: '0.05',
        perCustomerUses: bad,
      };
      const err = ruleErrors(form);
      expect(err).toEqual({ caps: true });

      const host = document.createElement('div');
      const dispose = render(
        () =>
          CampaignRulesEditor({
            rules: [form],
            readOnly: false,
            errors: [err],
            onChange: () => {},
          }),
        host,
      );
      expect(host.querySelector('[data-field-error="rules.caps"]')).not.toBeNull();
      dispose();
    }
  });

  it('shows a stored calculation with an unknown key read-only and sends it unchanged', () => {
    const stored = makeCampaignRuleDetail({
      id: 'rule-unknown-calc',
      ruleNo: 1,
      benefitType: 'RATE_IMPROVEMENT',
      benefitClass: 'PRICE',
      requiresCoupon: false,
      eligibility: {},
      calculation: { spreadShare: '0.20' },
      caps: {},
    });
    const form = ruleFormOf(stored);
    expect(ruleErrors(form)).toEqual({});
    expect(ruleInputOf(form).calculation).toEqual({ spreadShare: '0.20' });

    const host = document.createElement('div');
    const dispose = render(
      () =>
        CampaignRulesEditor({
          rules: [form],
          readOnly: false,
          errors: [{}],
          onChange: () => {},
        }),
      host,
    );
    const row = host.querySelector('[data-rule-row="0"]')!;
    const modeSelect = row.querySelector<HTMLSelectElement>('select[name="calcMode"]')!;
    const valInput = row.querySelector<HTMLInputElement>('input[name="calcValue"]')!;
    expect(modeSelect.value).toBe('spreadShare');
    expect(valInput.value).toBe('0.20');
    expect(modeSelect.disabled).toBe(true);
    expect(valInput.disabled).toBe(true);
    dispose();
  });
});
