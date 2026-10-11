// RD-3 (P01 public rate display): `boardRows` turns the public rates route's answer into the board's rows.
// Design `docs/design/blueprint/rate-display.md` D6, §3 (`BoardRows`, `BoardRow`), §4.3, owner Q-2 A.
// Written before the implementation by the test writer (claude/5d45b017). Do not edit, delete or weaken these tests;
// add your own in another file.
//
// The API this file pins, in `board-rows.ts` ([required], design §3):
//
//   export interface BoardRow { currency: string; group: string | null; unitLabel: string; buy: string; sell: string }
//   export function boardRows(data: PublishedRatesData): BoardRow[]
//
// - one row per published line, in the route's order (no sorting);
// - `unitLabel` = `unitSize` + ' ' + code (the route sends `unitSize` normalised, `"100"`, app_public_reads_test.rs:445);
// - `buy` / `sell` = `formatRate(rate, '1', code)` (number only; the unit has its own column), `—` for a null side;
// - `group` is `null` for `ALL` and the group's name otherwise (Q-2 A: such a line gets its own row);
// - no arithmetic on a rate: a float would change `12345678.12345678`.
import { describe, expect, it } from 'vitest';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import { formatRate } from '../../../shared/ui/rate';
import { boardRows } from './board-rows';

function line(over: Partial<PublishedLineDto> & Pick<PublishedLineDto, 'currency'>): PublishedLineDto {
  return {
    group: 'ALL',
    unitSize: '1',
    buyRate: '1.00000000',
    sellRate: '1.10000000',
    lineId: `line-${over.currency}-${over.group ?? 'ALL'}`,
    ...over,
  };
}

function data(lines: PublishedLineDto[]): PublishedRatesData {
  return {
    publication: { id: 'pub-1', version: 7, publishedAt: '2026-10-11T02:00:00Z' },
    sourceDate: '2026-10-10',
    stale: false,
    lines,
  };
}

const USD = line({ currency: 'USD', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000' });
const JPY = line({ currency: 'JPY', unitSize: '100', buyRate: '21.34500000', sellRate: '22.10000000' });
const KRW = line({ currency: 'KRW', unitSize: '1000', buyRate: '25.00000000', sellRate: '26.50000000' });

describe('boardRows: one row per line, in the route order (D6)', () => {
  it('keeps the route order and gives one row per line', () => {
    const rows = boardRows(data([JPY, KRW, USD]));
    expect(rows.map((r) => r.currency)).toEqual(['JPY', 'KRW', 'USD']);
  });

  it('labels the unit as unitSize and code: 1 USD, 100 JPY, 1000 KRW', () => {
    const rows = boardRows(data([USD, JPY, KRW]));
    expect(rows.map((r) => r.unitLabel)).toEqual(['1 USD', '100 JPY', '1000 KRW']);
  });

  it("shows each rate through formatRate(rate, '1', code): USD buy 35.12, JPY buy 21.345", () => {
    const rows = boardRows(data([USD, JPY]));
    expect(rows).toHaveLength(2);
    const [usd, jpy] = rows;
    expect(usd!.buy).toBe('35.12');
    expect(usd!.sell).toBe('35.42');
    expect(jpy!.buy).toBe('21.345');
    expect(jpy!.buy).toBe(formatRate('21.34500000', '1', 'JPY'));
    // the unit is its own column: the rate cell never carries "/ 100 JPY"
    expect(jpy!.buy).not.toContain('/');
    expect(jpy!.sell).not.toContain('JPY');
  });

  it('keeps every digit of a long rate, including one a float cannot hold', () => {
    // §4.3's example 12345678.12345678 has 16 significant digits, which a double still prints back exactly, so it
    // alone cannot catch a float. 1234567890.12345678 (17 digits, inside numeric(20,8)) can: parseFloat gives
    // 1234567890.1234567.
    const rows = boardRows(
      data([
        line({ currency: 'BTC', buyRate: '12345678.12345678', sellRate: '99999999.99999999' }),
        line({ currency: 'XAU', buyRate: '1234567890.12345678', sellRate: '1234567890.12345679' }),
      ]),
    );
    expect(rows).toHaveLength(2);
    const [btc, xau] = rows;
    expect(btc!.buy).toBe('12345678.12345678');
    expect(btc!.sell).toBe('99999999.99999999');
    expect(xau!.buy).toBe('1234567890.12345678');
    expect(xau!.sell).toBe('1234567890.12345679');
    // the trap a float falls into
    expect(String(parseFloat('1234567890.12345678'))).not.toBe('1234567890.12345678');
  });

  it('shows a null side as —, and the other side as published', () => {
    const both = boardRows(
      data([
        line({ currency: 'EUR', buyRate: '38.00000000', sellRate: null }),
        line({ currency: 'GBP', buyRate: null, sellRate: '44.50000000' }),
      ]),
    );
    expect(both).toHaveLength(2);
    const [onlyBuy, onlySell] = both;
    expect(onlyBuy!.buy).toBe('38.00');
    expect(onlyBuy!.sell).toBe('—');
    expect(onlySell!.buy).toBe('—');
    expect(onlySell!.sell).toBe('44.50');
  });
});

describe('boardRows: a banknote group is its own row (owner Q-2 A)', () => {
  it('gives the ALL line group null and the G100 line its own row named G100, under the same currency', () => {
    const rows = boardRows(
      data([USD, line({ currency: 'USD', group: 'G100', buyRate: '35.20000000', sellRate: '35.40000000' })]),
    );
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ currency: 'USD', group: null, buy: '35.12' });
    expect(rows[1]).toMatchObject({ currency: 'USD', group: 'G100', buy: '35.20', unitLabel: '1 USD' });
  });

  it('an empty publication gives no rows', () => {
    expect(boardRows({ publication: null, sourceDate: null, stale: false, lines: [] })).toEqual([]);
  });
});
