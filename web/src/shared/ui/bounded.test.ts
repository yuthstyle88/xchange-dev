// T1 (#4898): the web mirror of the kernel's `Bound` / `IntBound` and of `Money::parse`, `Rate::parse`,
// `RatePercent::parse` and `ForeignAmount::parse` (design docs/design/web-input-validation.md §3,
// docs/design/blueprint/bounded-inputs.md B-1/B-2/B-7). Rows B1-B9 are the ticket's §3 table; the
// other cases are copied from the kernel's pinned tests (api/crates/xc-kernel/tests/bounded_test.rs,
// api/crates/xc-kernel/tests/k2_typed_parsers_test.rs) with the same inputs and results.
// Written before the implementation by the XC FE lane controller (claude/e8020026).
// Do not edit, delete or weaken; add tests in a file of your own.
import { describe, expect, it } from 'vitest';
import {
  MONEY,
  RATE,
  RATE_PERCENT,
  atMost,
  checkDecimal,
  checkInteger,
  foreignAmount,
  numeric,
  type Bound,
  type BoundError,
} from './bounded';

function ok(text: string, b: Bound) {
  expect(checkDecimal(text, b), JSON.stringify(text)).toEqual({ ok: true, text });
}

function err(text: string, b: Bound, error: BoundError) {
  expect(checkDecimal(text, b), JSON.stringify(text)).toMatchObject({ ok: false, error });
}

function tooMany(text: string, b: Bound, scale: number) {
  expect(checkDecimal(text, b), JSON.stringify(text)).toEqual({
    ok: false,
    error: 'tooManyDecimals',
    scale,
  });
}

// numeric(20,2) non-negative: the cash-limit / money column shape (bounded_test.rs N20_2).
const N20_2 = numeric(20, 2, 'nonNegative');

// ================================================================ §3 behaviour table

describe('§3 behaviour table', () => {
  it('B1: MONEY refuses text that is not a plain decimal as syntax, and never trims', () => {
    for (const t of ['', ' 1', '1 ', '.5', '5.', '1e5', '1,000']) err(t, MONEY, 'syntax');
  });

  it('B2: "+1" is syntax for MONEY and accepted, unchanged, for RATE_PERCENT', () => {
    err('+1', MONEY, 'syntax');
    ok('+1', RATE_PERCENT);
  });

  it('B3: "-1" is sign for MONEY and accepted for RATE_PERCENT', () => {
    err('-1', MONEY, 'sign');
    ok('-1', RATE_PERCENT);
  });

  it('B4: MONEY accepts extra zero decimals and refuses a non-zero third decimal', () => {
    ok('1.230', MONEY);
    tooMany('1.234', MONEY, 2);
  });

  it('B5: MONEY and RATE accept 100000000 and refuse 100000000.01 as outOfRange', () => {
    for (const b of [MONEY, RATE]) {
      ok('100000000', b);
      err('100000000.01', b, 'outOfRange');
    }
  });

  it('B6: "0" is sign for RATE (positive) and accepted for MONEY', () => {
    err('0', RATE, 'sign');
    ok('0', MONEY);
  });

  it('B7: "1.123" is accepted by foreignAmount(3) and tooManyDecimals for foreignAmount(0)', () => {
    ok('1.123', foreignAmount(3));
    tooMany('1.123', foreignAmount(0), 0);
  });

  it('B8: a 19-digit value is judged on its digits, with no precision loss', () => {
    err('9999999999999999999', numeric(20, 4, 'positive'), 'outOfRange');
  });

  it('B9: checkInteger refuses hex and decimals as syntax', () => {
    for (const t of ['0x10', '1.5']) {
      expect(checkInteger(t, 3, 120), t).toMatchObject({ ok: false, error: 'syntax' });
    }
  });
});

// ================================================================ Bound::parse (bounded_test.rs)

describe('checkDecimal mirrors Bound::parse', () => {
  it('numeric(p, s) keeps p - s integer digits, s decimals and the sign', () => {
    expect(numeric(20, 2, 'nonNegative')).toMatchObject({ intDigits: 18, scale: 2, sign: 'nonNegative' });
    expect(numeric(9, 4, 'signed')).toMatchObject({ intDigits: 5, scale: 4, sign: 'signed' });
    expect(numeric(20, 8, 'positive')).toMatchObject({ intDigits: 12, scale: 8, sign: 'positive' });
  });

  it('syntax that is not a plain decimal is refused under every sign', () => {
    const texts = ['', ' 1', '1 ', '+1', '.5', '5.', '1e5', '1.2.3', '1,000', 'abc', '-', '--1', '0x10'];
    for (const b of [numeric(20, 2, 'positive'), numeric(20, 2, 'nonNegative'), numeric(9, 4, 'signed')]) {
      for (const t of texts) err(t, b, 'syntax');
    }
  });

  it('plain decimals are accepted and returned as typed', () => {
    for (const t of ['0', '7', '100.5', '0.07', '00012.30']) ok(t, N20_2);
  });

  it('extra decimals that are zeros are accepted', () => {
    ok('1.230', N20_2);
    ok('1.2300000', N20_2);
  });

  it('a non-zero extra decimal is tooManyDecimals with the scale', () => {
    tooMany('1.234', N20_2, 2);
    tooMany('0.001', N20_2, 2);
    tooMany('0.00001', numeric(20, 4, 'positive'), 4);
    tooMany('1.5', numeric(20, 0, 'positive'), 0);
  });

  it('positive refuses zero and negatives as sign', () => {
    const b = numeric(20, 4, 'positive');
    for (const t of ['0', '0.0000', '-1', '-0.00']) err(t, b, 'sign');
    ok('0.0001', b);
  });

  it('nonNegative accepts zero and refuses negatives as sign', () => {
    ok('0', N20_2);
    ok('0.00', N20_2);
    err('-1', N20_2, 'sign');
    err('-0.00', N20_2, 'sign');
  });

  it('signed accepts a leading minus and never a plus', () => {
    const b = numeric(9, 4, 'signed');
    ok('-1.25', b);
    ok('-0.00', b);
    ok('3', b);
    err('+3', b, 'syntax');
  });

  it('numeric(20,2) takes everything below 10^18', () => {
    ok('999999999999999999.99', N20_2);
    err('1000000000000000000', N20_2, 'outOfRange');
    err('1000000000000000000.00', N20_2, 'outOfRange');
  });

  it('numeric(9,4) signed is strict on both sides', () => {
    const b = numeric(9, 4, 'signed');
    ok('99999.9999', b);
    ok('-99999.9999', b);
    err('100000', b, 'outOfRange');
    err('-100000', b, 'outOfRange');
  });

  it('numeric(20,8) takes everything below 10^12', () => {
    const b = numeric(20, 8, 'positive');
    ok('999999999999.99999999', b);
    err('1000000000000', b, 'outOfRange');
  });

  it('numeric(20,4) takes 9999999999999999.9999 and refuses 10^16 (UnitSize, no precision loss)', () => {
    const b = numeric(20, 4, 'positive');
    ok('9999999999999999.9999', b);
    err('10000000000000000', b, 'outOfRange');
    err('9'.repeat(30), b, 'outOfRange');
  });

  it('a number with 40 integer digits is outOfRange, not syntax', () => {
    const huge = '1'.repeat(40);
    err(huge, N20_2, 'outOfRange');
    err(`-${huge}`, numeric(9, 4, 'signed'), 'outOfRange');
  });

  it('atMost is inclusive', () => {
    const b = atMost(numeric(20, 2, 'nonNegative'), '100000000');
    ok('100000000', b);
    ok('100000000.00', b);
    err('100000000.01', b, 'outOfRange');
    err('100000001', b, 'outOfRange');
  });

  it('atMost applies to the absolute value under signed', () => {
    const b = atMost(numeric(20, 2, 'signed'), '500');
    ok('-500', b);
    err('-500.01', b, 'outOfRange');
  });

  it('syntax is reported before scale, sign or range', () => {
    err('-1.234x', numeric(20, 2, 'positive'), 'syntax');
  });
});

// ================================================================ the presets (k2_typed_parsers_test.rs)

// The texts Money, Rate and ForeignAmount all refuse as Invalid, with the BoundError behind each.
const INVALID_FOR_ALL: [string, BoundError][] = [
  ['', 'syntax'],
  [' 1', 'syntax'],
  ['1 ', 'syntax'],
  ['+1', 'syntax'],
  ['-1', 'sign'],
  ['-0.00', 'sign'],
  ['.5', 'syntax'],
  ['5.', 'syntax'],
  ['1e5', 'syntax'],
  ['1.2.3', 'syntax'],
  ['abc', 'syntax'],
];

describe('MONEY mirrors Money::parse', () => {
  it('refuses the same texts as the kernel', () => {
    for (const [t, e] of INVALID_FOR_ALL) err(t, MONEY, e);
    tooMany('1.234', MONEY, 2);
  });

  it('accepts zero, one decimal and extra zero decimals, returning the text unchanged', () => {
    for (const t of ['0', '1.5', '1.230', '100.5']) ok(t, MONEY);
  });

  it('the cap is inclusive at 100000000', () => {
    ok('100000000', MONEY);
    ok('100000000.00', MONEY);
    err('100000000.01', MONEY, 'outOfRange');
    err('100000001', MONEY, 'outOfRange');
  });
});

describe('RATE mirrors Rate::parse', () => {
  it('refuses the same texts as the kernel, and zero', () => {
    for (const [t, e] of INVALID_FOR_ALL) err(t, RATE, e);
    err('0', RATE, 'sign');
    err('0.00000000', RATE, 'sign');
    tooMany('1.123456789', RATE, 8);
  });

  it('accepts up to 8 decimals and a zero ninth', () => {
    for (const t of ['32.1', '0.02152', '25.12345678', '1.123456780']) ok(t, RATE);
  });

  it('the cap is inclusive at 100000000', () => {
    ok('100000000', RATE);
    ok('100000000.00', RATE);
    err('100000000.01', RATE, 'outOfRange');
    err('100000001', RATE, 'outOfRange');
  });

  it('the cap is compared on the digits: one hundred-millionth above it is outOfRange', () => {
    // As a JS number, 100000000.00000001 rounds to 100000000 and would pass.
    err('100000000.00000001', RATE, 'outOfRange');
  });
});

describe('foreignAmount mirrors ForeignAmount::parse', () => {
  it('refuses the same texts as the kernel, and zero', () => {
    const b = foreignAmount(2);
    for (const [t, e] of INVALID_FOR_ALL) err(t, b, e);
    err('0', b, 'sign');
    tooMany('1.234', b, 2);
    tooMany('1.5', foreignAmount(0), 0);
  });

  it('accepts the value as typed', () => {
    for (const t of ['1.5', '1.50', '1.500']) ok(t, foreignAmount(2));
    ok('100', foreignAmount(0));
  });

  it('the cap is inclusive at 100000000', () => {
    const b = foreignAmount(2);
    ok('100000000', b);
    ok('100000000.00', b);
    err('100000000.01', b, 'outOfRange');
    err('100000001', b, 'outOfRange');
  });
});

describe('RATE_PERCENT mirrors RatePercent::parse', () => {
  it('accepts a leading minus, one leading plus and zero', () => {
    for (const t of ['-2.5', '+2.5', '0']) ok(t, RATE_PERCENT);
  });

  it('refuses a fifth non-zero decimal', () => {
    tooMany('1.23456', RATE_PERCENT, 4);
  });

  it('refuses text that is not a plain decimal', () => {
    for (const t of ['', '-', '+', '.5', '5.', '1e2', '--1']) err(t, RATE_PERCENT, 'syntax');
    // The kernel pins only "Invalid" for "+-1": a plus followed by a minus is never accepted.
    expect(checkDecimal('+-1', RATE_PERCENT)).toMatchObject({ ok: false });
  });

  it('is bounded by numeric(9,4) on both sides', () => {
    ok('99999.9999', RATE_PERCENT);
    ok('-99999.9999', RATE_PERCENT);
    err('100000', RATE_PERCENT, 'outOfRange');
    err('-100000', RATE_PERCENT, 'outOfRange');
    err('+100000', RATE_PERCENT, 'outOfRange');
    err('9'.repeat(30), RATE_PERCENT, 'outOfRange');
  });
});

// ================================================================ IntBound (bounded_test.rs, B-7)

describe('checkInteger mirrors IntBound', () => {
  it('is inclusive on both sides of a BETWEEN check', () => {
    expect(checkInteger('5', 5, 240)).toEqual({ ok: true, text: '5' });
    expect(checkInteger('240', 5, 240)).toEqual({ ok: true, text: '240' });
    expect(checkInteger('4', 5, 240)).toMatchObject({ ok: false, error: 'outOfRange' });
    expect(checkInteger('241', 5, 240)).toMatchObject({ ok: false, error: 'outOfRange' });
  });

  it('mirrors > 0 and >= 0', () => {
    expect(checkInteger('1', 1, Number.MAX_SAFE_INTEGER)).toEqual({ ok: true, text: '1' });
    expect(checkInteger('0', 1, Number.MAX_SAFE_INTEGER)).toMatchObject({ ok: false, error: 'outOfRange' });
    expect(checkInteger('0', 0, Number.MAX_SAFE_INTEGER)).toEqual({ ok: true, text: '0' });
  });

  it('takes digits only: no blank, space, hex, exponent or decimal point', () => {
    for (const t of ['', ' 5', '5 ', '0x10', '1.5', '1e2', 'abc']) {
      expect(checkInteger(t, 3, 120), JSON.stringify(t)).toMatchObject({ ok: false, error: 'syntax' });
    }
  });

  it('a 20-digit integer is outOfRange, not rounded into range', () => {
    expect(checkInteger('99999999999999999999', 3, 120)).toMatchObject({ ok: false, error: 'outOfRange' });
  });
});
