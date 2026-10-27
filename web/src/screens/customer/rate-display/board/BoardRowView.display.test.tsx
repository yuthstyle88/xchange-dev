// #5202 up-front tests (V.9 PV-i): a featured row is highlighted, and a "contact the counter" row shows the label
// instead of prices. The implementer makes these pass and must not edit them.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { BoardRow } from './board-model';
import { BoardRowView } from './BoardRowView';

let host: HTMLTableSectionElement;
let dispose: (() => void) | undefined;

function makeRow(over: Partial<BoardRow> = {}): BoardRow {
  return {
    key: 'USD|ALL',
    code: 'USD',
    group: null,
    unitLabel: '',
    cells: [{ tier: 'ALL', buy: '35.40', sell: '35.50', repeat: false }],
    featured: false,
    contactOnly: false,
    ...over,
  };
}

function mount(row: BoardRow, singleColumn = true) {
  host = document.createElement('tbody');
  document.body.appendChild(host);
  dispose = render(() => <BoardRowView row={row} singleColumn={singleColumn} />, host);
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BoardRowView with display (#5202)', () => {
  it('V1 a featured row: the row element has class xc-rb-row-featured', () => {
    mount(makeRow({ featured: true }));
    const tr = host.querySelector('tr');
    expect(tr).not.toBeNull();
    expect(tr!.classList.contains('xc-rb-row-featured')).toBe(true);
  });

  it('V2 a contact row: shows ติดต่อเคาน์เตอร์ and Contact the counter, no price and no 0', () => {
    mount(makeRow({ key: 'KRW|more', code: 'KRW', cells: [], contactOnly: true }), false);
    const text = host.textContent ?? '';
    expect(text).toContain('KRW');
    expect(text).toContain('ติดต่อเคาน์เตอร์');
    expect(text).toContain('Contact the counter');
    expect(host.querySelector('.xc-rb-price')).toBeNull();
    expect(text).not.toContain('0');
  });
});
