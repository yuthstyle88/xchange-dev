import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import { BOARD } from './__fixtures__/pricing-board.fixture';
import { th } from './i18n';
import { PricingRow, type PricingRowProps } from './PricingRow';

vi.mock('./pricing-labels', () => ({
  BLOCKING_FLAGS: ['BelowCost', 'StaleSource', 'Precision'],
  statusLabel: (status: string) => `status:${status}`,
  flagLabel: (flag: string) => `flag:${flag}`,
}));

const USD = BOARD.rows.find((r) => r.currency === 'USD')!;
const EUR = BOARD.rows.find((r) => r.currency === 'EUR')!;
const JPY = BOARD.rows.find((r) => r.currency === 'JPY')!;
const GBP = BOARD.rows.find((r) => r.currency === 'GBP')!;
const CNY = BOARD.rows.find((r) => r.currency === 'CNY')!;
const KRW = BOARD.rows.find((r) => r.currency === 'KRW')!;

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(row: PricingRowDto, props: Partial<PricingRowProps> = {}) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <table>
        <tbody>
          <PricingRow
            row={row}
            selected={props.selected ?? false}
            onToggle={props.onToggle ?? vi.fn()}
            onOpen={props.onOpen ?? vi.fn()}
          />
        </tbody>
      </table>
    ),
    host,
  );
}

describe('PricingRow', () => {
  it('R1 a row: renders code, TT / SR / BoT rates (— when absent), tier-1 price, suggested tier 1, status chip, flag chips', () => {
    mount(USD);
    expect(host.textContent).toContain('USD');
    // TT quote
    expect(host.querySelector('.pricing-cell-quote-tt')?.textContent).toContain('35.10');
    expect(host.querySelector('.pricing-cell-quote-tt')?.textContent).toContain('35.40');
    // SR quote
    expect(host.querySelector('.pricing-cell-quote-sr')?.textContent).toContain('35.05');
    expect(host.querySelector('.pricing-cell-quote-sr')?.textContent).toContain('35.45');
    // BoT quote
    expect(host.querySelector('.pricing-cell-quote-bot')?.textContent).toContain('35.00');
    expect(host.querySelector('.pricing-cell-quote-bot')?.textContent).toContain('35.50');
    // Current tier 1
    expect(host.querySelector('.pricing-cell-current')?.textContent).toContain('35.00');
    expect(host.querySelector('.pricing-cell-current')?.textContent).toContain('35.50');
    // Suggested tier 1
    expect(host.querySelector('.pricing-cell-suggested')?.textContent).toContain('35.02');
    expect(host.querySelector('.pricing-cell-suggested')?.textContent).toContain('35.48');
    // Status chip and flag chip
    expect(host.textContent).toContain('status:Tiers');
    expect(host.textContent).toContain('flag:LargeMove');

    // Absent quotes & suggestions (KRW has no TT, no SR, no suggestion)
    dispose?.();
    mount(KRW);
    expect(host.querySelector('.pricing-cell-quote-tt')?.textContent).toContain('—');
    expect(host.querySelector('.pricing-cell-quote-sr')?.textContent).toContain('—');
    expect(host.querySelector('.pricing-cell-suggested')?.textContent).toContain('—');
  });

  it('R2 a blocking flag: input[type=checkbox][name=select-<code>] disabled', () => {
    // BelowCost
    mount(EUR);
    const cbEur = host.querySelector<HTMLInputElement>('input[type=checkbox][name=select-EUR]');
    expect(cbEur?.disabled).toBe(true);

    // StaleSource
    dispose?.();
    mount(JPY);
    const cbJpy = host.querySelector<HTMLInputElement>('input[type=checkbox][name=select-JPY]');
    expect(cbJpy?.disabled).toBe(true);

    // Precision
    dispose?.();
    mount(GBP);
    const cbGbp = host.querySelector<HTMLInputElement>('input[type=checkbox][name=select-GBP]');
    expect(cbGbp?.disabled).toBe(true);
  });

  it('R3 no blocking flag: checkbox calls onToggle(code); selected sets checked', () => {
    const onToggle = vi.fn();
    mount(USD, { selected: true, onToggle });
    const cb = host.querySelector<HTMLInputElement>('input[type=checkbox][name=select-USD]');
    expect(cb?.disabled).toBe(false);
    expect(cb?.checked).toBe(true);

    cb?.click();
    expect(onToggle).toHaveBeenCalledWith('USD');
  });

  it('R4 the code clicked: calls onOpen(code)', () => {
    const onOpen = vi.fn();
    mount(USD, { onOpen });
    const btn = host.querySelector<HTMLButtonElement>('.pricing-btn-code');
    expect(btn?.textContent).toBe('USD');
    btn?.click();
    expect(onOpen).toHaveBeenCalledWith('USD');
  });

  it('R5 overridden: renders the overridden text when true, omitted when false', () => {
    mount(CNY);
    expect(host.querySelector('.pricing-badge-overridden')?.textContent).toBe(th.overridden);

    dispose?.();
    mount(USD);
    expect(host.querySelector('.pricing-badge-overridden')).toBeNull();
  });
});
