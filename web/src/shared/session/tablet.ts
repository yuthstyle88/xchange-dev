import type { ApiClient } from '../api/client';
import { idleTimer, TABLET_IDLE_MS } from './idle';

const SHARED_TABLET_KEY = 'xc.sharedTablet';

export function isSharedTablet(): boolean {
  if (typeof window === 'undefined') return false;
  if (new URLSearchParams(window.location.search).get('shared') === '1') {
    try {
      window.sessionStorage.setItem(SHARED_TABLET_KEY, '1');
    } catch {
      // sessionStorage unavailable
    }
    return true;
  }
  try {
    return window.sessionStorage.getItem(SHARED_TABLET_KEY) === '1';
  } catch {
    return false;
  }
}

export async function signOutAndRestart(branch: string, api: ApiClient): Promise<void> {
  await api.post('/api/auth/logout', {});
  window.location.assign(`/app/queue?branch=${encodeURIComponent(branch)}&shared=1`);
}

export function armTabletIdle(branch: string, api: ApiClient): () => void {
  if (!isSharedTablet()) return () => {};
  const timer = idleTimer(TABLET_IDLE_MS, () => {
    void signOutAndRestart(branch, api);
  });
  return () => timer.dispose();
}
