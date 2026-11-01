// DC-1 (xchange#564 D-a gap): the currencies a rate draft can add come from the reference source, not from the last
// publication. Up-front tests by the FE sub-controller claude/01TxaRJG. Do not edit, delete or weaken.
// Design: docs/design/rates-draftable-currencies.md §2.
import { describe, expect, it } from 'vitest';
import { makeRatesReferenceData, makeRatesReferenceRow } from '../../../shared/testing/fixtures';
import { draftableCurrencies } from './draftable-currencies';

const ref = (...codes: string[]) => makeRatesReferenceData({ rows: codes.map((code) => makeRatesReferenceRow({ code })) });

describe('draftableCurrencies(reference, inDraft)', () => {
  it('D1: the reference codes not yet in the draft, A to Z', () => {
    expect(draftableCurrencies(ref('USD', 'JPY', 'EUR', 'CNY'), ['USD'])).toEqual(['CNY', 'EUR', 'JPY']);
  });

  it('D2: a code already in the draft is never offered', () => {
    expect(draftableCurrencies(ref('USD', 'EUR'), ['USD', 'EUR'])).toEqual([]);
  });

  it('D3: no reference (not read, or unreadable) offers nothing', () => {
    expect(draftableCurrencies(null, ['USD'])).toEqual([]);
  });

  it('D4: THB, blank codes and duplicates are dropped; codes are trimmed and upper-cased', () => {
    expect(draftableCurrencies(ref('THB', '', ' eur ', 'EUR', 'jpy'), [])).toEqual(['EUR', 'JPY']);
  });

  it('D5: a branch with no publication yet (empty draft) is offered every reference code', () => {
    expect(draftableCurrencies(ref('USD', 'EUR'), [])).toEqual(['EUR', 'USD']);
  });
});
