// RB-7 tests (rate-display.md §9.2, §9.4, §9.6): band headings, currency rows, key preservation, and attract motion.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { BoardModel } from './board-model';
import { BoardTable } from './BoardTable';

const FOUR_HEADINGS_MODEL: BoardModel = {
  headings: [
    { tier: 'T1', label: '≤ 10,000 ฿' },
    { tier: 'T2', label: '10,001–50,000 ฿' },
    { tier: 'T3', label: '50,001–200,000 ฿' },
    { tier: 'T4', label: '> 200,000 ฿' },
  ],
  singleColumn: false,
  rows: [
    {
      key: 'USD|ALL',
      code: 'USD',
      group: null,
      unitLabel: '',
      cells: [
        { tier: 'T1', buy: '35.10', sell: '35.40', repeat: false },
        { tier: 'T2', buy: '35.12', sell: '35.38', repeat: false },
        { tier: 'T3', buy: '35.15', sell: '35.35', repeat: false },
        { tier: 'T4', buy: '35.20', sell: '35.30', repeat: false },
      ],
    },
    {
      key: 'JPY|G100',
      code: 'JPY',
      group: 'G100',
      unitLabel: '100 JPY',
      cells: [
        { tier: 'T1', buy: '21.35', sell: '22.10', repeat: false },
        { tier: 'T2', buy: '21.35', sell: '22.10', repeat: true },
        { tier: 'T3', buy: '21.35', sell: '22.10', repeat: true },
        { tier: 'T4', buy: '21.35', sell: '22.10', repeat: true },
      ],
    },
  ],
};

const SINGLE_COLUMN_MODEL: BoardModel = {
  headings: [],
  singleColumn: true,
  rows: [
    {
      key: 'USD|ALL',
      code: 'USD',
      group: null,
      unitLabel: '',
      cells: [{ tier: 'ALL', buy: '35.12', sell: '35.42', repeat: false }],
    },
  ],
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
});

describe('BoardTable (RB-7)', () => {
  it('model with 4 headings, 2 rows: 5 <th> in <thead>; 2 <tr> in <tbody>; 4 <td> per row', () => {
    dispose = render(() => <BoardTable model={FOUR_HEADINGS_MODEL} attract={false} />, host);
    const theadThs = host.querySelectorAll('table thead th');
    expect(theadThs).toHaveLength(5);
    expect(theadThs[0]?.textContent).toContain('สกุลเงิน');
    expect(theadThs[0]?.textContent).toContain('Currency');
    expect(theadThs[1]?.textContent).toContain('≤ 10,000 ฿');
    expect(theadThs[1]?.textContent).toContain('รับซื้อ · ขาย');
    expect(theadThs[4]?.textContent).toContain('> 200,000 ฿');

    const tbodyRows = host.querySelectorAll('table tbody tr');
    expect(tbodyRows).toHaveLength(2);
    expect(tbodyRows[0]?.querySelectorAll('td')).toHaveLength(4);
    expect(tbodyRows[1]?.querySelectorAll('td')).toHaveLength(4);
  });

  it('singleColumn: one band heading ทุกยอด · All amounts; 1 <td> per row', () => {
    dispose = render(() => <BoardTable model={SINGLE_COLUMN_MODEL} attract={false} />, host);
    const theadThs = host.querySelectorAll('table thead th');
    expect(theadThs).toHaveLength(2);
    expect(theadThs[1]?.textContent).toContain('ทุกยอด · All amounts');

    const tbodyRows = host.querySelectorAll('table tbody tr');
    expect(tbodyRows).toHaveLength(1);
    expect(tbodyRows[0]?.querySelectorAll('td')).toHaveLength(1);
  });

  it('repeat cells: .xc-rb-band-repeat on bands 2..4', () => {
    dispose = render(() => <BoardTable model={FOUR_HEADINGS_MODEL} attract={false} />, host);
    const tbodyRows = host.querySelectorAll('table tbody tr');
    const usdTds = Array.from(tbodyRows[0]!.querySelectorAll('td'));
    expect(usdTds.map((td) => td.classList.contains('xc-rb-band-repeat'))).toEqual([false, false, false, false]);

    const jpyTds = Array.from(tbodyRows[1]!.querySelectorAll('td'));
    expect(jpyTds.map((td) => td.classList.contains('xc-rb-band-repeat'))).toEqual([false, true, true, true]);
  });

  it('group G100 / unit 100 JPY: shown in the row heading', () => {
    dispose = render(() => <BoardTable model={FOUR_HEADINGS_MODEL} attract={false} />, host);
    const tbodyRows = host.querySelectorAll('table tbody tr');
    const usdRowHeader = tbodyRows[0]!.querySelector('th[scope="row"]')!;
    expect(usdRowHeader.textContent).toContain('USD');
    expect(usdRowHeader.textContent).not.toContain('G100');

    const jpyRowHeader = tbodyRows[1]!.querySelector('th[scope="row"]')!;
    expect(jpyRowHeader.textContent).toContain('JPY');
    expect(jpyRowHeader.textContent).toContain('G100');
    expect(jpyRowHeader.textContent).toContain('100 JPY');
  });

  it('a row object replaced by the same key on a poll: the <tr> node is kept', () => {
    const [model, setModel] = createSignal<BoardModel>(FOUR_HEADINGS_MODEL);
    dispose = render(() => <BoardTable model={model()} attract={false} />, host);

    const beforeRows = Array.from(host.querySelectorAll('table tbody tr'));
    expect(beforeRows).toHaveLength(2);
    const beforeTd = beforeRows[0]!.querySelector('td')!;

    setModel({
      ...FOUR_HEADINGS_MODEL,
      rows: [
        {
          ...FOUR_HEADINGS_MODEL.rows[0]!,
          cells: [
            { tier: 'T1', buy: '35.10', sell: '35.45', repeat: false },
            ...FOUR_HEADINGS_MODEL.rows[0]!.cells.slice(1),
          ],
        },
        { ...FOUR_HEADINGS_MODEL.rows[1]! },
      ],
    });

    const afterRows = Array.from(host.querySelectorAll('table tbody tr'));
    expect(afterRows[0]).toBe(beforeRows[0]);
    expect(afterRows[1]).toBe(beforeRows[1]);
    expect(afterRows[0]!.querySelector('td')).toBe(beforeTd);
    expect(afterRows[0]!.querySelector('td')!.textContent).toContain('35.45');
  });

  it('attract=false: no spotlight element animating (and attract=true walks every 1.9 s and waves every 70 s)', () => {
    const [attract, setAttract] = createSignal(true);
    dispose = render(() => <BoardTable model={FOUR_HEADINGS_MODEL} attract={attract()} />, host);

    vi.advanceTimersByTime(1900);
    const rows = Array.from(host.querySelectorAll('table tbody tr'));
    expect(rows[0]!.classList.contains('xc-rb-lit')).toBe(true);
    expect(host.querySelector('.xc-rb-spot.xc-rb-spot-on')).not.toBeNull();

    vi.advanceTimersByTime(1900);
    expect(rows[0]!.classList.contains('xc-rb-lit')).toBe(false);
    expect(rows[1]!.classList.contains('xc-rb-lit')).toBe(true);

    vi.advanceTimersByTime(70000);
    expect(host.querySelectorAll('.xc-rb-flip').length).toBeGreaterThan(0);

    setAttract(false);
    vi.advanceTimersByTime(1900);
    expect(host.querySelector('.xc-rb-spot-on')).toBeNull();
    expect(host.querySelector('.xc-rb-lit')).toBeNull();
  });
});
