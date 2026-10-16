// #2519: Every web test worker runs in UTC (including at module evaluation time),
// so any code that forgets `timeZone: 'Asia/Bangkok'` goes red even on a Bangkok machine.
import { describe, expect, it } from 'vitest';

const TOP = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', hourCycle: 'h23' });

describe('web test environment timezone pin (#2519)', () => {
  it('resolves Intl.DateTimeFormat default timeZone to UTC', () => {
    expect(Intl.DateTimeFormat().resolvedOptions().timeZone).toBe('UTC');
  });

  it('computes local Date.prototype.getHours() in UTC', () => {
    expect(new Date('2026-01-01T00:00:00Z').getHours()).toBe(0);
  });

  it('pins UTC before module top-level formatters are constructed', () => {
    expect(TOP.format(new Date('2026-01-01T00:00:00Z'))).toBe('00');
  });
});
