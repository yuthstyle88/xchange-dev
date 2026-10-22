// QW-2g (kiosk-shared-phone §5r F9; rows U6–U8 of §5r.3 and the failed sign-out follow-up; §5r.4a): the borrowed
// tablet on the /app/queue page. Skeleton by QW-2.0; the body is QW-2g's. QW-3's shared/session/tablet.ts is the
// core; this is the page's one call into it.
//
// `useQueueTablet(branchCode, api)`, called inside a component (or a reactive root):
// - `shared` = isSharedTablet() when called.
// - On a shared tablet, arms `armTabletIdle(branchCode, api)` while mounted and disposes it on unmount (U7).
// - `done()` on a shared tablet ⇒ `signOutAndRestart(branchCode, api)` (U6). A failed sign-out ⇒ `error()` = the
//   worded `signOutFailed` text (the page shows it as `role="alert"`) and no restart. On a phone `done()` signs
//   nothing out and leaves the page alone (U8).
import { createSignal, onCleanup, type Accessor } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { armTabletIdle, isSharedTablet, signOutAndRestart } from '../../../shared/session/tablet';
import { en, th, zh } from './i18n';

const tr = screenT('customer.queue', { th, en, zh });

export interface QueueTablet {
  readonly shared: boolean;
  error: Accessor<string | null>;
  done(): Promise<void>;
}

export function useQueueTablet(branchCode: string, api: ApiClient): QueueTablet {
  const shared = isSharedTablet();
  const [error, setError] = createSignal<string | null>(null);

  if (shared) {
    onCleanup(armTabletIdle(branchCode, api, () => setError(tr('signOutFailed'))));
  }

  return {
    shared,
    error,
    async done(): Promise<void> {
      if (!shared) return;
      setError(null);
      try {
        await signOutAndRestart(branchCode, api);
      } catch {
        setError(tr('signOutFailed'));
      }
    },
  };
}
