// Repairs the `pattern` attributes of the kept pages (row FV, docs/design/xchange-v3.md §5.3).
//
// Current browsers compile `pattern` with the `v` flag, where a `-` that ends a character class
// is a syntax error, and a pattern that does not compile is ignored: the form submits unchecked.

const LOGIN = '[a-zA-Z0-9._-]{3,30}';
const LOGIN_FIXED = '[a-zA-Z0-9._\\-]{3,40}';

/** Patterns replaced outright: the sign-in door accepts 3-40, not v2's 3-30. */
export const PATTERN_OVERRIDES: Readonly<Record<string, string>> = Object.freeze({
  [LOGIN]: LOGIN_FIXED,
});

/** Number of backslashes directly before index `i` of `s`. */
function backslashesBefore(s: string, i: number): number {
  let count = 0;
  for (let j = i - 1; j >= 0 && s[j] === '\\'; j--) count++;
  return count;
}

/** Escapes a `-` that ends a character class, the one construct `v` refuses and `u` allowed. */
export function repairDashes(p: string): string {
  let result = '';
  let inClass = false;
  for (let i = 0; i < p.length; i++) {
    const ch = p[i];
    const escaped = backslashesBefore(p, i) % 2 === 1;
    if (!inClass) {
      if (ch === '[' && !escaped) inClass = true;
    } else if (ch === ']' && !escaped) {
      if (result.endsWith('-') && backslashesBefore(result, result.length - 1) % 2 === 0) {
        result = result.slice(0, -1) + '\\-';
      }
      inClass = false;
    }
    result += ch;
  }
  return result;
}

function compiles(p: string): boolean {
  try {
    new RegExp(`^(?:${p})$`, 'v');
    return true;
  } catch {
    return false;
  }
}

/** The pattern to use instead of `p`, or null when `p` should stay as it is. */
export function fixPattern(p: unknown): string | null {
  if (typeof p !== 'string') return null;
  if (Object.prototype.hasOwnProperty.call(PATTERN_OVERRIDES, p)) return PATTERN_OVERRIDES[p];
  if (compiles(p)) return null;
  const repaired = repairDashes(p);
  return repaired !== p && compiles(repaired) ? repaired : null;
}
