// RE-5 (#602): the one rate formatter, `formatRate` in `web/src/shared/ui/rate.ts`.
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md` §3.3 (precision and display: a rate on the wire is the
// 8-place string the database holds; one web formatter shows at least 2 and at most 8 places with
// trailing zeros trimmed, followed by `/ <unit_size> <code>` when `unit_size > 1`; no screen formats
// a rate itself), §9 RE-5 (`formatRate("23.50000000", 100, "JPY")` gives `23.50 / 100 JPY`), §11
// (no JS float is authoritative; the screens only show strings), `xchange-v3-web.md` W7.
//
// Signatures this test assumes:
// - `formatRate(rate: string, unitSize: number | string, code: string): string`, a named export of
//   `web/src/shared/ui/rate.ts`. `unitSize` comes as a number from `rates_reference` (`unit: 100`)
//   and as a decimal string from `published_rates` (`unitSize: "100"`, `published_rates.rs`
//   `unit_size_dec.normalize().to_string()`), so both forms are accepted.
// - Grouping of the integer part is not fixed here (no rate below 1000 has any); the digits are.
//
// The module is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import (and `tsc` stays green before it lands).
import { describe, expect, it } from 'vitest';

interface RateModule {
  formatRate: (rate: string, unitSize: number | string, code: string) => string;
}

const modules = import.meta.glob<RateModule>('./rate.ts', { eager: true });
const sources = import.meta.glob<string>('./rate.ts', { query: '?raw', import: 'default', eager: true });

function formatRate(rate: string, unitSize: number | string, code: string): string {
  const m = modules['./rate.ts'];
  expect(m, 'web/src/shared/ui/rate.ts exists').toBeDefined();
  expect(typeof m!.formatRate, 'rate.ts exports formatRate').toBe('function');
  return m!.formatRate(rate, unitSize, code);
}

describe('RE-5 formatRate: per unit_size, 2 to 8 places (#602; design §3.3, §9)', () => {
  it('the design\'s example: formatRate("23.50000000", 100, "JPY") is "23.50 / 100 JPY"', () => {
    expect(formatRate('23.50000000', 100, 'JPY')).toBe('23.50 / 100 JPY');
  });

  it('takes the unit as published_rates sends it, a decimal string', () => {
    expect(formatRate('23.50000000', '100', 'JPY')).toBe('23.50 / 100 JPY');
    expect(formatRate('25.12345678', '1000', 'KRW')).toBe('25.12345678 / 1000 KRW');
  });

  it('a unit of 1 has no suffix, whether 1 or "1"', () => {
    expect(formatRate('32.10000000', 1, 'USD')).toBe('32.10');
    expect(formatRate('32.10000000', '1', 'USD')).toBe('32.10');
  });

  it('trims trailing zeros down to 2 places and keeps every significant place up to 8', () => {
    expect(formatRate('0.02152000', 1, 'IDR')).toBe('0.02152');
    expect(formatRate('33.5008', 1, 'USD')).toBe('33.5008');
    expect(formatRate('0.10000000', 1, 'EUR')).toBe('0.10');
    expect(formatRate('1.23400000', 1, 'GBP')).toBe('1.234');
    expect(formatRate('0.12345678', 1, 'CNY')).toBe('0.12345678');
  });

  it('pads a rate with fewer than 2 places to 2 (the 4-place and bare strings of older readers)', () => {
    expect(formatRate('32.1', 1, 'USD')).toBe('32.10');
    expect(formatRate('23', 100, 'JPY')).toBe('23.00 / 100 JPY');
    expect(formatRate('21.0100', 100, 'JPY')).toBe('21.01 / 100 JPY');
  });

  it('never rounds: a rate is shown as the server sent it, digit for digit', () => {
    // 1.005 is the classic float trap: Number("1.005").toFixed(2) is "1.00"
    expect(formatRate('1.00500000', 1, 'USD')).toBe('1.005');
    expect(formatRate('0.99999999', 1, 'USD')).toBe('0.99999999');
  });

  it('keeps every digit of a rate larger than a JavaScript number holds', () => {
    const shown = formatRate('99999999.99999999', 1, 'USD').replace(/[\s,  ]/g, '');
    expect(shown).toBe('99999999.99999999');
  });
});

describe('RE-5 formatRate: no number arithmetic in its source (W7; design §11)', () => {
  it('parses no figure into a JS number and rounds nothing', () => {
    const text = sources['./rate.ts'];
    expect(text, 'web/src/shared/ui/rate.ts exists').toBeDefined();
    const forbidden = /Number\(|parseFloat|parseInt|Math\.|toFixed|toPrecision|toLocaleString|Intl\.NumberFormat/;
    text!.split('\n').forEach((line, i) => {
      expect(forbidden.test(line), `rate.ts:${i + 1} ${line.trim()}`).toBe(false);
    });
  });
});
