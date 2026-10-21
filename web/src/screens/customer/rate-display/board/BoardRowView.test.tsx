import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { BoardRow } from './board-model';
import { BoardRowView } from './BoardRowView';

let host: HTMLTableSectionElement;
let dispose: (() => void) | undefined;

function makeRow(buy: string | null, sell: string | null): BoardRow {
  return {
    key: 'USD|ALL',
    code: 'USD',
    group: null,
    unitLabel: '',
    cells: [{ tier: 'ALL', buy, sell, repeat: false }],
  };
}

function mount(buy: string | null = '35.40', sell: string | null = '35.50') {
  const [row, setRow] = createSignal(makeRow(buy, sell));
  host = document.createElement('tbody');
  document.body.appendChild(host);
  dispose = render(() => <BoardRowView row={row()} singleColumn={true} />, host);
  return (nextBuy: string | null, nextSell: string | null = sell) => setRow(makeRow(nextBuy, nextSell));
}

const buyPrice = () => host.querySelector<HTMLElement>('.xc-rb-buy .xc-rb-price')!;
const sellPrice = () => host.querySelector<HTMLElement>('.xc-rb-sell .xc-rb-price')!;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BoardRowView', () => {
  it('first render: no arrow (direction 0)', () => {
    mount('35.40', '35.50');
    expect(buyPrice().classList.contains('xc-rb-up')).toBe(false);
    expect(buyPrice().classList.contains('xc-rb-down')).toBe(false);
    expect(buyPrice().querySelector('em')).toBeNull();
    expect(sellPrice().classList.contains('xc-rb-up')).toBe(false);
    expect(sellPrice().classList.contains('xc-rb-down')).toBe(false);
    expect(sellPrice().querySelector('em')).toBeNull();
  });

  it('35.40 → 35.42 → 35.30: xc-rb-up, then xc-rb-down', () => {
    const update = mount('35.40', '35.50');
    update('35.42');
    expect(buyPrice().classList.contains('xc-rb-up')).toBe(true);
    expect(buyPrice().classList.contains('xc-rb-down')).toBe(false);
    expect(buyPrice().querySelector('em')?.textContent).toBe('▲');

    update('35.30');
    expect(buyPrice().classList.contains('xc-rb-down')).toBe(true);
    expect(buyPrice().classList.contains('xc-rb-up')).toBe(false);
    expect(buyPrice().querySelector('em')?.textContent).toBe('▼');
  });

  it('X → null → Y: no arrow after null', () => {
    const update = mount('35.40', '35.50');
    update(null);
    expect(buyPrice().classList.contains('xc-rb-up')).toBe(false);
    expect(buyPrice().classList.contains('xc-rb-down')).toBe(false);
    expect(buyPrice().querySelector('em')).toBeNull();

    update('35.60');
    expect(buyPrice().classList.contains('xc-rb-up')).toBe(false);
    expect(buyPrice().classList.contains('xc-rb-down')).toBe(false);
    expect(buyPrice().querySelector('em')).toBeNull();
  });

  it('buy moves, sell does not: arrow on buy only', () => {
    const update = mount('35.40', '35.50');
    update('35.45', '35.50');
    expect(buyPrice().classList.contains('xc-rb-up')).toBe(true);
    expect(buyPrice().querySelector('em')?.textContent).toBe('▲');
    expect(sellPrice().classList.contains('xc-rb-up')).toBe(false);
    expect(sellPrice().classList.contains('xc-rb-down')).toBe(false);
    expect(sellPrice().querySelector('em')).toBeNull();
  });
});
