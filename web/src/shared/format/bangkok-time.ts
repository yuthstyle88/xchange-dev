const dtf = new Intl.DateTimeFormat('en-GB', {
  timeZone: 'Asia/Bangkok',
  day: '2-digit',
  month: '2-digit',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

export function formatBangkokDateTime(iso: string): string {
  if (!iso) return iso;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const parts = dtf.formatToParts(d);
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
