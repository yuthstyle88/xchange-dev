// FE-T6 #5854: `formatBangkokDateTime` shows a server time in ICT as `dd/MM/yyyy HH:mm`.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/fe-triage-1101.md` §2 (BangkokTime) and W1 (PR #5848): `dd/MM/yyyy HH:mm` in
// Asia/Bangkok; the input comes back unchanged when it doesn't parse; no `Number(`, `Math.`, `toFixed`
// or `parseFloat` in the module (the rule of `desk-transaction.source.test.ts:53`, which scans only its
// own folder, so this file checks the new module itself).
// The module is read through globs, so on a branch without it each test fails on an assertion.
import { describe, expect, it } from 'vitest';

type Mod = { formatBangkokDateTime: (iso: string) => string };
const mods = import.meta.glob<Mod>('./bangkok-time.ts', { eager: true });
const raws = import.meta.glob<string>('./bangkok-time.ts', { query: '?raw', import: 'default', eager: true });

function fmt(): (iso: string) => string {
  const m = mods['./bangkok-time.ts'];
  expect(m, 'web/src/shared/format/bangkok-time.ts exists').toBeDefined();
  expect(typeof m!.formatBangkokDateTime, 'exports formatBangkokDateTime').toBe('function');
  return m!.formatBangkokDateTime;
}

describe('FE-T6 #5854: formatBangkokDateTime (design W1)', () => {
  it('B1: an RFC 3339 time with nanoseconds and +00:00 shows in ICT', () => {
    expect(fmt()('2026-11-01T05:44:25.455952232+00:00')).toBe('01/11/2026 12:44');
  });

  it('B2: a Z time after 17:00 UTC rolls to the next Bangkok day', () => {
    expect(fmt()('2026-10-31T17:30:00Z')).toBe('01/11/2026 00:30');
  });

  it('B3: Bangkok midnight is 00:00, never 24:00', () => {
    expect(fmt()('2026-11-01T17:00:00Z')).toBe('02/11/2026 00:00');
  });

  it('B4: a time already in +07:00 keeps its wall clock', () => {
    expect(fmt()('2026-11-01T09:05:00+07:00')).toBe('01/11/2026 09:05');
  });

  it('B5: input that does not parse comes back unchanged', () => {
    const f = fmt();
    expect(f('garbage')).toBe('garbage');
    expect(f('')).toBe('');
    expect(f('2026-13-45T99:99:99Z')).toBe('2026-13-45T99:99:99Z');
  });

  it('B6: the digits are Latin whatever the page language', () => {
    expect(fmt()('2026-01-02T03:04:00Z')).toMatch(/^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2}$/);
    expect(fmt()('2026-01-02T03:04:00Z')).toBe('02/01/2026 10:04');
  });

  it('B7: the module has no Number(, Math., toFixed or parseFloat', () => {
    const src = raws['./bangkok-time.ts'];
    expect(src, 'web/src/shared/format/bangkok-time.ts exists').toBeDefined();
    const hits = [/\bNumber\(/, /\bparseFloat\b/, /\.toFixed\(/, /\bMath\./].filter((re) => re.test(src!)).map(String);
    expect(hits).toEqual([]);
  });
});
