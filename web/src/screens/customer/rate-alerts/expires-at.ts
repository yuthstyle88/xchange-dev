function offsetAt(fmt: Intl.DateTimeFormat, instant: Date): string | null {
  if (Number.isNaN(instant.getTime())) return null;
  const raw = fmt.formatToParts(instant).find((p) => p.type === 'timeZoneName')?.value;
  if (!raw) return null;
  if (raw === 'GMT') return '+00:00';
  const m = /^GMT([+-]\d{2}:\d{2})$/.exec(raw);
  return m ? m[1]! : null;
}

export function endOfDayInZone(date: string, timeZone: string): string | null {
  const trimmedDate = date.trim();
  const trimmedZone = timeZone.trim();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(trimmedDate) || !trimmedZone) return null;
  let fmt: Intl.DateTimeFormat;
  try {
    fmt = new Intl.DateTimeFormat('en-US', { timeZone: trimmedZone, timeZoneName: 'longOffset' });
  } catch {
    return null;
  }
  const guess = offsetAt(fmt, new Date(`${trimmedDate}T23:59:59Z`));
  if (!guess) return null;
  const offset = offsetAt(fmt, new Date(`${trimmedDate}T23:59:59${guess}`));
  return offset ? `${trimmedDate}T23:59:59${offset}` : null;
}
