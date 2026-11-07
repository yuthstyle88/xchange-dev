const DEFAULT_ZONE = 'Asia/Bangkok';
const dtfCache = new Map<string, Intl.DateTimeFormat>();

function getDtf(timeZone?: string | null): Intl.DateTimeFormat {
  const tz = timeZone || DEFAULT_ZONE;
  let fmt = dtfCache.get(tz);
  if (fmt) return fmt;
  try {
    fmt = new Intl.DateTimeFormat('en-GB', {
      timeZone: tz,
      day: '2-digit',
      month: '2-digit',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      hourCycle: 'h23',
    });
  } catch {
    fmt = getDtf(DEFAULT_ZONE);
  }
  dtfCache.set(tz, fmt);
  return fmt;
}

export function formatDateTimeInZone(iso: string, timeZone?: string | null): string {
  if (!iso) return iso;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const parts = getDtf(timeZone).formatToParts(d);
  let day = '';
  let month = '';
  let year = '';
  let hour = '';
  let minute = '';
  for (const p of parts) {
    if (p.type === 'day') day = p.value;
    else if (p.type === 'month') month = p.value;
    else if (p.type === 'year') year = p.value;
    else if (p.type === 'hour') hour = p.value;
    else if (p.type === 'minute') minute = p.value;
  }
  return `${day}/${month}/${year} ${hour}:${minute}`;
}

export function formatDateInZone(value: string, timeZone?: string | null): string {
  if (!value) return value;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (m) {
    const y = parseInt(m[1]!, 10);
    const mo = parseInt(m[2]!, 10);
    const d = parseInt(m[3]!, 10);
    const dt = new Date(Date.UTC(y, mo - 1, d));
    if (dt.getUTCFullYear() !== y || dt.getUTCMonth() + 1 !== mo || dt.getUTCDate() !== d) {
      return value;
    }
    return `${m[3]}/${m[2]}/${m[1]}`;
  }
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  const parts = getDtf(timeZone).formatToParts(d);
  let day = '';
  let month = '';
  let year = '';
  for (const p of parts) {
    if (p.type === 'day') day = p.value;
    else if (p.type === 'month') month = p.value;
    else if (p.type === 'year') year = p.value;
  }
  return `${day}/${month}/${year}`;
}

export function formatBangkokDateTime(iso: string): string {
  return formatDateTimeInZone(iso, DEFAULT_ZONE);
}

export function formatBangkokDate(value: string): string {
  return formatDateInZone(value, DEFAULT_ZONE);
}
