// #5204 review: the server leaves `out` off a still-in row (`skip_serializing_if` on `StaffAttendanceRow.out_time`),
// so the row the card gets has no `out` key at all, not `out: null`.
import { describe, expect, it } from 'vitest';
import { makeStaffAttendanceRow } from '../../../shared/testing/fixtures';
import { attendanceLines } from './attendance-rows';

type Row = Parameters<typeof attendanceLines>[0][number];

describe('attendanceLines on the wire shape', () => {
  it('W1 a row with no out key is still in: "09:00", outText null, stillIn', () => {
    const wire: Record<string, unknown> = {
      ...makeStaffAttendanceRow({ id: 'w1', name: 'n', in: '2026-10-27T02:00:00Z' }),
    };
    delete wire.out;
    const [line] = attendanceLines([wire as unknown as Row], new Date('2026-10-27T05:00:00Z'));
    expect(line).toMatchObject({ inText: '09:00', outText: null, stillIn: true, earlierDay: false });
  });
});
