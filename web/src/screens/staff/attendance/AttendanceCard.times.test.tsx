// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// #5204 (web): the /staff attendance card shows each row's Bangkok clock-in/out times and marks rows from an earlier
// day. The card filters nothing: the staff_attendance view (#5220) already sends only today's and still-open rows.
// Written before the implementation by the up-front test writer, on the XC FE lane controller claude/8be3a451's
// order. Do not edit, delete or weaken; add tests in a file of your own.
//
// The contract this file assumes (ticket §3, [required]):
//   AttendanceCard.tsx renders attendanceLines(rows(), new Date()). Each row (.xc-attendance-row) shows the name,
//     span.xc-attendance-time with "inText – outText", or inText followed by t(stillIn);
//     an earlier-day row carries the class xc-attendance-row-earlier.
//   No rows => p.xc-attendance-empty with t(empty): TH "วันนี้ยังไม่มีการลงเวลา", EN "No clock-ins today".
// 'now' is injected with fake Date only (vi.useFakeTimers({ toFake: ['Date'] })), so the fetch/microtask flow is real.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeStaffAttendanceRow,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { AttendanceCard } from './AttendanceCard';
import * as attI18n from './i18n';

/** 12:00 ICT on 2026-10-27. */
const NOW = new Date('2026-10-27T05:00:00Z');

const A1 = () =>
  makeStaffAttendanceRow({ id: 'a1', name: 'Somchai', in: '2026-10-27T01:00:00Z', out: '2026-10-27T10:05:00Z' });
const A3 = () => makeStaffAttendanceRow({ id: 'a3', name: 'Ploy', in: '2026-10-26T01:00:00Z', out: null });

function makeCtx(): StaffCtx {
  return {
    me: makeStaffMeView({
      userId: '00000000-0000-4000-8000-0000000000c1',
      name: 'Ploy',
      roles: [],
      branchId: 'MAIN',
      branchName: 'XC-B',
      permissions: ['AttendanceClock'],
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: null,
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function view(data: unknown): Response {
  return new Response(JSON.stringify({ version: 1, changed: true, data }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

function flat(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

describe('AttendanceCard times and earlier-day mark (#5204, now = 2026-10-27T05:00:00Z = 12:00 ICT)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let reads: number;

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(NOW);
    setLang('th');
    reads = 0;
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    setLang('th');
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mount(rows: unknown[]): void {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        reads++;
        return view(rows);
      }),
    );
    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);
  }

  async function loaded(): Promise<void> {
    await vi.waitFor(() => expect(reads).toBeGreaterThan(0));
    for (let i = 0; i < 8; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }

  it('A7 no rows: the card shows p.xc-attendance-empty with t(empty), in TH and EN, and no row', async () => {
    mount([]);
    await vi.waitFor(() => expect(host.querySelector('p.xc-attendance-empty')).not.toBeNull());
    await loaded();
    expect(host.querySelectorAll('.xc-attendance-row')).toHaveLength(0);
    expect(flat(host.querySelector('p.xc-attendance-empty')!.textContent)).toBe('วันนี้ยังไม่มีการลงเวลา');
    expect((attI18n.th as Record<string, string>)['empty']).toBe('วันนี้ยังไม่มีการลงเวลา');
    expect((attI18n.en as Record<string, string>)['empty']).toBe('No clock-ins today');
    setLang('en');
    await vi.waitFor(() =>
      expect(flat(host.querySelector('p.xc-attendance-empty')?.textContent)).toBe('No clock-ins today'),
    );
  });

  it('A8 A1 + A3: two .xc-attendance-row, A3 has xc-attendance-row-earlier, each with its xc-attendance-time', async () => {
    mount([A1(), A3()]);
    await vi.waitFor(() => expect(host.querySelectorAll('.xc-attendance-row')).toHaveLength(2));
    await loaded();
    expect(host.querySelectorAll('.xc-attendance-row')).toHaveLength(2);
    expect(host.querySelector('p.xc-attendance-empty')).toBeNull();

    const a1 = host.querySelector('[data-attendance-id="a1"]');
    const a3 = host.querySelector('[data-attendance-id="a3"]');
    expect(a1?.classList.contains('xc-attendance-row')).toBe(true);
    expect(a3?.classList.contains('xc-attendance-row')).toBe(true);
    expect([...host.querySelectorAll('.xc-attendance-row')]).toEqual([a1, a3]);

    expect(a1!.classList.contains('xc-attendance-row-earlier')).toBe(false);
    expect(a1!.textContent).toContain('Somchai');
    expect(flat(a1!.querySelector('span.xc-attendance-time')?.textContent)).toContain('08:00 – 17:05');
    expect(a1!.textContent).not.toContain(attI18n.th.stillIn);

    expect(a3!.classList.contains('xc-attendance-row-earlier')).toBe(true);
    expect(a3!.textContent).toContain('Ploy');
    expect(flat(a3!.querySelector('span.xc-attendance-time')?.textContent)).toContain('26/10/2026 08:00');
    expect(flat(a3!.textContent)).toMatch(/26\/10\/2026 08:00.*ยังไม่ออก/);
  });
});
