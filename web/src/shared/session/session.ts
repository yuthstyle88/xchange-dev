import { createSignal, type Accessor } from 'solid-js';
import type { ApiClient } from '../api/client';
import type { Me } from '../api/generated/common';
import { ViewPoller } from '../live/view-poller';

export interface SessionStore {
  me: Accessor<Me | undefined>;
  signOut(): Promise<void>;
  dispose(): void;
}

export function createSessionStore(api: Pick<ApiClient, 'view' | 'post'>): SessionStore {
  const [signedOut, setSignedOut] = createSignal(false);
  const poller = new ViewPoller<Me>(api, 'me', {
    intervalMs: 60_000,
    maxIntervalMs: 60_000,
  });
  poller.start();

  const me: Accessor<Me | undefined> = () => {
    if (signedOut()) return undefined;
    return poller.data();
  };

  return {
    me,
    async signOut(): Promise<void> {
      await api.post('/api/auth', { action: 'logout' });
      poller.stop();
      setSignedOut(true);
    },
    dispose(): void {
      poller.stop();
    },
  };
}
