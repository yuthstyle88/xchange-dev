import { createSignal } from 'solid-js';

let holds = 0;
const [queueCardYielding, setYielding] = createSignal(false);

export { queueCardYielding };

export function yieldQueueCard(): () => void {
  holds += 1;
  setYielding(true);
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holds -= 1;
    setYielding(holds > 0);
  };
}
