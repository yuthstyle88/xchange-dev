// #5202 up-front tests (V.9 PV-i): boardModel follows the route's `display` setting (tiers, order, featured,
// contact-the-counter rows). With no `display` (an older server), the board behaves exactly as today.
// The implementer makes these pass and must not edit them.
import { describe, expect, it } from 'vitest';
import {
  makePublishedBandDto,
  makePublishedBandHeadingDto,
  makePublishedDisplayDto,
  makePublishedLineDto,
  makePublishedRatesData,
} from '../../../../shared/testing/fixtures';
import { boardModel, type BoardRatesData } from './board-model';

const HEADINGS = [
  makePublishedBandHeadingDto({ tier: 'T1', lowerExclusiveThb: null, upperInclusiveThb: '10000.00' }),
  makePublishedBandHeadingDto({ tier: 'T2', lowerExclusiveThb: '10000.00', upperInclusiveThb: '50000.00' }),
  makePublishedBandHeadingDto({ tier: 'T3', lowerExclusiveThb: '50000.00', upperInclusiveThb: '200000.00' }),
  makePublishedBandHeadingDto({ tier: 'T4', lowerExclusiveThb: '200000.00', upperInclusiveThb: null }),
];

const USD_BANDS = [
  makePublishedBandDto({ tier: 'T1', buyRate: '35.10', sellRate: '35.40' }),
  makePublishedBandDto({ tier: 'T2', buyRate: '35.15', sellRate: '35.38' }),
  makePublishedBandDto({ tier: 'T3', buyRate: '35.20', sellRate: '35.36' }),
  makePublishedBandDto({ tier: 'T4', buyRate: '35.25', sellRate: '35.34' }),
];

function line(currency: string, group = 'ALL', over: Parameters<typeof makePublishedLineDto>[0] = {}) {
  return makePublishedLineDto({
    currency,
    group,
    unitSize: '1',
    buyRate: '10.00',
    sellRate: '11.00',
    lineId: `l-${currency}-${group}`,
    pricingSource: 'AUTO',
    ...over,
  });
}

/** A board answer with all four headings and a neutral display (every tier, nothing featured, no order, no more). */
function data(over: Partial<BoardRatesData> = {}): BoardRatesData {
  return {
    ...makePublishedRatesData(),
    bandHeadings: HEADINGS,
    display: makePublishedDisplayDto({ tiers: [1, 2, 3, 4], featured: [], highlight: true, order: [], more: [] }),
    ...over,
  };
}

function display(over: Parameters<typeof makePublishedDisplayDto>[0] = {}) {
  return makePublishedDisplayDto({ tiers: [1, 2, 3, 4], featured: [], highlight: true, order: [], more: [], ...over });
}

/** The same answer as an older server sends it: no `display` at all. */
function withoutDisplay(d: BoardRatesData): BoardRatesData {
  const copy = { ...d };
  delete copy.display;
  return copy;
}

describe('boardModel with display (#5202)', () => {
  it('D1 no display: headings, row order and cells are exactly as today', () => {
    const m = boardModel(
      withoutDisplay(
        data({
          lines: [
            line('JPY', 'ALL', { unitSize: '100' }),
            line('USD', 'ALL', { bands: USD_BANDS }),
            line('EUR', 'ALL', { buyRate: '38.00', sellRate: '38.50', hidden: true }),
          ],
        }),
      ),
    );
    expect(m.singleColumn).toBe(false);
    expect(m.headings).toEqual([
      { tier: 'T1', label: '≤ 10,000 ฿' },
      { tier: 'T2', label: '10,001–50,000 ฿' },
      { tier: 'T3', label: '50,001–200,000 ฿' },
      { tier: 'T4', label: '> 200,000 ฿' },
    ]);
    expect(m.rows.map((r) => [r.key, r.code, r.group, r.unitLabel])).toEqual([
      ['JPY|ALL', 'JPY', null, '100 JPY'],
      ['USD|ALL', 'USD', null, ''],
      ['EUR|ALL', 'EUR', null, ''],
    ]);
    expect(m.rows.map((r) => r.cells)).toEqual([
      [
        { tier: 'T1', buy: '10.00', sell: '11.00', repeat: false },
        { tier: 'T2', buy: '10.00', sell: '11.00', repeat: true },
        { tier: 'T3', buy: '10.00', sell: '11.00', repeat: true },
        { tier: 'T4', buy: '10.00', sell: '11.00', repeat: true },
      ],
      [
        { tier: 'T1', buy: '35.10', sell: '35.40', repeat: false },
        { tier: 'T2', buy: '35.15', sell: '35.38', repeat: false },
        { tier: 'T3', buy: '35.20', sell: '35.36', repeat: false },
        { tier: 'T4', buy: '35.25', sell: '35.34', repeat: false },
      ],
      [
        { tier: 'T1', buy: '38.00', sell: '38.50', repeat: false },
        { tier: 'T2', buy: '38.00', sell: '38.50', repeat: true },
        { tier: 'T3', buy: '38.00', sell: '38.50', repeat: true },
        { tier: 'T4', buy: '38.00', sell: '38.50', repeat: true },
      ],
    ]);
  });

  it('D1 no display: every row has featured false and contactOnly false', () => {
    const m = boardModel(withoutDisplay(data({ lines: [line('USD'), line('EUR')] })));
    expect(m.rows.map((r) => [r.featured, r.contactOnly])).toEqual([
      [false, false],
      [false, false],
    ]);
  });

  it('D2 a line with hidden: true is not in rows', () => {
    const m = boardModel(data({ lines: [line('USD'), line('EUR', 'ALL', { hidden: true }), line('JPY')] }));
    expect(m.rows.map((r) => r.key)).toEqual(['USD|ALL', 'JPY|ALL']);
  });

  it('D3 headings T1..T4, tiers [1, 3]: headings T1, T3 only, and each row has the T1 and T3 cells', () => {
    const m = boardModel(
      data({ display: display({ tiers: [1, 3] }), lines: [line('USD', 'ALL', { bands: USD_BANDS }), line('EUR')] }),
    );
    expect(m.headings).toEqual([
      { tier: 'T1', label: '≤ 10,000 ฿' },
      { tier: 'T3', label: '50,001–200,000 ฿' },
    ]);
    expect(m.rows[0]?.cells).toEqual([
      { tier: 'T1', buy: '35.10', sell: '35.40', repeat: false },
      { tier: 'T3', buy: '35.20', sell: '35.36', repeat: false },
    ]);
    expect(m.rows.map((r) => r.cells.map((c) => c.tier))).toEqual([
      ['T1', 'T3'],
      ['T1', 'T3'],
    ]);
  });

  it('D4 lines USD, EUR, JPY with order ["JPY", "USD"]: rows JPY, USD, EUR', () => {
    const m = boardModel(data({ display: display({ order: ['JPY', 'USD'] }), lines: [line('USD'), line('EUR'), line('JPY')] }));
    expect(m.rows.map((r) => r.code)).toEqual(['JPY', 'USD', 'EUR']);
  });

  it('D5 USD with two lines (50/100 and 1–20): they stay adjacent, in their route order', () => {
    const m = boardModel(
      data({
        display: display({ order: ['USD', 'EUR'] }),
        lines: [line('USD', '50/100'), line('EUR'), line('USD', '1–20')],
      }),
    );
    expect(m.rows.map((r) => r.key)).toEqual(['USD|50/100', 'USD|1–20', 'EUR|ALL']);
  });

  it('D6 featured ["USD"] with highlight true: the USD row is featured, the others are not', () => {
    const m = boardModel(
      data({ display: display({ featured: ['USD'], highlight: true }), lines: [line('EUR'), line('USD'), line('JPY')] }),
    );
    expect(m.rows.map((r) => [r.code, r.featured])).toEqual([
      ['EUR', false],
      ['USD', true],
      ['JPY', false],
    ]);
  });

  it('D7 featured ["USD"] with highlight false: no row is featured', () => {
    const m = boardModel(
      data({ display: display({ featured: ['USD'], highlight: false }), lines: [line('EUR'), line('USD'), line('JPY')] }),
    );
    expect(m.rows.map((r) => r.featured)).toEqual([false, false, false]);
  });

  it('D8 more ["KRW", "VND"]: two contact rows after the priced rows, contactOnly, no cells, keys <code>|more', () => {
    const m = boardModel(data({ display: display({ more: ['KRW', 'VND'] }), lines: [line('USD'), line('EUR')] }));
    expect(m.rows).toHaveLength(4);
    expect(m.rows.slice(0, 2).map((r) => [r.key, r.contactOnly])).toEqual([
      ['USD|ALL', false],
      ['EUR|ALL', false],
    ]);
    expect(m.rows.slice(2).map((r) => [r.key, r.code, r.contactOnly, r.cells])).toEqual([
      ['KRW|more', 'KRW', true, []],
      ['VND|more', 'VND', true, []],
    ]);
  });
});
