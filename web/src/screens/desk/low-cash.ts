import { createSignal, getOwner, onCleanup, type Accessor } from 'solid-js';
import type { ApiClient } from '../../shared/api/client';
import type { MyShift } from '../../shared/api/generated/my_shift';

export interface LowCash {
  low: Accessor<string[]>;
  reload(): Promise<void>;
}

export function createLowCash(api: ApiClient): LowCash {
  const [low, setLow] = createSignal<string[]>([]);
  let inFlight: Promise<void> | undefined;
  let queued = false;

  async function readOnce(): Promise<void> {
    try {
      const answer = await api.view<MyShift | null>('my_shift');
      const balances = answer.data?.balances ?? [];
      setLow(balances.filter((b) => b.low).map((b) => b.code));
    } catch {
      // L4: the last answer stays and no error is shown
    }
  }

  function reload(): Promise<void> {
    if (inFlight) {
      queued = true;
      return inFlight;
    }
    inFlight = (async () => {
      do {
        queued = false;
        await readOnce();
      } while (queued);
    })().finally(() => {
      inFlight = undefined;
    });
    return inFlight;
  }

  function onVisibilityChange(): void {
    if (document.visibilityState === 'visible') {
      void reload();
    }
  }

  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', onVisibilityChange);
    if (getOwner()) onCleanup(() => document.removeEventListener('visibilitychange', onVisibilityChange));
  }

  return { low, reload };
}
