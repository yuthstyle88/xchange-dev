// F55 (docs/design/backdated-keying.md §16.7): one date format everywhere: `dd/mm/yyyy HH:mm` in Asia/Bangkok with a
// Western (CE) year. `formatBangkokDateTime` already gives the date and time; F55-0 adds its date-only twin
// `formatBangkokDate`, for places that show a day (a business date, a period, an expiry). Every screen then uses these
// two instead of `toLocale*String`/`th-TH` (Buddhist year) or a raw `2026-11-04`.
// Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own.
import { describe, expect, it } from 'vitest';
import { setLang } from '../i18n';

type Mod = { formatBangkokDate?: (value: string) => string; formatBangkokDateTime: (iso: string) => string };
const mods = import.meta.glob<Mod>('./bangkok-time.ts', { eager: true });

function dateFmt(): (value: string) => string {
  const m = mods['./bangkok-time.ts']!;
  expect(typeof m.formatBangkokDate, 'exports formatBangkokDate').toBe('function');
  return m.formatBangkokDate!;
}

describe('F55-0: formatBangkokDate, the date-only twin', () => {
  it('D1: a calendar day YYYY-MM-DD is shown as dd/mm/yyyy, never shifted by a time zone', () => {
    expect(dateFmt()('2026-11-04')).toBe('04/11/2026');
    expect(dateFmt()('2026-01-31')).toBe('31/01/2026');
  });

  it('D2: an RFC 3339 instant shows its Bangkok calendar day', () => {
    expect(dateFmt()('2026-11-04T16:59:59Z')).toBe('04/11/2026');
    expect(dateFmt()('2026-11-04T17:00:00Z')).toBe('05/11/2026');
    expect(dateFmt()('2026-11-05T02:59:00+07:00')).toBe('05/11/2026');
  });

  it('D3: the year is CE in Thai too (never 2569)', () => {
    setLang('th');
    expect(dateFmt()('2026-11-04')).toBe('04/11/2026');
    expect(mods['./bangkok-time.ts']!.formatBangkokDateTime('2026-11-04T06:23:00Z')).toBe('04/11/2026 13:23');
  });

  it('D4: input that does not parse comes back unchanged', () => {
    const f = dateFmt();
    expect(f('')).toBe('');
    expect(f('garbage')).toBe('garbage');
    expect(f('2026-13-45')).toBe('2026-13-45');
  });
});
