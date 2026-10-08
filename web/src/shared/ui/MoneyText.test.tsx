// V0-A: MoneyText (docs/design/xchange-v3-web.md W7, §5.2) — formats a decimal string, no arithmetic.
// The grouping and the place of the currency are not fixed here: the first screen that shows money
// pins v2's format. What is fixed: every digit, the sign and the decimals arrive as sent.
// Written before the implementation. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { formatMoney, MoneyText } from './MoneyText';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

function shown(value: string, currency: string): string {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose?.();
  dispose = render(() => <MoneyText value={value} currency={currency} />, host);
  return host.textContent ?? '';
}

/** The text with grouping and spacing taken out, the currency left in. */
function digits(text: string): string {
  return text.replace(/[\s,  ]/g, '');
}

describe('MoneyText', () => {
  it('keeps every digit of an amount larger than a JavaScript number holds', () => {
    const text = shown('90071992547409931.05', 'THB');
    expect(digits(text)).toContain('90071992547409931.05');
    expect(text).toContain('THB');
  });

  it('keeps the trailing zeros and the sign', () => {
    expect(digits(shown('-1234.50', 'USD'))).toContain('-1234.50');
    expect(digits(shown('0.10', 'JPY'))).toContain('0.10');
    expect(digits(shown('35.1000', 'USD'))).toContain('35.1000');
  });

  it('shows a value that is not a decimal string as it came, never NaN', () => {
    for (const bad of ['1e3', 'abc', '']) {
      const text = shown(bad, 'USD');
      expect(text).not.toContain('NaN');
      expect(text).not.toContain('1,000');
      expect(text).toContain(bad);
    }
  });
});

describe('formatMoney', () => {
  it('formats decimal strings with comma grouping and preserves decimals and sign', () => {
    expect(formatMoney('1234567.89')).toBe('1,234,567.89');
    expect(formatMoney('-1234567.89')).toBe('-1,234,567.89');
    expect(formatMoney('1000')).toBe('1,000');
    expect(formatMoney('-1000')).toBe('-1,000');
    expect(formatMoney('0')).toBe('0');
    expect(formatMoney('0.10')).toBe('0.10');
    expect(formatMoney('90071992547409931.05')).toBe('90,071,992,547,409,931.05');
  });

  it('returns non-decimal strings untouched', () => {
    expect(formatMoney('abc')).toBe('abc');
    expect(formatMoney('1e3')).toBe('1e3');
    expect(formatMoney('')).toBe('');
  });
});

