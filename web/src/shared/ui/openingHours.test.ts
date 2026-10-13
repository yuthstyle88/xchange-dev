// APP-12 / APP-8 formatOpeningHours unit tests.
import { describe, expect, it } from 'vitest';
import { formatOpeningHours } from './openingHours';

describe('formatOpeningHours(hours, lang)', () => {
  it('returns null for null or undefined', () => {
    expect(formatOpeningHours(null)).toBeNull();
    expect(formatOpeningHours(undefined)).toBeNull();
  });

  it('an empty opening-hours object {} reads as closed every day', () => {
    expect(formatOpeningHours({}, 'en')).toBe(
      'Mon: Closed, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );
    expect(formatOpeningHours({}, 'th')).toBe(
      'จันทร์: ปิดทำการ, อังคาร: ปิดทำการ, พุธ: ปิดทำการ, พฤหัสบดี: ปิดทำการ, ศุกร์: ปิดทำการ, เสาร์: ปิดทำการ, อาทิตย์: ปิดทำการ',
    );
    expect(formatOpeningHours({}, 'zh')).toBe(
      '周一: 休息, 周二: 休息, 周三: 休息, 周四: 休息, 周五: 休息, 周六: 休息, 周日: 休息',
    );
  });

  it('Mon-Tue only branch renders days mon..sun with absent days closed and never reads as open daily', () => {
    const hours = {
      mon: [{ open: '09:00', close: '18:00' }],
      tue: [{ open: '09:00', close: '18:00' }],
    };

    expect(formatOpeningHours(hours, 'en')).toBe(
      'Mon: 09:00 - 18:00, Tue: 09:00 - 18:00, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );

    expect(formatOpeningHours(hours, 'th')).toBe(
      'จันทร์: 09:00 - 18:00, อังคาร: 09:00 - 18:00, พุธ: ปิดทำการ, พฤหัสบดี: ปิดทำการ, ศุกร์: ปิดทำการ, เสาร์: ปิดทำการ, อาทิตย์: ปิดทำการ',
    );

    expect(formatOpeningHours(hours, 'zh')).toBe(
      '周一: 09:00 - 18:00, 周二: 09:00 - 18:00, 周三: 休息, 周四: 休息, 周五: 休息, 周六: 休息, 周日: 休息',
    );
  });

  it('formats split day with multiple intervals', () => {
    const hours = {
      mon: [
        { open: '09:00', close: '12:00' },
        { open: '13:00', close: '18:00' },
      ],
    };

    expect(formatOpeningHours(hours, 'en')).toBe(
      'Mon: 09:00 - 12:00, 13:00 - 18:00, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );
  });

  it('formats explicit empty interval array as closed', () => {
    const hours = {
      mon: [{ open: '10:00', close: '19:00' }],
      tue: [],
    };

    expect(formatOpeningHours(hours, 'en')).toBe(
      'Mon: 10:00 - 19:00, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );
  });

  it('normalizes locale strings like zh-CN or en-US', () => {
    const hours = {
      mon: [{ open: '08:30', close: '16:30' }],
    };

    expect(formatOpeningHours(hours, 'zh-CN')).toBe(
      '周一: 08:30 - 16:30, 周二: 休息, 周三: 休息, 周四: 休息, 周五: 休息, 周六: 休息, 周日: 休息',
    );

    expect(formatOpeningHours(hours, 'en-US')).toBe(
      'Mon: 08:30 - 16:30, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );
  });

  it('defaults to Thai when lang is omitted or unrecognized', () => {
    const hours = {
      mon: [{ open: '10:00', close: '20:00' }],
    };

    expect(formatOpeningHours(hours)).toBe(
      'จันทร์: 10:00 - 20:00, อังคาร: ปิดทำการ, พุธ: ปิดทำการ, พฤหัสบดี: ปิดทำการ, ศุกร์: ปิดทำการ, เสาร์: ปิดทำการ, อาทิตย์: ปิดทำการ',
    );

    expect(formatOpeningHours(hours, 'unknown')).toBe(
      'จันทร์: 10:00 - 20:00, อังคาร: ปิดทำการ, พุธ: ปิดทำการ, พฤหัสบดี: ปิดทำการ, ศุกร์: ปิดทำการ, เสาร์: ปิดทำการ, อาทิตย์: ปิดทำการ',
    );
  });
});
