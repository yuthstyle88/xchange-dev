// P01 v2 decimal formatting (rate-display.md §9.4; B7: decimal strings, never a JS float as authority).
// SKELETON STUB (feat/fe-rateboard): RB-3 replaces both bodies; the signatures stay.

/**
 * A rate string as the board shows it: `—` for null, empty or not > 0; otherwise 2 decimals when the integer part
 * is ≥ 10, 3 when ≥ 1, else 4, rounded half up on the string, with thousands separators. Never `parseFloat`,
 * `Number(` or `Math.` on the value.
 */
export function formatBandRate(value: string | null | undefined): string {
  return value ?? '—';
}

/** Exact comparison of two decimal strings: -1, 0 or 1. Never through a float. */
export function compareDecimal(_a: string, _b: string): -1 | 0 | 1 {
  return 0;
}
