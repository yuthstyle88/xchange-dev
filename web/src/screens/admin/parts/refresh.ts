// V2-parts (#476): page-wide refresh counter (ticket §7, design §12.2, §12.6)

import { createSignal } from 'solid-js';

const [tick, setTick] = createSignal(0);

export function refreshTick(): number {
  return tick();
}

export function requestRefresh(): void {
  setTick((t) => t + 1);
}
