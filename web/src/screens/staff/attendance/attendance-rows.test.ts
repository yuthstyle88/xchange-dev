// #5204 (web): the Bangkok clock-in/out times of each attendance row on the /staff card, and which rows are from an
// earlier day. The card filters nothing: the staff_attendance view (#5220) already sends only today's rows plus rows
// still clocked in from any day.
// Written before the implementation by the up-front test writer, on the XC FE lane controller claude/8be3a451's
// order. Do not edit, delete or weaken; add tests in a file of your own.
//
// The contract this file assumes (ticket §3, [required]):
//   attendance-rows.ts  export function attendanceLines(rows: readonly StaffAttendanceRow[], now: Date): AttendanceLine[]
//     keeps every row, in the server's order; filters nothing;
//     each line {id, name, inText, outText, stillIn, earlierDay}: inText/outText Bangkok HH:mm, outText null while
//     still in; earlierDay when `in` falls before now's Asia/Bangkok calendar day, and then inText is dd/MM HH:mm.
// Times are Bangkok whatever the machine's zone is: this file must pass under TZ=UTC and TZ=Asia/Bangkok.
import { describe, expect, it } from 'vitest';
import { makeStaffAttendanceRow } from '../../../shared/testing/fixtures';
import { attendanceLines } from './attendance-rows';

/** 12:00 ICT on 2026-10-27. */
const NOW = new Date('2026-10-27T05:00:00Z');

function only(id: string, inAt: string, out: string | null) {
  return attendanceLines([makeStaffAttendanceRow({ id, name: `name-${id}`, in: inAt, out })], NOW);
}

describe('attendanceLines (#5204 behaviour table, now = 2026-10-27T05:00:00Z = 12:00 ICT)', () => {
  it('A1 a row clocked in and out today: Bangkok "08:00" and "17:05", not earlierDay', () => {
    const lines = only('a1', '2026-10-27T01:00:00Z', '2026-10-27T10:05:00Z');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({
      id: 'a1',
      name: 'name-a1',
      inText: '08:00',
      outText: '17:05',
      stillIn: false,
      earlierDay: false,
    });
  });

  it('A2 a row clocked in and out yesterday is kept (the card filters nothing): earlierDay, "26/10 08:00" and "17:00"', () => {
    const lines = only('a2', '2026-10-26T01:00:00Z', '2026-10-26T10:00:00Z');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({
      id: 'a2',
      name: 'name-a2',
      inText: '26/10 08:00',
      outText: '17:00',
      stillIn: false,
      earlierDay: true,
    });
  });

  it('A3 a row still open from yesterday: earlierDay, inText "26/10 08:00", outText null, stillIn', () => {
    const lines = only('a3', '2026-10-26T01:00:00Z', null);
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({
      id: 'a3',
      name: 'name-a3',
      inText: '26/10 08:00',
      outText: null,
      stillIn: true,
      earlierDay: true,
    });
  });

  it('A4 a row clocked in at 00:30 ICT on 10-27 (17:30Z on 10-26) is today: not earlierDay, "00:30" and "01:00"', () => {
    const lines = only('a4', '2026-10-26T17:30:00Z', '2026-10-26T18:00:00Z');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({
      id: 'a4',
      inText: '00:30',
      outText: '01:00',
      stillIn: false,
      earlierDay: false,
    });
  });

  it('A5 a row clocked in at 23:59 ICT on 10-26 (16:59Z) is earlierDay (the midnight edge), though it closed after midnight ICT', () => {
    const lines = only('a5', '2026-10-26T16:59:00Z', '2026-10-26T17:10:00Z');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({
      id: 'a5',
      inText: '26/10 23:59',
      outText: '00:10',
      stillIn: false,
      earlierDay: true,
    });
  });

  it('A6 a row still open from today: inText "09:00", outText null, stillIn, not earlierDay', () => {
    const lines = only('a6', '2026-10-27T02:00:00Z', null);
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({
      id: 'a6',
      inText: '09:00',
      outText: null,
      stillIn: true,
      earlierDay: false,
    });
  });

  it('A9 rows in a given order: the lines keep that order and that count, dropping none', () => {
    const rows = [
      makeStaffAttendanceRow({ id: 'a6', in: '2026-10-27T02:00:00Z', out: null }),
      makeStaffAttendanceRow({ id: 'a2', in: '2026-10-26T01:00:00Z', out: '2026-10-26T10:00:00Z' }),
      makeStaffAttendanceRow({ id: 'a1', in: '2026-10-27T01:00:00Z', out: '2026-10-27T10:05:00Z' }),
      makeStaffAttendanceRow({ id: 'a5', in: '2026-10-26T16:59:00Z', out: '2026-10-26T17:10:00Z' }),
      makeStaffAttendanceRow({ id: 'a4', in: '2026-10-26T17:30:00Z', out: '2026-10-26T18:00:00Z' }),
      makeStaffAttendanceRow({ id: 'a3', in: '2026-10-26T01:00:00Z', out: null }),
    ];
    const lines = attendanceLines(rows, NOW);
    expect(lines).toHaveLength(6);
    expect(lines.map((l) => l.id)).toEqual(['a6', 'a2', 'a1', 'a5', 'a4', 'a3']);
    expect(lines.map((l) => l.earlierDay)).toEqual([false, true, false, true, false, true]);
  });
});
