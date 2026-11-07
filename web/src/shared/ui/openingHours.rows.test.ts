// OL-6 wave B (cust-opening-hours, audit #564 issuecomment-6702970101): openingHoursRows, and formatOpeningHours built from it with its output unchanged.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { describe, expect, it } from 'vitest';
import { formatOpeningHours, openingHoursRows } from './openingHours';

describe('openingHoursRows(hours, lang)', () => {
  it('OH-1 null and undefined give no rows', () => {
    expect(openingHoursRows(null)).toEqual([]);
    expect(openingHoursRows(undefined)).toEqual([]);
  });

  it('OH-2 an empty object is seven closed rows, Monday first', () => {
    expect(openingHoursRows({}, 'en')).toEqual([
      { day: 'Mon', text: 'Closed' },
      { day: 'Tue', text: 'Closed' },
      { day: 'Wed', text: 'Closed' },
      { day: 'Thu', text: 'Closed' },
      { day: 'Fri', text: 'Closed' },
      { day: 'Sat', text: 'Closed' },
      { day: 'Sun', text: 'Closed' },
    ]);
  });

  it('OH-3 two intervals stay in one row, whatever order the object lists its days', () => {
    const rows = openingHoursRows(
      { tue: [{ open: '10:00', close: '20:00' }], mon: [{ open: '09:00', close: '12:00' }, { open: '13:00', close: '18:00' }] },
      'th',
    );
    expect(rows.slice(0, 3)).toEqual([
      { day: 'จันทร์', text: '09:00 - 12:00, 13:00 - 18:00' },
      { day: 'อังคาร', text: '10:00 - 20:00' },
      { day: 'พุธ', text: 'ปิดทำการ' },
    ]);
  });

  it('OH-4 an interval missing a time is dropped; a day left with none is closed', () => {
    const rows = openingHoursRows({ mon: [{ open: '', close: '18:00' }], tue: [] }, 'zh');
    expect(rows[0]).toEqual({ day: '周一', text: '休息' });
    expect(rows[1]).toEqual({ day: '周二', text: '休息' });
  });
});

describe('formatOpeningHours is built from the rows, its output unchanged', () => {
  const hours = { mon: [{ open: '09:00', close: '12:00' }, { open: '13:00', close: '18:00' }], wed: [{ open: '10:00', close: '20:00' }] };

  it.each(['th', 'en', 'zh'])('OH-5 %s: "<day>: <text>" joined by ", "', (lang) => {
    const joined = openingHoursRows(hours, lang).map((r) => `${r.day}: ${r.text}`).join(', ');
    expect(formatOpeningHours(hours, lang)).toBe(joined);
  });

  it('OH-6 the exact string, as before', () => {
    expect(formatOpeningHours(hours, 'en')).toBe(
      'Mon: 09:00 - 12:00, 13:00 - 18:00, Tue: Closed, Wed: 10:00 - 20:00, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );
    expect(formatOpeningHours(null)).toBeNull();
  });
});
