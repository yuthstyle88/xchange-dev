// XC-BOOKING-ESTIMATE FE-0 (#5395): tests for IndicativeBenefit block (§8 rows 1-8).
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

function mount(benefit?: IndicativeBenefitProps['benefit'], slotStartAt?: string | null) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <IndicativeBenefit benefit={benefit} slotStartAt={slotStartAt} />, host);
}

const section = () => host.querySelector('[data-row="benefit-estimate"]');
const items = () => host.querySelectorAll('[data-row="benefit-item"]');
const total = () => host.querySelector('[data-row="benefit-total"]');
const note = () => host.querySelector('[data-row="benefit-indicative-note"]');
const endsBeforeSlot = () => host.querySelector('[data-row="benefit-ends-before-slot"]');

beforeEach(() => {
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('IndicativeBenefit (#5395 §8)', () => {
  it('row 1: benefit undefined renders nothing', () => {
    mount(undefined);
    expect(section()).toBeNull();
  });

  it('row 2: NONE renders nothing; no "0" anywhere', () => {
    const answer = makePlannerPreviewAnswer({
      benefit: {
        state: 'NONE',
        basis: 'INDICATIVE',
        channel: 'APP',
        evaluatedAt: '2026-10-28T00:00:00Z',
        reason: 'noActiveRule',
      },
    });
    mount(answer.benefit);
    expect(section()).toBeNull();
    expect(host.textContent ?? '').not.toContain('0');
  });

  it('row 3: ESTIMATED, one PRICE item, benefitThb non-zero', () => {
    const item = makePlannerBenefitItemDto({
      label: 'Gold week',
      benefitType: 'RATE_IMPROVEMENT',
      benefitClass: 'PRICE',
      rateBefore: '35.10',
      rateAfter: '35.30',
    });
    const answer = makePlannerPreviewAnswer({
      benefit: {
        state: 'ESTIMATED',
        basis: 'INDICATIVE',
        channel: 'APP',
        evaluatedAt: '2026-10-28T00:00:00Z',
        validUntil: '2026-11-01T00:00:00Z',
        publicationId: 'pub-1',
        finalRate: '35.30',
        thbAfter: '3530.00',
        benefitThb: '20.00',
        items: [item],
      },
    });
    mount(answer.benefit);
    expect(items()).toHaveLength(1);
    expect(items()[0]?.textContent).toBe('Gold week: 35.10 → 35.30');
    expect(total()?.textContent).toBe('สิทธิประโยชน์โดยประมาณ 20.00 THB');
    expect(note()?.textContent).toBe('เรทสุดท้ายยืนยันเมื่อทำรายการ');
  });

  it('row 4: ESTIMATED, one REWARD item', () => {
    const item = makePlannerBenefitItemDto({
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
        items: [item],
      },
    });
    mount(answer.benefit);
    expect(items()).toHaveLength(1);
    expect(items()[0]?.textContent).toBe('Points bonus: 50.00 THB');
  });

  it('row 5: ESTIMATED, one FEE_WAIVER item, amountThb 0.00, benefitThb 0.00', () => {
    const item = makePlannerBenefitItemDto({
      label: 'Transfer fee',
      benefitType: 'FEE_WAIVER',
      benefitClass: 'PRICE',
      amountThb: '0.00',
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
        benefitThb: '0.00',
        items: [item],
      },
    });
    mount(answer.benefit);
    expect(items()).toHaveLength(1);
    expect(items()[0]?.textContent).toBe('Transfer fee: ยกเว้นค่าธรรมเนียม');
    expect(items()[0]?.textContent).not.toContain('0.00');
    expect(total()).toBeNull();
  });

  it('row 6: ESTIMATED validUntil equals slotStartAt shows warning', () => {
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
        benefitThb: '0.00',
        items: [],
      },
    });
    mount(answer.benefit, '2026-11-01T00:00:00Z');
    expect(endsBeforeSlot()).not.toBeNull();
    expect(endsBeforeSlot()?.getAttribute('role')).toBe('note');
    expect(endsBeforeSlot()?.textContent).toBe('โปรโมชันสิ้นสุดก่อนช่วงเวลาที่คุณจอง');
  });

  it('row 7: ESTIMATED slotStartAt before validUntil shows no warning', () => {
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
        benefitThb: '0.00',
        items: [],
      },
    });
    mount(answer.benefit, '2026-10-31T23:59:00Z');
    expect(endsBeforeSlot()).toBeNull();
  });

  it('row 8: ESTIMATED no slotStartAt shows no warning', () => {
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
        benefitThb: '0.00',
        items: [],
      },
    });
    mount(answer.benefit);
    expect(endsBeforeSlot()).toBeNull();
  });

  it('edge case §9: two items render in order', () => {
    const item1 = makePlannerBenefitItemDto({
      label: 'Item 1',
      benefitType: 'RATE_IMPROVEMENT',
      benefitClass: 'PRICE',
      rateBefore: '35.10',
      rateAfter: '35.30',
    });
    const item2 = makePlannerBenefitItemDto({
      label: 'Item 2',
      benefitType: 'REWARD_POINTS',
      benefitClass: 'REWARD',
      amountThb: '10.00',
    });
    const answer = makePlannerPreviewAnswer({
      benefit: {
        state: 'ESTIMATED',
        basis: 'INDICATIVE',
        channel: 'APP',
        evaluatedAt: '2026-10-28T00:00:00Z',
        validUntil: '2026-11-01T00:00:00Z',
        publicationId: 'pub-1',
        finalRate: '35.30',
        thbAfter: '3530.00',
        benefitThb: '10.00',
        items: [item1, item2],
      },
    });
    mount(answer.benefit);
    expect(items()).toHaveLength(2);
    expect(items()[0]?.textContent).toBe('Item 1: 35.10 → 35.30');
    expect(items()[1]?.textContent).toBe('Item 2: 10.00 THB');
  });
});
