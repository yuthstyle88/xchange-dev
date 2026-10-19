// P01 v2 clock (rate-display.md §9.2): HH:MM in Asia/Bangkok with a blinking colon, and the Thai long date.
import { createSignal, onCleanup, type JSX } from 'solid-js';

export interface BoardClockProps {
  /** Injected for tests; defaults to `() => new Date()`. */
  now?: () => Date;
}

function getParts(d: Date) {
  const hm = d.toLocaleTimeString('en-GB', {
    timeZone: 'Asia/Bangkok',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
  const [h, m] = hm.split(':');
  const date = d.toLocaleDateString('th-TH', {
    timeZone: 'Asia/Bangkok',
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    year: 'numeric',
  });
  return { h, m, hm, date };
}

export function BoardClock(props: BoardClockProps): JSX.Element {
  const getNow = () => (props.now ? props.now() : new Date());
  const initial = getParts(getNow());

  const [hour, setHour] = createSignal(initial.h);
  const [minute, setMinute] = createSignal(initial.m);
  const [date, setDate] = createSignal(initial.date);

  let currentHm = initial.hm;
  let currentDate = initial.date;

  const tick = () => {
    const now = getNow();
    const next = getParts(now);
    if (next.hm !== currentHm) {
      currentHm = next.hm;
      setHour(next.h);
      setMinute(next.m);
    }
    if (next.date !== currentDate) {
      currentDate = next.date;
      setDate(next.date);
    }
  };

  const timer = setInterval(tick, 1000);
  onCleanup(() => clearInterval(timer));

  return (
    <div class="xc-rb-clock">
      <span class="xc-rb-time">
        {hour()}
        <i class="xc-rb-colon" aria-hidden="true">:</i>
        {minute()}
      </span>
      <small class="xc-rb-date">{date()}</small>
    </div>
  );
}
