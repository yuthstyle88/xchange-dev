// §9 PricingGrid (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
// Tests covering every behaviour row of §9:
// - rows given: one PricingRow per row, enabled currencies first in the given order
// - disabled currencies: folded below, behind a toggle, in the given order
// - no rows: an empty text
// - toggle / open: passes onToggle / onOpen through to each row
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import { BOARD } from './__fixtures__/pricing-board.fixture';

vi.mock('./PricingRow', () => ({
  PricingRow: (p: {
    row: PricingRowDto;
    selected: boolean;
    onToggle(currency: string): void;
    onOpen(currency: string): void;
  }) => (
    <div
      data-testid={`row-${p.row.currency}`}
      data-currency={p.row.currency}
      data-enabled={p.row.enabled ? 'true' : 'false'}
      data-selected={p.selected ? 'true' : 'false'}
    >
      <button name={`toggle-${p.row.currency}`} onClick={() => p.onToggle(p.row.currency)}>
        toggle
      </button>
      <button name={`open-${p.row.currency}`} onClick={() => p.onOpen(p.row.currency)}>
        open
      </button>
    </div>
  ),
}));

import { PricingGrid } from './PricingGrid';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount(
  rows: readonly PricingRowDto[],
  selected: ReadonlySet<string> = new Set(),
  onToggle = vi.fn(),
  onOpen = vi.fn(),
) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <PricingGrid rows={rows} selected={selected} onToggle={onToggle} onOpen={onOpen} />,
    host,
  );
  return { onToggle, onOpen };
}

describe('PricingGrid', () => {
  it('G1 rows given: renders one PricingRow per row, enabled currencies first in the given order', () => {
    mount(BOARD.rows);
    const renderedRows = [...host.querySelectorAll<HTMLElement>('[data-testid^="row-"]')];
    const enabledCurrencies = BOARD.rows.filter((r) => r.enabled).map((r) => r.currency);
    const renderedCurrencies = renderedRows.map((el) => el.getAttribute('data-currency'));
    expect(renderedCurrencies).toEqual(enabledCurrencies);
  });

  it('G2 disabled currencies: folded below, behind a toggle, in the given order', async () => {
    mount(BOARD.rows);
    expect(host.querySelector('[data-testid="row-MYR"]')).toBeNull();

    const toggleBtn = host.querySelector<HTMLButtonElement>('button[name="toggleDisabled"]');
    expect(toggleBtn).not.toBeNull();
    expect(toggleBtn?.getAttribute('aria-expanded')).toBe('false');

    toggleBtn?.click();
    await flush();

    expect(toggleBtn?.getAttribute('aria-expanded')).toBe('true');
    const myrRow = host.querySelector('[data-testid="row-MYR"]');
    expect(myrRow).not.toBeNull();
    expect(myrRow?.getAttribute('data-enabled')).toBe('false');
  });

  it('G3 no rows: renders an empty text', () => {
    mount([]);
    const emptyEl = host.querySelector('[data-testid="pricing-grid-empty"]');
    expect(emptyEl).not.toBeNull();
    expect(emptyEl?.textContent?.trim().length).toBeGreaterThan(0);
    expect(host.querySelectorAll('[data-testid^="row-"]')).toHaveLength(0);
  });

  it('G4 toggle / open: passes onToggle / onOpen through to each row with selected state', () => {
    const onToggle = vi.fn();
    const onOpen = vi.fn();
    const selected = new Set(['USD']);
    mount(BOARD.rows, selected, onToggle, onOpen);

    const usdRow = host.querySelector('[data-testid="row-USD"]');
    expect(usdRow?.getAttribute('data-selected')).toBe('true');

    const eurRow = host.querySelector('[data-testid="row-EUR"]');
    expect(eurRow?.getAttribute('data-selected')).toBe('false');

    const toggleBtn = usdRow?.querySelector<HTMLButtonElement>('button[name="toggle-USD"]');
    toggleBtn?.click();
    expect(onToggle).toHaveBeenCalledWith('USD');

    const openBtn = usdRow?.querySelector<HTMLButtonElement>('button[name="open-USD"]');
    openBtn?.click();
    expect(onOpen).toHaveBeenCalledWith('USD');
  });
});
