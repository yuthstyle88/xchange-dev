export type CloseForClockOutCode = 'beforeClockIn' | 'inFuture' | 'notEarlierDay';

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

export function toRfc3339Bangkok(local: string): string {
  const trimmed = local.trim();
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(trimmed)) {
    return `${trimmed}:00+07:00`;
  }
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?$/.test(trimmed)) {
    return `${trimmed}+07:00`;
  }
  return trimmed;
}

export function defaultClockOutLocal(clockIn: string): string {
  const d = new Date(clockIn);
  if (Number.isNaN(d.getTime())) return '';
  return `${DAY_FMT.format(d)}T23:59`;
}

export function formatOpenClockIn(clockIn: string): string {
  const d = new Date(clockIn);
  if (Number.isNaN(d.getTime())) return '';
  const [, mm, dd] = DAY_FMT.format(d).split('-');
  return `${dd}/${mm} ${TIME_FMT.format(d)}`;
}

export function checkCloseFor(
  clockIn: string,
  clockOutLocal: string,
  now: Date,
): { field: 'clockOut'; code: CloseForClockOutCode } | undefined {
  const inDate = new Date(clockIn);
  const outDate = new Date(toRfc3339Bangkok(clockOutLocal));
  if (Number.isNaN(inDate.getTime()) || Number.isNaN(outDate.getTime()) || Number.isNaN(now.getTime())) {
    return undefined;
  }
  if (outDate.getTime() < inDate.getTime()) {
    return { field: 'clockOut', code: 'beforeClockIn' };
  }
  if (outDate.getTime() > now.getTime()) {
    return { field: 'clockOut', code: 'inFuture' };
  }
  if (DAY_FMT.format(outDate) >= DAY_FMT.format(now)) {
    return { field: 'clockOut', code: 'notEarlierDay' };
  }
  return undefined;
}
