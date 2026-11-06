// Up-front tests for ticket CS-14a (design configurable-shifts.md §2.9, T1, T5). Written by the ticket author
// claude/35271802 — do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';
import { coverageGaps, cutoverOf, defaultEnd } from './shift-coverage';

const r = (startsAt: string, endsAt: string) => ({ startsAt, endsAt });

describe('cutoverOf', () => {
  it('is the earliest start', () => {
    expect(cutoverOf([r('14:00', '22:00'), r('06:00', '14:00'), r('22:00', '06:00')])).toBe('06:00');
  });
  it('is null without rows', () => {
    expect(cutoverOf([])).toBeNull();
  });
});

describe('coverageGaps', () => {
  it('the defaults 06/14/22 cover the day', () => {
    expect(coverageGaps([r('06:00', '14:00'), r('14:00', '22:00'), r('22:00', '06:00')])).toEqual([]);
  });
  it('an overlapping 24 h plan covers the day', () => {
    expect(coverageGaps([r('06:00', '15:00'), r('14:00', '23:00'), r('22:00', '07:00')])).toEqual([]);
  });
  it('names the night gap of A 06–14 + B 14–22', () => {
    expect(coverageGaps([r('06:00', '14:00'), r('14:00', '22:00')])).toEqual([{ from: '22:00', to: '06:00' }]);
  });
  it("names the owner's first table's gap 04:00–10:00", () => {
    expect(coverageGaps([r('10:00', '19:00'), r('15:00', '00:00'), r('19:00', '04:00')])).toEqual([
      { from: '04:00', to: '10:00' },
    ]);
  });
  it('lists every gap in order from the cutover', () => {
    expect(coverageGaps([r('06:00', '10:00'), r('12:00', '20:00'), r('22:00', '04:00')])).toEqual([
      { from: '10:00', to: '12:00' },
      { from: '20:00', to: '22:00' },
      { from: '04:00', to: '06:00' },
    ]);
  });
  it('no rows: no gaps reported (the server refuses an empty list itself)', () => {
    expect(coverageGaps([])).toEqual([]);
  });
});

describe('defaultEnd', () => {
  it('adds the default length', () => {
    expect(defaultEnd('06:00', 8)).toBe('14:00');
  });
  it('wraps past midnight', () => {
    expect(defaultEnd('22:00', 8)).toBe('06:00');
    expect(defaultEnd('19:30', 8)).toBe('03:30');
  });
});
