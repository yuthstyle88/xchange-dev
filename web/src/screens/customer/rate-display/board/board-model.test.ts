// RB-1 up-front tests (feat/fe-rateboard skeleton; rate-display.md §9.3, §9.4, rulings on PR 3558): boardModel arranges
// the route's strings into band headings and rows. It formats nothing. The implementer makes these pass and must not
// edit them.
import { describe, expect, it } from 'vitest';
import { bandLabel, boardModel, type BandHeadingIn, type BoardRatesData } from './board-model';

const HEADINGS: BandHeadingIn[] = [
  { tier: 'T1', lowerExclusiveThb: null, upperInclusiveThb: '10000.00' },
  { tier: 'T2', lowerExclusiveThb: '10000.00', upperInclusiveThb: '50000.00' },
  { tier: 'T3', lowerExclusiveThb: '50000.00', upperInclusiveThb: '200000.00' },
  { tier: 'T4', lowerExclusiveThb: '200000.00', upperInclusiveThb: null },
];

function line(currency: string, over: Partial<BoardRatesData['lines'][number]> = {}): BoardRatesData['lines'][number] {
  return { currency, group: 'ALL', unitSize: '1', buyRate: '35.10000000', sellRate: '35.40000000', lineId: `l-${currency}`, ...over };
}

function data(over: Partial<BoardRatesData> = {}): BoardRatesData {
  return {
    publication: { id: 'pub-1', version: 3, publishedAt: '2026-10-19T03:00:00Z' },
    sourceDate: '2026-10-19',
    stale: false,
    lines: [],
    bandHeadings: HEADINGS,
    ...over,
  };
}

const USD_BANDS = [
  { tier: 'T1', buyRate: '35.10', sellRate: '35.40' },
  { tier: 'T2', buyRate: '35.15', sellRate: '35.38' },
  { tier: 'T3', buyRate: '35.20', sellRate: '35.36' },
  { tier: 'T4', buyRate: '35.25', sellRate: '35.34' },
];

describe('bandLabel (RB-1)', () => {
  it('builds the four labels from the bound strings', () => {
    expect(HEADINGS.map(bandLabel)).toEqual(['≤ 10,000 ฿', '10,001–50,000 ฿', '50,001–200,000 ฿', '> 200,000 ฿']);
  });

  it('a bound that is not whole baht is shown as "> lower – upper"', () => {
    expect(bandLabel({ tier: 'T2', lowerExclusiveThb: '10000.50', upperInclusiveThb: '50000.00' })).toBe('> 10,000.50 – 50,000 ฿');
  });
});

describe('boardModel (RB-1)', () => {
  it('a tiered board: four headings in tier order, and a row with one cell per band holding the raw strings', () => {
    const m = boardModel(data({ lines: [line('USD', { bands: USD_BANDS })] }));
    expect(m.singleColumn).toBe(false);
    expect(m.headings.map((h) => h.tier)).toEqual(['T1', 'T2', 'T3', 'T4']);
    expect(m.headings[0]?.label).toBe('≤ 10,000 ฿');
    expect(m.rows).toHaveLength(1);
    expect(m.rows[0]?.cells).toEqual([
      { tier: 'T1', buy: '35.10', sell: '35.40', repeat: false },
      { tier: 'T2', buy: '35.15', sell: '35.38', repeat: false },
      { tier: 'T3', buy: '35.20', sell: '35.36', repeat: false },
      { tier: 'T4', buy: '35.25', sell: '35.34', repeat: false },
    ]);
  });

  it('a line with no bands on a tiered board: its own rate in band 1, the same rate marked repeat in the others', () => {
    const m = boardModel(data({ lines: [line('EUR', { buyRate: '38.00', sellRate: '38.50', bands: [] })] }));
    expect(m.rows[0]?.cells).toEqual([
      { tier: 'T1', buy: '38.00', sell: '38.50', repeat: false },
      { tier: 'T2', buy: '38.00', sell: '38.50', repeat: true },
      { tier: 'T3', buy: '38.00', sell: '38.50', repeat: true },
      { tier: 'T4', buy: '38.00', sell: '38.50', repeat: true },
    ]);
  });

  it('a band missing from a tiered line is an empty cell (null buy and sell), not a repeat', () => {
    const m = boardModel(data({ lines: [line('USD', { bands: USD_BANDS.slice(0, 3) })] }));
    expect(m.rows[0]?.cells[3]).toEqual({ tier: 'T4', buy: null, sell: null, repeat: false });
  });

  it('tiers off (bandHeadings: []): one column, each row one cell from the line (ruling Q1)', () => {
    const m = boardModel(data({ bandHeadings: [], lines: [line('USD'), line('JPY', { unitSize: '100' })] }));
    expect(m.singleColumn).toBe(true);
    expect(m.headings).toEqual([]);
    expect(m.rows.map((r) => r.cells)).toEqual([
      [{ tier: 'ALL', buy: '35.10000000', sell: '35.40000000', repeat: false }],
      [{ tier: 'ALL', buy: '35.10000000', sell: '35.40000000', repeat: false }],
    ]);
  });

  it('an answer from today\'s route, without bandHeadings or bands at all, reads as tiers off', () => {
    const d = data({ lines: [line('USD')] });
    delete d.bandHeadings;
    const m = boardModel(d);
    expect(m.singleColumn).toBe(true);
    expect(m.rows[0]?.cells).toHaveLength(1);
  });

  it('keeps the route order, keys rows by currency and group, and shows a non-ALL group under the code (v1 Q-2)', () => {
    const m = boardModel(data({ lines: [line('USD'), line('USD', { group: 'G100', lineId: 'l-g' }), line('EUR')] }));
    expect(m.rows.map((r) => [r.key, r.code, r.group])).toEqual([
      ['USD|ALL', 'USD', null],
      ['USD|G100', 'USD', 'G100'],
      ['EUR|ALL', 'EUR', null],
    ]);
  });

  it('the unit label: 100 JPY and 1000 KRW, nothing for a unit of 1 (also written 1.0000)', () => {
    const m = boardModel(
      data({ bandHeadings: [], lines: [line('JPY', { unitSize: '100.0000' }), line('KRW', { unitSize: '1000' }), line('USD', { unitSize: '1.0000' })] }),
    );
    expect(m.rows.map((r) => r.unitLabel)).toEqual(['100 JPY', '1000 KRW', '']);
  });

  it('no lines: no rows', () => {
    expect(boardModel(data({ lines: [] })).rows).toEqual([]);
  });
});
