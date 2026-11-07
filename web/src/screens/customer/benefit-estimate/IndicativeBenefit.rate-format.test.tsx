import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePlannerBenefitItemDto,
  makePlannerPreviewAnswer,
} from '../../../shared/testing/fixtures/planner_preview';
import { IndicativeBenefit, type IndicativeBenefitProps } from './IndicativeBenefit';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(benefit?: IndicativeBenefitProps['benefit']) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <IndicativeBenefit benefit={benefit} />, host);
}

const items = () => host.querySelectorAll('[data-row="benefit-item"]');

beforeEach(() => {
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('IndicativeBenefit rate format (#6462 §9)', () => {
  it('shows 32.15 → 32.25 for a RATE benefit item 32.15000000 → 32.25000000', () => {
    const item = makePlannerBenefitItemDto({
      label: 'Gold week',
      benefitType: 'RATE_IMPROVEMENT',
      benefitClass: 'PRICE',
      rateBefore: '32.15000000',
      rateAfter: '32.25000000',
    });
    const answer = makePlannerPreviewAnswer({
      benefit: {
        state: 'ESTIMATED',
        basis: 'INDICATIVE',
        channel: 'APP',
        evaluatedAt: '2026-10-28T00:00:00Z',
        validUntil: '2026-11-01T00:00:00Z',
        publicationId: 'pub-1',
        finalRate: '32.25000000',
        thbAfter: '3225.00',
        benefitThb: '10.00',
        items: [item],
      },
    });
    mount(answer.benefit);
    expect(items()).toHaveLength(1);
    expect(items()[0]?.textContent).toBe('Gold week: 32.15 → 32.25');
  });

  it('leaves FEE_WAIVER and REWARD items unchanged', () => {
    const feeWaiver = makePlannerBenefitItemDto({
      label: 'Transfer fee',
      benefitType: 'FEE_WAIVER',
      benefitClass: 'PRICE',
      amountThb: '0.00',
    });
    const reward = makePlannerBenefitItemDto({
      label: 'Points bonus',
      benefitType: 'REWARD_POINTS',
      benefitClass: 'REWARD',
      amountThb: '50.00',
    });
    const answer = makePlannerPreviewAnswer({
      benefit: {
        state: 'ESTIMATED',
        basis: 'INDICATIVE',
        channel: 'APP',
        evaluatedAt: '2026-10-28T00:00:00Z',
        validUntil: '2026-11-01T00:00:00Z',
        publicationId: 'pub-1',
        finalRate: '35.00',
        thbAfter: '3500.00',
        benefitThb: '50.00',
        items: [feeWaiver, reward],
      },
    });
    mount(answer.benefit);
    expect(items()).toHaveLength(2);
    expect(items()[0]?.textContent).toBe('Transfer fee: ยกเว้นค่าธรรมเนียม');
    expect(items()[1]?.textContent).toBe('Points bonus: 50.00 THB');
  });
});
