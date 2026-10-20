import { createSignal, type Accessor } from 'solid-js';
import { ApiClient, type Fetch, type ViewAnswer, type ViewParams } from '../api/client';
import type { Me, MeRole } from '../api/generated/common';
import { ViewPoller } from '../live/view-poller';
import { goChangePassword, needsPasswordChange, passwordChangeUrl } from './password-change';

export { needsPasswordChange, passwordChangeUrl };

export type EndReason = 'expired' | 'switched' | 'branchChanged';

/** Who an answer names: an id and its role grants (`Me`, or `kiosk_notifications.employee`). */
export interface Account {
  id: string;
  roles: readonly MeRole[];
}

/**
 * The same account (#1030 §3.10, ruling (a)): the same id and the same set of role grants, in any order.
 * Another id, a grant added or removed, or the same role at another branch is a switch. Shared by the
 * session and the queue store.
 */
export function sameAccount(a: Account, b: Account): boolean {
  if (a.id !== b.id) return false;
  const grants = (x: Account) => new Set(x.roles.map((r) => JSON.stringify([r.role, r.branchId])));
  const ga = grants(a);
  const gb = grants(b);
  return ga.size === gb.size && [...ga].every((g) => gb.has(g));
}

export interface SessionStore {
  me: Accessor<Me | undefined>;
  ended?: Accessor<EndReason | undefined>;
  end?: (reason: EndReason) => void;
  switchBranch?(branch: string): Promise<void>;
  signOut(): Promise<void>;
  dispose(): void;
  updateMe?(me: Me): void;
}

export interface EndingSessionStore extends SessionStore {
  ended: Accessor<EndReason | undefined>;
  end(reason: EndReason): void;
  switchBranch(branch: string): Promise<void>;
  updateMe(me: Me): void;
}

export function endsSession(status: number, code: string | undefined): boolean {
  return status === 401 || (status === 400 && code === 'kioskStaff');
}

export function endingFetch(inner: Fetch, onEnd: () => void, onPasswordChange?: () => void): Fetch {
  return async (input: string, init?: RequestInit): Promise<Response> => {
    const res = await inner(input, init);

    if (res.status === 403) {
      let code: string | undefined;
      try {
        const data = await res.clone().json();
        code = data?.error?.code ?? data?.code;
      } catch {
        // not json
      }
      if (needsPasswordChange(res.status, code)) {
        onPasswordChange?.();
        return res;
      }
    }

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
          if (held !== undefined) {
            if (!sameAccount(next, held)) {
              end('switched');
            } else if (next.branch !== held.branch) {
              end('branchChanged');
            }
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

  const [manualMe, setManualMe] = createSignal<Me | undefined>(undefined);

  const me: Accessor<Me | undefined> = () => {
    if (signedOut()) return undefined;
    return manualMe() ?? poller.data();
  };

  const store: EndingSessionStore = {
    me,
    ended,
    end,
    async switchBranch(branch: string): Promise<void> {
      await api.post('/api/auth/session/branch', { branch });
    },
    async signOut(): Promise<void> {
      await api.post('/api/auth/logout', {});
      poller.stop();
      setSignedOut(true);
    },
    dispose(): void {
      poller.stop();
    },
    updateMe(m: Me): void {
      setManualMe(m);
    },
  };
  return store;
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
        goChangePassword,
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
