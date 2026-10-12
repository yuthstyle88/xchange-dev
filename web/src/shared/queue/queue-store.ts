import {
  createEffect,
  createSignal,
  onCleanup,
  untrack,
  type Accessor,
} from 'solid-js';
import { ApiClient, ApiError, type ViewParams } from '../api/client';
import { sameAccount, sharedSession, type Account, type SessionStore } from '../session/session';
import { pageBranch } from '../session/page-branch';
import type { QueueBoardData } from './wire';

export type QueueError =
  | { kind: 'offline' }
  | { kind: 'ended'; reason: 'expired' | 'switched' };

export interface QueueStore {
  state: Accessor<QueueBoardData | undefined>;
  error: Accessor<QueueError | undefined>;
  lastUpdated: Accessor<number | undefined>;
  version: Accessor<number | undefined>;
  now(): number;
  refresh(): Promise<void>;
}

export interface QueueStoreDeps {
  api?: ApiClient;
  session?: SessionStore;
}

export interface QueueStoreInternal extends QueueStore {
  retain(): void;
  release(): void;
}

class BranchQueueStore implements QueueStoreInternal {
  readonly state: Accessor<QueueBoardData | undefined>;
  private readonly setState: (val: QueueBoardData | undefined) => void;

  readonly error: Accessor<QueueError | undefined>;
  private readonly setError: (val: QueueError | undefined) => void;

  readonly lastUpdated: Accessor<number | undefined>;
  private readonly setLastUpdated: (val: number | undefined) => void;

  readonly version: Accessor<number | undefined>;
  private readonly setVersion: (val: number | undefined) => void;

  private inFlight = false;
  private inFlightPromise: Promise<void> | null = null;
  private stopped = false;
  private retainCount = 0;
  private pollTimer: ReturnType<typeof setInterval> | null = null;
  private changeTimer: ReturnType<typeof setTimeout> | null = null;
  private lastMe: Account | undefined = undefined;
  private meInitialized = false;

  private onFocus: (() => void) | null = null;
  private onVisibility: (() => void) | null = null;
  private onChange: (() => void) | null = null;

  constructor(
    readonly branch: string,
    private readonly api: ApiClient,
    private readonly session: SessionStore,
  ) {
    const [state, setState] = createSignal<QueueBoardData | undefined>(undefined);
    const [error, setError] = createSignal<QueueError | undefined>(undefined);
    const [lastUpdated, setLastUpdated] = createSignal<number | undefined>(undefined);
    const [version, setVersion] = createSignal<number | undefined>(undefined);

    this.state = state;
    this.setState = setState;
    this.error = error;
    this.setError = setError;
    this.lastUpdated = lastUpdated;
    this.setLastUpdated = setLastUpdated;
    this.version = version;
    this.setVersion = setVersion;
  }

  now(): number {
    return Date.now();
  }

  retain(): void {
    this.retainCount += 1;
    if (this.retainCount === 1 && !this.stopped) {
      this.attachListeners();
      this.pollTimer = setInterval(() => {
        void this.poll();
      }, 5000);
      void this.poll();
    }
  }

  release(): void {
    this.retainCount -= 1;
    if (this.retainCount <= 0) {
      this.retainCount = 0;
      if (!this.stopped) {
        this.clearTimersAndListeners();
      }
    }
  }

  refresh(): Promise<void> {
    if (this.stopped) {
      return Promise.resolve();
    }
    return this.poll();
  }

  private attachListeners(): void {
    if (typeof window === 'undefined') return;

    this.onFocus = () => {
      void this.poll();
    };
    this.onVisibility = () => {
      if (typeof document !== 'undefined' && !document.hidden) {
        void this.poll();
      }
    };
    this.onChange = () => {
      if (this.changeTimer !== null) {
        clearTimeout(this.changeTimer);
      }
      this.changeTimer = setTimeout(() => {
        this.changeTimer = null;
        void this.poll();
      }, 100);
    };

    window.addEventListener('focus', this.onFocus);
    if (typeof document !== 'undefined') {
      document.addEventListener('visibilitychange', this.onVisibility);
      document.addEventListener('change', this.onChange);
    }
  }

  private clearTimersAndListeners(): void {
    if (this.pollTimer !== null) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
    if (this.changeTimer !== null) {
      clearTimeout(this.changeTimer);
      this.changeTimer = null;
    }
    if (typeof window !== 'undefined' && this.onFocus) {
      window.removeEventListener('focus', this.onFocus);
    }
    if (typeof document !== 'undefined') {
      if (this.onVisibility) {
        document.removeEventListener('visibilitychange', this.onVisibility);
      }
      if (this.onChange) {
        document.removeEventListener('change', this.onChange);
      }
    }
    this.onFocus = null;
    this.onVisibility = null;
    this.onChange = null;
  }

  private stop(): void {
    this.stopped = true;
    this.clearTimersAndListeners();
  }

  private poll(): Promise<void> {
    if (this.stopped) {
      return Promise.resolve();
    }
    if (this.inFlight) {
      return this.inFlightPromise ?? Promise.resolve();
    }

    const currentMe = this.session.me();
    let sinceToSend: number | undefined = this.version();

    if (!this.meInitialized) {
      sinceToSend = undefined;
      this.lastMe = currentMe;
      this.meInitialized = true;
    } else if (
      currentMe === undefined || this.lastMe === undefined
        ? currentMe !== this.lastMe
        : !sameAccount(currentMe, this.lastMe)
    ) {
      sinceToSend = undefined;
      this.lastMe = currentMe;
    }

    const params: ViewParams = { branch: this.branch };
    if (sinceToSend !== undefined) {
      params.since = sinceToSend;
    }

    this.inFlight = true;
    const run = async () => {
      try {
        // QU-6 (#1354): the card and the admin bar read `queue_board` (design queue-tickets.md §4.4–§4.5). It
        // names no employee and no server time, so an account switch is caught by `me()` (no `since`, above)
        // and the clock is this browser's.
        const answer = await this.api.view<QueueBoardData>('queue_board', params);
        if (this.stopped) return;

        this.setError(undefined);
        this.setLastUpdated(Date.now());
        if (answer.changed && answer.data) {
          this.setVersion(answer.version);
          this.setState(answer.data);
        }
      } catch (err: unknown) {
        if (this.stopped) return;

        let status: number | undefined;
        let code: string | undefined;

        if (err instanceof ApiError) {
          status = err.status;
          code = err.code;
        } else if (
          typeof err === 'object' &&
          err !== null &&
          'status' in err
        ) {
          status = (err as { status?: number }).status;
          code = (err as { code?: string }).code;
        }

        if (status === 401 || code === 'kioskStaff') {
          this.setError({ kind: 'ended', reason: 'expired' });
          this.stop();
          return;
        }

        this.setError({ kind: 'offline' });
      } finally {
        this.inFlight = false;
        this.inFlightPromise = null;
      }
    };

    this.inFlightPromise = run();
    return this.inFlightPromise;
  }
}

const stores = new Map<string, BranchQueueStore>();

export function queueStore(branch: string, deps?: QueueStoreDeps): QueueStore {
  let existing = stores.get(branch);
  if (!existing) {
    const api = deps?.api ?? new ApiClient();
    const session = deps?.session ?? sharedSession();
    existing = new BranchQueueStore(branch, api, session);
    stores.set(branch, existing);
  }
  return existing;
}

const idleStore: QueueStore = {
  state: () => undefined,
  error: () => undefined,
  lastUpdated: () => undefined,
  version: () => undefined,
  now: () => Date.now(),
  refresh: async () => {},
};

export function useQueueStore(branch: Accessor<string | undefined>): Accessor<QueueStore> {
  let prevBranch: string | undefined = undefined;
  let prevStore: QueueStoreInternal | undefined = undefined;

  const initialBranch = untrack(branch);
  const initialStore = initialBranch ? untrack(() => queueStore(initialBranch)) : idleStore;
  const [currentStore, setCurrentStore] = createSignal<QueueStore>(initialStore);

  createEffect(() => {
    const b = branch();
    if (b !== prevBranch) {
      if (prevStore) {
        prevStore.release();
        prevStore = undefined;
      }
      prevBranch = b;
      if (b) {
        const s = untrack(() => queueStore(b)) as QueueStoreInternal;
        prevStore = s;
        setCurrentStore(() => s);
        s.retain();
      } else {
        setCurrentStore(() => idleStore);
      }
    }
  });

  onCleanup(() => {
    if (prevStore) {
      prevStore.release();
      prevStore = undefined;
      prevBranch = undefined;
    }
  });

  return currentStore;
}

const [explicitBranch, setExplicitBranch] = createSignal<string | undefined>(
  undefined,
);

export function setQueueBranch(branch: string): void {
  setExplicitBranch(branch);
}

export const queueBranch: Accessor<string | undefined> = () => {
  const explicit = explicitBranch();
  if (explicit !== undefined && explicit !== '') {
    return explicit;
  }
  return pageBranch(sharedSession())();
};
