// F55 (docs/design/backdated-keying.md §16.7), owner ruling 2026-11-07: customer booking times use the branch's own
// time zone, in the same dd/mm/yyyy HH:mm shape with a CE year. `formatDateTimeInZone` and `formatDateInZone` are the
// zone-aware twins of `formatBangkokDateTime` and `formatBangkokDate`.
// Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own.
import { describe, expect, it } from 'vitest';
import { setLang } from '../i18n';

type Mod = {
  formatDateTimeInZone?: (iso: string, timeZone: string | null | undefined) => string;
  formatDateInZone?: (iso: string, timeZone: string | null | undefined) => string;
};
const mods = import.meta.glob<Mod>('./bangkok-time.ts', { eager: true });

function dt(): NonNullable<Mod['formatDateTimeInZone']> {
  const f = mods['./bangkok-time.ts']!.formatDateTimeInZone;
  expect(typeof f, 'exports formatDateTimeInZone').toBe('function');
  return f!;
}
function d(): NonNullable<Mod['formatDateInZone']> {
  const f = mods['./bangkok-time.ts']!.formatDateInZone;
  expect(typeof f, 'exports formatDateInZone').toBe('function');
  return f!;
}

describe('F55 zone-aware formatters', () => {
  it('Z1: the same instant reads in the given zone', () => {
    expect(dt()('2026-11-04T06:23:00Z', 'Asia/Bangkok')).toBe('04/11/2026 13:23');
    expect(dt()('2026-11-04T06:23:00Z', 'Asia/Tokyo')).toBe('04/11/2026 15:23');
  });

  it("Z2: the day rolls over at the zone's midnight", () => {
    expect(dt()('2026-11-04T15:30:00Z', 'Asia/Tokyo')).toBe('05/11/2026 00:30');
    expect(d()('2026-11-04T15:30:00Z', 'Asia/Tokyo')).toBe('05/11/2026');
    expect(d()('2026-11-04T15:30:00Z', 'Asia/Bangkok')).toBe('04/11/2026');
  });

  it('Z3: no zone, or an unknown one, falls back to Asia/Bangkok', () => {
    expect(dt()('2026-11-04T06:23:00Z', null)).toBe('04/11/2026 13:23');
    expect(dt()('2026-11-04T06:23:00Z', undefined)).toBe('04/11/2026 13:23');
    expect(dt()('2026-11-04T06:23:00Z', 'Not/AZone')).toBe('04/11/2026 13:23');
  });

  it('Z4: CE year and Latin digits in Thai too; unparseable input comes back unchanged', () => {
    setLang('th');
    expect(dt()('2026-11-04T06:23:00Z', 'Asia/Tokyo')).toBe('04/11/2026 15:23');
    expect(dt()('garbage', 'Asia/Tokyo')).toBe('garbage');
    expect(d()('', 'Asia/Tokyo')).toBe('');
  });
});
