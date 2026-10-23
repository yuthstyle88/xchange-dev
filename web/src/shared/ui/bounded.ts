export type Sign = 'positive' | 'nonNegative' | 'signed';
export type BoundError = 'syntax' | 'tooManyDecimals' | 'outOfRange' | 'sign';
export interface Bound {
  intDigits: number;
  scale: number;
  sign: Sign;
  maxInclusive?: string;
  plusAllowed?: boolean;
}
export type Checked = { ok: true; text: string } | { ok: false; error: BoundError; scale: number };

export function numeric(precision: number, scale: number, sign: Sign): Bound {
  return { intDigits: Math.max(0, precision - scale), scale, sign };
}

export function atMost(b: Bound, max: string): Bound {
  return { ...b, maxInclusive: max };
}

function fail(error: BoundError, scale: number): Checked {
  return { ok: false, error, scale };
}

export function checkDecimal(text: string, b: Bound): Checked {
  if (!text) return fail('syntax', b.scale);

  let negative = false;
  let unsigned = text;
  if (text.startsWith('-')) {
    negative = true;
    unsigned = text.slice(1);
  } else if (b.plusAllowed && text.startsWith('+')) {
    unsigned = text.slice(1);
  }

  if (!/^\d+(?:\.\d+)?$/.test(unsigned)) return fail('syntax', b.scale);

  const dotIdx = unsigned.indexOf('.');
  const intPart = dotIdx === -1 ? unsigned : unsigned.slice(0, dotIdx);
  const fracPart = dotIdx === -1 ? '' : unsigned.slice(dotIdx + 1);

  if (fracPart.length > b.scale && /[^0]/.test(fracPart.slice(b.scale))) {
    return fail('tooManyDecimals', b.scale);
  }

  const sigInt = intPart.replace(/^0+/, '');
  const isZero = sigInt.length === 0 && /^0*$/.test(fracPart);

  if (b.sign === 'positive' && (negative || isZero)) return fail('sign', b.scale);
  if (b.sign === 'nonNegative' && negative) return fail('sign', b.scale);
  if (sigInt.length > b.intDigits) return fail('outOfRange', b.scale);

  if (b.maxInclusive !== undefined) {
    const cap = BigInt(b.maxInclusive);
    const intVal = BigInt(sigInt || '0');
    const fracNonZero = /[^0]/.test(fracPart.slice(0, b.scale));
    if (intVal > cap || (intVal === cap && fracNonZero)) {
      return fail('outOfRange', b.scale);
    }
  }

  return { ok: true, text };
}

export function checkInteger(text: string, min: number, max: number): Checked {
  if (!/^\d+$/.test(text)) return fail('syntax', 0);
  const val = BigInt(text);
  if (val < BigInt(min) || val > BigInt(max)) return fail('outOfRange', 0);
  return { ok: true, text };
}

export const MONEY: Bound = atMost(numeric(20, 2, 'nonNegative'), '100000000');
export const RATE: Bound = atMost(numeric(20, 8, 'positive'), '100000000');
export const RATE_PERCENT: Bound = { ...numeric(9, 4, 'signed'), plusAllowed: true };

export function foreignAmount(minorUnit: number): Bound {
  return atMost(numeric(20, minorUnit, 'positive'), '100000000');
}
