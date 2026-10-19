// P01 v2 clock (rate-display.md §9.2): HH:MM in Asia/Bangkok with a blinking colon, and the Thai long date.
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';

export interface BoardClockProps {
  /** Injected for tests; defaults to `() => new Date()`. */
  now?: () => Date;
}

export function BoardClock(_props: BoardClockProps): JSX.Element {
  return null;
}
