// §9 `Recommendations` (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §6, §9).
// Behaviour table:
// - items: listed by severity High → Warn → Medium → Low, then in the given order
// - an item: severity, code, currency, side (absent ⇒ blank)
// - none: nothing rendered
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal } from 'solid-js';
import {
  makePricingRecommendationDto,
  type PricingRecommendationDto,
} from '../../../shared/testing/fixtures/pricing_board';
import { BOARD } from './__fixtures__/pricing-board.fixture';
import { Recommendations } from './Recommendations';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

function mount(items: readonly PricingRecommendationDto[]) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <Recommendations items={items} />, host);
}

describe('Recommendations', () => {
  it('items: listed by severity High → Warn → Medium → Low, then in the given order', () => {
    const items = [
      makePricingRecommendationDto({ severity: 'Low', code: 'LOW_1', currency: 'USD', side: 'BUY' }),
      makePricingRecommendationDto({ severity: 'Medium', code: 'MED_1', currency: 'EUR', side: null }),
      makePricingRecommendationDto({ severity: 'High', code: 'HIGH_1', currency: null, side: null }),
      makePricingRecommendationDto({ severity: 'Warn', code: 'WARN_1', currency: 'JPY', side: 'SELL' }),
      makePricingRecommendationDto({ severity: 'High', code: 'HIGH_2', currency: 'GBP', side: 'BUY' }),
      makePricingRecommendationDto({ severity: 'Warn', code: 'WARN_2', currency: null, side: 'BUY' }),
      makePricingRecommendationDto({ severity: 'Medium', code: 'MED_2', currency: 'CHF', side: null }),
      makePricingRecommendationDto({ severity: 'Low', code: 'LOW_2', currency: null, side: null }),
    ];
    mount(items);
    const elements = host.querySelectorAll('.recommendation-item');
    expect(elements).toHaveLength(8);
    const codes = [...elements].map((el) => el.querySelector('.recommendation-code')?.textContent);
    expect(codes).toEqual([
      'HIGH_1',
      'HIGH_2',
      'WARN_1',
      'WARN_2',
      'MED_1',
      'MED_2',
      'LOW_1',
      'LOW_2',
    ]);
  });

  it('an item: severity, code, currency, side (absent ⇒ blank)', () => {
    const items = [
      makePricingRecommendationDto({ severity: 'High', code: 'BELOW_COST', currency: 'EUR', side: 'BUY' }),
      makePricingRecommendationDto({ severity: 'Warn', code: 'STALE_SOURCE', currency: null, side: null }),
    ];
    mount(items);
    const elements = host.querySelectorAll('.recommendation-item');
    expect(elements).toHaveLength(2);

    const first = elements[0];
    expect(first.querySelector('.recommendation-severity')?.textContent).toBe('High');
    expect(first.querySelector('.recommendation-code')?.textContent).toBe('BELOW_COST');
    expect(first.querySelector('.recommendation-currency')?.textContent).toBe('EUR');
    expect(first.querySelector('.recommendation-side')?.textContent).toBe('BUY');

    const second = elements[1];
    expect(second.querySelector('.recommendation-severity')?.textContent).toBe('Warn');
    expect(second.querySelector('.recommendation-code')?.textContent).toBe('STALE_SOURCE');
    expect(second.querySelector('.recommendation-currency')?.textContent).toBe('');
    expect(second.querySelector('.recommendation-side')?.textContent).toBe('');
  });

  it('none: nothing rendered when items is empty', () => {
    host = document.createElement('div');
    document.body.appendChild(host);
    const [items, setItems] = createSignal<readonly PricingRecommendationDto[]>(BOARD.recommendations);
    dispose = render(() => <Recommendations items={items()} />, host);
    expect(host.querySelectorAll('.recommendation-item')).toHaveLength(4);
    setItems([]);
    expect(host.innerHTML).toBe('');
  });
});
