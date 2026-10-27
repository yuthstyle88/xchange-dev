import type { StaffAttendanceRow } from '../../../shared/api/generated/staff_attendance';

export interface AttendanceLine {
  id: string;
  name: string;
  inText: string;
  outText: string | null;
  stillIn: boolean;
  earlierDay: boolean;
}

const DAY_FMT = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'Asia/Bangkok',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
});

const TIME_FMT = new Intl.DateTimeFormat('en-GB', {
  timeZone: 'Asia/Bangkok',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

function parseDate(iso: string): Date | null {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d;
}

export function attendanceLines(rows: readonly StaffAttendanceRow[], now: Date): AttendanceLine[] {
  const nowDay = Number.isNaN(now.getTime()) ? '' : DAY_FMT.format(now);

  return rows.map((row) => {
    const inDate = parseDate(row.in);
    let inText = '';
    let earlierDay = false;

    if (inDate) {
      const inDay = DAY_FMT.format(inDate);
      const time = TIME_FMT.format(inDate);
      earlierDay = Boolean(nowDay) && inDay < nowDay;
      if (earlierDay) {
        const [, mm, dd] = inDay.split('-');
        inText = `${dd}/${mm} ${time}`;
      } else {
        inText = time;
      }
    }

    // The server omits `out` while the person is still in (`skip_serializing_if`, design §3.1 `out?`), although the
    // generated type says `string | null`: treat absent and null alike.
    const stillIn = row.out == null;
    let outText: string | null = null;
    if (row.out != null) {
      const outDate = parseDate(row.out);
      outText = outDate ? TIME_FMT.format(outDate) : '';
    }

    return {
      id: row.id,
      name: row.name,
      inText,
      outText,
      stillIn,
      earlierDay,
    };
  });
}
