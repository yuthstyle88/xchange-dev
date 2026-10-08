import { createSignal, type Accessor } from 'solid-js';
import { ApiClient, type Fetch, type ViewAnswer, type ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import { ViewPoller } from '../live/view-poller';

export type EndReason = 'expired' | 'switched';

export interface SessionStore {
  me: Accessor<Me | undefined>;
  ended?: Accessor<EndReason | undefined>;
  end?: (reason: EndReason) => void;
  signOut(): Promise<void>;
  dispose(): void;
}

export interface EndingSessionStore extends SessionStore {
  ended: Accessor<EndReason | undefined>;
  end(reason: EndReason): void;
}

export function endsSession(status: number, code: string | undefined): boolean {
  return status === 401 || (status === 400 && code === 'kioskStaff');
}

export function endingFetch(inner: Fetch, onEnd: () => void): Fetch {
  return async (input: string, init?: RequestInit): Promise<Response> => {
    const res = await inner(input, init);

    const method = (init?.method ?? 'GET').toUpperCase();

    let pathname: string;
    try {
      pathname = new URL(input, 'http://localhost').pathname;
    } catch {
      pathname = input.split('?')[0]!;
    }

    if (method === 'GET' && pathname.startsWith('/api/v3/views/')) {
      let code: string | undefined;
      if (res.status === 400) {
        try {
          const data = await res.clone().json();
          code = data?.error?.code ?? data?.code;
        } catch {
          // not json
        }
      }
      if (endsSession(res.status, code)) {
        onEnd();
      }
    }

    return res;
  };
}

export function createSessionStore(api: Pick<ApiClient, 'view' | 'post'>): EndingSessionStore {
  const [signedOut, setSignedOut] = createSignal(false);
  const [ended, setEnded] = createSignal<EndReason | undefined>(undefined);

  let poller!: ViewPoller<Me>;

  const end = (reason: EndReason): void => {
    if (ended() !== undefined) return;
    setEnded(reason);
    poller.stop();
  };

  const wrappedApi: Pick<ApiClient, 'view' | 'post'> = {
    post: (path, body) => api.post(path, body),
    view: async <T>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      try {
        const answer = await api.view<T>(name, params);
        if (name === 'me' && answer.changed && answer.data !== undefined) {
          const held = poller.data();
          const next = answer.data as unknown as Me;
          if (held !== undefined && (next.id !== held.id || next.role !== held.role)) {
            end('switched');
          }
        }
        return answer;
      } catch (err: unknown) {
        const status = (err as { status?: number })?.status;
        const code = (err as { code?: string })?.code;
        if (status !== undefined && endsSession(status, code) && poller.data() !== undefined) {
          end('expired');
        }
        throw err;
      }
    },
  };

  poller = new ViewPoller<Me>(wrappedApi, 'me', {
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
    ended,
    end,
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

let sharedClient: ApiClient | undefined;
let shared: EndingSessionStore | undefined;

export function sharedApi(): ApiClient {
  if (!sharedClient) {
    sharedClient = new ApiClient(
      endingFetch(
        (input, init) => fetch(input, init),
        () => {
          if (shared && shared.me() !== undefined) {
            shared.end('expired');
          }
        },
      ),
    );
  }
  return sharedClient;
}

export function sharedSession(): EndingSessionStore {
  if (!shared) {
    shared = createSessionStore(sharedApi());
  }
  return shared;
}
