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

/** The cutover: the earliest start ("HH:MM"), or null for no rows. */
export function cutoverOf(rows: ShiftRowTimes[]): string | null {
  void rows;
  throw new Error('CS-14a');
}

/** Every uncovered window of the 24 hours from the cutover, in order; [] when fully covered; [] for no rows. */
export function coverageGaps(rows: ShiftRowTimes[]): CoverageGap[] {
  void rows;
  throw new Error('CS-14a');
}

/** The default end for a new row: start + `hours` (wraps past midnight), "HH:MM". */
export function defaultEnd(startsAt: string, hours: number): string {
  void startsAt;
  void hours;
  throw new Error('CS-14a');
}
