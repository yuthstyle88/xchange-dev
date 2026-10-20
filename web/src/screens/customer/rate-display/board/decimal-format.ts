// P01 v2 decimal formatting (rate-display.md §9.4; B7: decimal strings, never a JS float as authority).

function splitNormalized(raw: string): { neg: boolean; int: string; frac: string } | null {
  const s = raw.trim();
  if (!/^[+-]?\d+(?:\.\d+)?$/.test(s)) return null;
  const neg = s.startsWith('-');
  const unsigned = s.startsWith('-') || s.startsWith('+') ? s.slice(1) : s;
  const [rawInt, rawFrac = ''] = unsigned.split('.');
  const int = rawInt.replace(/^0+/, '') || '0';
  const frac = rawFrac.replace(/0+$/, '');
  const isZero = int === '0' && frac === '';
  return { neg: isZero ? false : neg, int, frac };
}

/**
 * A rate string as the board shows it: `—` for null, empty or not > 0; otherwise 2 decimals when the integer part
 * is ≥ 10, 3 when ≥ 1, else 4, rounded half up on the string, with thousands separators.
 */
export function formatBandRate(value: string | null | undefined): string {
  if (value == null) return '—';
  const s = value.trim();
  if (!/^\+?\d+(?:\.\d+)?$/.test(s)) return '—';
  const unsigned = s.startsWith('+') ? s.slice(1) : s;
  const [rawInt, rawFrac = ''] = unsigned.split('.');
  const intPart = rawInt.replace(/^0+/, '') || '0';
  if (intPart === '0' && /^0*$/.test(rawFrac)) return '—';

  const decimals = intPart.length >= 2 ? 2 : intPart !== '0' ? 3 : 4;
  const paddedFrac = rawFrac.padEnd(decimals + 1, '0');
  const digits = (intPart + paddedFrac.slice(0, decimals)).split('');

  if (paddedFrac[decimals] >= '5') {
    let carry = true;
    for (let i = digits.length - 1; i >= 0; i--) {
      if (digits[i] === '9') {
        digits[i] = '0';
      } else {
        digits[i] = String.fromCharCode(digits[i].charCodeAt(0) + 1);
        carry = false;
        break;
      }
    }
    if (carry) digits.unshift('1');
  }

  const roundedInt = digits.slice(0, digits.length - decimals).join('');
  const roundedFrac = digits.slice(digits.length - decimals).join('');
  const withCommas = roundedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${withCommas}.${roundedFrac}`;
}

/** Exact comparison of two decimal strings: -1, 0 or 1. Never through a float. */
export function compareDecimal(a: string, b: string): -1 | 0 | 1 {
  const pa = splitNormalized(a);
  const pb = splitNormalized(b);
  if (!pa || !pb) return 0;
  if (pa.neg !== pb.neg) return pa.neg ? -1 : 1;

  let mag: -1 | 0 | 1 = 0;
  if (pa.int.length !== pb.int.length) {
    mag = pa.int.length < pb.int.length ? -1 : 1;
  } else if (pa.int !== pb.int) {
    mag = pa.int < pb.int ? -1 : 1;
  } else if (pa.frac !== pb.frac) {
    mag = pa.frac < pb.frac ? -1 : 1;
  }

  if (mag === 0) return 0;
  return (pa.neg ? -mag : mag) as -1 | 1;
}
