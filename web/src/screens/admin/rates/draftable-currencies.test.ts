// BC-6 (xchange#6085): the currencies a rate draft can add come from the rates_draftable view.
// Design: docs/design/board-currencies.md § BC-6.
import { describe, expect, it } from 'vitest';
import { makeDraftableCurrency, makeRatesDraftableData } from '../../../shared/testing/fixtures';
import { draftableCurrencies } from './draftable-currencies';

const draftable = (...codes: string[]) =>
  makeRatesDraftableData({
    rows: codes.map((code) => makeDraftableCurrency({ code, group: 'ALL', unitSize: '1', hasReference: true })),
  });

describe('draftableCurrencies(data, inDraft)', () => {
  it('D1: the draftable codes not yet in the draft, A to Z', () => {
    expect(draftableCurrencies(draftable('USD', 'JPY', 'EUR', 'CNY'), ['USD'])).toEqual(['CNY', 'EUR', 'JPY']);
  });

  it('D2: a code already in the draft is never offered', () => {
    expect(draftableCurrencies(draftable('USD', 'EUR'), ['USD', 'EUR'])).toEqual([]);
  });

  it('D3: no view data (not read, or unreadable) offers nothing', () => {
    expect(draftableCurrencies(null, ['USD'])).toEqual([]);
  });

  it('D4: THB, blank codes and duplicates are dropped; codes are trimmed and upper-cased', () => {
    expect(draftableCurrencies(draftable('THB', '', ' eur ', 'EUR', 'jpy'), [])).toEqual(['EUR', 'JPY']);
  });

  it('D5: a branch with no publication yet (empty draft) is offered every draftable code', () => {
    expect(draftableCurrencies(draftable('USD', 'EUR'), [])).toEqual(['EUR', 'USD']);
  });
});
