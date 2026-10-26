// #5127 row 1: a denomination's face value is shown as money — thousands grouped, trailing zeros of the
// fraction dropped, and a fraction that remains keeps at least 2 places. The wire string stays on the row as
// `data-face-value`; this helper only words it. `formatAmount` words an amount (the close count's expected):
// always at least 2 places, trailing zeros beyond 2 dropped, nothing rounded away.
import { describe, expect, it } from 'vitest';
import { formatAmount, formatFaceValue } from './face-value';

describe('#5127 formatFaceValue', () => {
  it.each([
    ['1000.0000', '1,000'],
    ['0.2500', '0.25'],
    ['0.5000', '0.50'],
    ['0.0100', '0.01'],
    ['50.0000', '50'],
    ['100000.0000', '100,000'],
    ['2.5000', '2.50'],
    ['1000', '1,000'],
    ['0.1250', '0.125'],
  ])('%s → %s', (wire, shown) => {
    expect(formatFaceValue(wire)).toBe(shown);
  });

  it('returns a string that is not a number unchanged', () => {
    expect(formatFaceValue('abc')).toBe('abc');
    expect(formatFaceValue('')).toBe('');
  });
});

describe('#5127 formatAmount', () => {
  it.each([
    ['9839.5000', '9,839.50'],
    ['9839.50', '9,839.50'],
    ['5.0000', '5.00'],
    ['0', '0.00'],
    ['1234567.8900', '1,234,567.89'],
    ['9839.5050', '9,839.505'],
    ['-160.5000', '-160.50'],
  ])('%s → %s', (wire, shown) => {
    expect(formatAmount(wire)).toBe(shown);
  });

  it('returns a string that is not a number unchanged', () => {
    expect(formatAmount('n/a')).toBe('n/a');
  });
});
