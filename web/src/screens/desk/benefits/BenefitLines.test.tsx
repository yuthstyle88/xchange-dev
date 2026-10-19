import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { BenefitLines, type AppliedBenefitView } from './BenefitLines';
import { th, en } from './i18n';

describe('FE-desk F7 (B03): BenefitLines component (#3341)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  it('row 1: [] renders nothing', () => {
    dispose = render(() => <dl><BenefitLines benefits={[]} /></dl>, host);
    expect(host.querySelector('dt')).toBeNull();
    expect(host.querySelector('dd')).toBeNull();
  });

  it('row 2: one PRICE benefit, rateBefore "32.10" → rateAfter "32.20" -> line with benefit label and both rates as sent', () => {
    const benefit: AppliedBenefitView = {
      ruleId: 'r-1',
      versionId: 'v-1',
      benefitType: 'RATE_DISCOUNT',
      benefitClass: 'PRICE',
      amountThb: '0.00',
      rateBefore: '32.10',
      rateAfter: '32.20',
      feeBefore: '0.00',
      feeAfter: '0.00',
      floorCapped: false,
    };
    dispose = render(() => <dl><BenefitLines benefits={[benefit]} /></dl>, host);
    const dt = host.querySelector('dt');
    const dd = host.querySelector('dd');
    expect(dt).not.toBeNull();
    expect(dt?.textContent).toContain(en.benefit);
    expect(dd).not.toBeNull();
    expect(dd?.textContent).toContain('32.10');
    expect(dd?.textContent).toContain('32.20');
  });

  it('row 3: a fee benefit (feeBefore ≠ feeAfter) -> the fee before → after', () => {
    const benefit: AppliedBenefitView = {
      ruleId: 'r-2',
      versionId: 'v-2',
      benefitType: 'FEE_WAIVER',
      benefitClass: 'PRICE',
      amountThb: '0.00',
      rateBefore: '32.10',
      rateAfter: '32.10',
      feeBefore: '100.00',
      feeAfter: '20.00',
      floorCapped: false,
    };
    dispose = render(() => <dl><BenefitLines benefits={[benefit]} /></dl>, host);
    const dd = host.querySelector('dd');
    expect(dd).not.toBeNull();
    expect(dd?.textContent).toContain('100.00');
    expect(dd?.textContent).toContain('20.00');
  });

  it('row 4: a REWARD benefit with amountThb "50.00" -> a line with "50.00 THB"', () => {
    const benefit: AppliedBenefitView = {
      ruleId: 'r-3',
      versionId: 'v-3',
      benefitType: 'CASHBACK',
      benefitClass: 'REWARD',
      amountThb: '50.00',
      rateBefore: '32.10',
      rateAfter: '32.10',
      feeBefore: '0.00',
      feeAfter: '0.00',
      floorCapped: false,
    };
    dispose = render(() => <dl><BenefitLines benefits={[benefit]} /></dl>, host);
    const dd = host.querySelector('dd');
    expect(dd).not.toBeNull();
    expect(dd?.textContent).toContain('50.00 THB');
  });

  it('row 5: couponId present -> the line says it came from a coupon', () => {
    const benefit: AppliedBenefitView = {
      ruleId: 'r-4',
      versionId: 'v-4',
      benefitType: 'COUPON_RATE',
      benefitClass: 'PRICE',
      couponId: 'cpn-001',
      amountThb: '0.00',
      rateBefore: '32.10',
      rateAfter: '32.25',
      feeBefore: '0.00',
      feeAfter: '0.00',
      floorCapped: false,
    };
    dispose = render(() => <dl><BenefitLines benefits={[benefit]} /></dl>, host);
    const lineText = host.textContent?.toLowerCase() ?? '';
    expect(lineText).toContain('coupon');
  });

  it('row 6: floorCapped: true -> a note that the benefit was capped', () => {
    const benefit: AppliedBenefitView = {
      ruleId: 'r-5',
      versionId: 'v-5',
      benefitType: 'RATE_DISCOUNT',
      benefitClass: 'PRICE',
      amountThb: '0.00',
      rateBefore: '32.10',
      rateAfter: '32.20',
      feeBefore: '0.00',
      feeAfter: '0.00',
      floorCapped: true,
    };
    dispose = render(() => <dl><BenefitLines benefits={[benefit]} /></dl>, host);
    const lineText = host.textContent?.toLowerCase() ?? '';
    expect(lineText).toContain('capped');
  });

  it('skips non-object items in benefits array', () => {
    dispose = render(() => <dl><BenefitLines benefits={[null, undefined, 'bad', 42, false] as unknown as readonly unknown[]} /></dl>, host);
    expect(host.querySelector('dt')).toBeNull();
    expect(host.querySelector('dd')).toBeNull();
  });

  it('renders Thai labels when lang is th', () => {
    setLang('th');
    const benefit: AppliedBenefitView = {
      ruleId: 'r-6',
      versionId: 'v-6',
      benefitType: 'COUPON_RATE',
      benefitClass: 'PRICE',
      couponId: 'cpn-002',
      amountThb: '0.00',
      rateBefore: '32.10',
      rateAfter: '32.20',
      feeBefore: '0.00',
      feeAfter: '0.00',
      floorCapped: true,
    };
    dispose = render(() => <dl><BenefitLines benefits={[benefit]} /></dl>, host);
    const text = host.textContent ?? '';
    expect(text).toContain(th.benefit);
    expect(text).toContain(th.coupon);
    expect(text).toContain(th.capped);
  });
});
