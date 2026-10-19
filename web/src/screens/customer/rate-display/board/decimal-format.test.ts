// RB-3 up-front tests (feat/fe-rateboard skeleton; rate-display.md §9.4, B7): formatBandRate and compareDecimal work
// on the route's decimal strings, never through a float. The implementer makes these pass and must not edit them.
import { describe, expect, it } from 'vitest';
import { compareDecimal, formatBandRate } from './decimal-format';

describe('formatBandRate (RB-3)', () => {
  it.each([
    ['35.42000000', '35.42'],
    ['21.34500000', '21.35'],
    ['5.67890000', '5.679'],
    ['1.00000000', '1.000'],
    ['0.12345000', '0.1235'],
    ['0.00456700', '0.0046'],
    ['1000', '1,000.00'],
    ['36500.5', '36,500.50'],
  ])('%s → %s (2 decimals from 10, 3 from 1, else 4; half up; thousands separators)', (input, out) => {
    expect(formatBandRate(input)).toBe(out);
  });

  it('keeps a value a float cannot hold: 1234567890.12345678 → 1,234,567,890.12', () => {
    expect(formatBandRate('1234567890.12345678')).toBe('1,234,567,890.12');
  });

  it('rounds half up on the string, carrying into the integer part and the separators', () => {
    expect(formatBandRate('99.995')).toBe('100.00');
    expect(formatBandRate('999.995')).toBe('1,000.00');
    expect(formatBandRate('35.424999')).toBe('35.42');
  });

  it.each([[null], [undefined], [''], ['0'], ['0.00000000'], ['-1.5']])('%s → — (missing, zero or negative)', (input) => {
    expect(formatBandRate(input as string | null | undefined)).toBe('—');
  });
});

describe('compareDecimal (RB-3)', () => {
  it.each<[string, string, -1 | 0 | 1]>([
    ['35.42', '35.45', -1],
    ['35.45', '35.42', 1],
    ['35.450', '35.45', 0],
    ['100', '99.999', 1],
    ['0.5', '0.49', 1],
    ['9', '10', -1],
    ['1234567890.12345678', '1234567890.12345677', 1],
  ])('compareDecimal(%s, %s) = %i', (a, b, out) => {
    expect(compareDecimal(a, b)).toBe(out);
  });
});

describe('no float on a rate (B7)', () => {
  it('decimal-format.ts never parses a rate into a number', async () => {
    const src = (await import('./decimal-format.ts?raw')).default as string;
    const code = src.replace(/\/\/.*$/gm, '').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(code).not.toMatch(/parseFloat|Number\(|Math\.|toFixed|toLocaleString/);
  });
});
