// Pure coverage rules for the Owner's shift settings screen (design configurable-shifts.md §2.9, T1, T5; ticket CS-14a).
// Times are wall-clock "HH:MM"; an end at or before its start runs into the next day.

/** One row as the editor holds it. */
export interface ShiftRowTimes {
  startsAt: string;
  endsAt: string;
}

/** An uncovered window, "HH:MM" each, the same rule as the server's `PlanRefusal::Gap`. */
export interface CoverageGap {
  from: string;
  to: string;
}

const DAY_MINS = 24 * 60;

function toMinutes(hhmm: string): number | null {
  const m = /^(\d{2}):(\d{2})$/.exec(hhmm);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return null;
  return h * 60 + min;
}

function formatMinutes(total: number): string {
  const wrapped = ((total % DAY_MINS) + DAY_MINS) % DAY_MINS;
  const h = Math.floor(wrapped / 60);
  const m = wrapped % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
}

/** The cutover: the earliest start ("HH:MM"), or null for no rows. */
export function cutoverOf(rows: ShiftRowTimes[]): string | null {
  const starts = rows.map((r) => toMinutes(r.startsAt)).filter((v): v is number => v !== null);
  if (starts.length === 0) return null;
  return formatMinutes(Math.min(...starts));
}

/** Every uncovered window of the 24 hours from the cutover, in order; [] when fully covered; [] for no rows. */
export function coverageGaps(rows: ShiftRowTimes[]): CoverageGap[] {
  const parsed = rows
    .map((r) => ({ start: toMinutes(r.startsAt), end: toMinutes(r.endsAt) }))
    .filter((r): r is { start: number; end: number } => r.start !== null && r.end !== null);
  if (parsed.length === 0) return [];

  const s0 = Math.min(...parsed.map((r) => r.start));
  const segments: [number, number][] = [];
  for (const { start, end } of parsed) {
    const offsetStart = ((start - s0) % DAY_MINS + DAY_MINS) % DAY_MINS;
    const duration = end <= start ? end - start + DAY_MINS : end - start;
    const offsetEnd = offsetStart + duration;
    segments.push([offsetStart, Math.min(offsetEnd, DAY_MINS)]);
    if (offsetEnd > DAY_MINS) {
      segments.push([0, offsetEnd - DAY_MINS]);
    }
  }
  segments.sort((a, b) => a[0] - b[0]);

  const gaps: CoverageGap[] = [];
  let coveredUntil = 0;
  for (const [segStart, segEnd] of segments) {
    if (segStart > coveredUntil) {
      gaps.push({
        from: formatMinutes(s0 + coveredUntil),
        to: formatMinutes(s0 + segStart),
      });
    }
    if (segEnd > coveredUntil) {
      coveredUntil = segEnd;
    }
  }
  if (coveredUntil < DAY_MINS) {
    gaps.push({
      from: formatMinutes(s0 + coveredUntil),
      to: formatMinutes(s0),
    });
  }
  return gaps;
}

/** The default end for a new row: start + `hours` (wraps past midnight), "HH:MM". */
export function defaultEnd(startsAt: string, hours: number): string {
  const start = toMinutes(startsAt) ?? 0;
  return formatMinutes(start + Math.round(hours * 60));
}
