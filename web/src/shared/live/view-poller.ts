// SF-1 (docs/design/web-silent-failures.md §2, B1): the poller reports its own failures. Skeleton by the SF
// skeleton commit; the bodies of `error()` and `lastOkAt()` are SF-a's.
//
// - `error()`: the last failed read as an `ApiError` (a thrown `ApiError` as is; anything else, e.g. a network
//   `TypeError`, as `ApiError {code: 'network', status: 0}`), or null before any failure. The next good read clears
//   it. A 401 is kept as `error()` with status 401, like any other failure. Reactive (a signal).
// - `lastOkAt()`: `Date.now()` at the last good read (`changed: true` or `false`), or null before the first one.
//   A failed read leaves it as it is. Reactive (a signal). Same meaning as `PublicPoller.lastOkAt()`.
// - Backoff and polling do not change: an error doubles the interval up to `maxIntervalMs`, a good read goes back to
//   `intervalMs` (pinned by view-poller.test.ts and view-poller.error.test.ts).
import { createSignal, type Accessor } from 'solid-js';
import { ApiError, type ApiClient, type ViewParams } from '../api/client';

export interface ViewPollerOptions {
  params?: ViewParams;
  intervalMs: number;
  maxIntervalMs: number;
}

export class ViewPoller<T> {
  private readonly client: Pick<ApiClient, 'view'>;
  private readonly name: string;
  private readonly opts: ViewPollerOptions;
  private readonly _data: Accessor<T | undefined>;
  private readonly setData: (v: T | undefined | ((prev: T | undefined) => T | undefined)) => void;
  private readonly _version: Accessor<number | undefined>;
  private readonly setVersion: (v: number | undefined) => void;
  private readonly _error: Accessor<ApiError | null>;
  private readonly setError: (v: ApiError | null) => void;
  private readonly _lastOkAt: Accessor<number | null>;
  private readonly setLastOkAt: (v: number | null) => void;

  private running = false;
  private currentInterval: number;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private lastVersion: number | undefined;
  private onVisibilityChange: (() => void) | undefined;

  constructor(client: Pick<ApiClient, 'view'>, name: string, opts: ViewPollerOptions) {
    this.client = client;
    this.name = name;
    this.opts = opts;
    this.currentInterval = opts.intervalMs;

    const [data, setData] = createSignal<T | undefined>(undefined);
    const [version, setVersion] = createSignal<number | undefined>(undefined);
    const [error, setError] = createSignal<ApiError | null>(null);
    const [lastOkAt, setLastOkAt] = createSignal<number | null>(null);
    this._data = data;
    this.setData = setData;
    this._version = version;
    this.setVersion = setVersion;
    this._error = error;
    this.setError = setError;
    this._lastOkAt = lastOkAt;
    this.setLastOkAt = setLastOkAt;
  }

  get data(): Accessor<T | undefined> {
    return this._data;
  }

  get version(): Accessor<number | undefined> {
    return this._version;
  }

  /** The last failed read, cleared by the next good read; null before any failure. SF-a. */
  error(): ApiError | null {
    return this._error();
  }

  /** `Date.now()` at the last good read; null before the first one. SF-a. */
  lastOkAt(): number | null {
    return this._lastOkAt();
  }

  start(): void {
    if (this.running) return;
    this.running = true;

    this.onVisibilityChange = () => {
      if (document.hidden) {
        if (this.timer) {
          clearTimeout(this.timer);
          this.timer = undefined;
        }
      } else {
        if (this.running && !this.timer) {
          this.scheduleTick(0);
        }
      }
    };
    document.addEventListener('visibilitychange', this.onVisibilityChange);

    if (!document.hidden) {
      this.scheduleTick(0);
    }
  }

  stop(): void {
    this.running = false;
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    if (this.onVisibilityChange) {
      document.removeEventListener('visibilitychange', this.onVisibilityChange);
      this.onVisibilityChange = undefined;
    }
  }

  private scheduleTick(delay: number): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.tick();
    }, delay);
  }

  private async tick(): Promise<void> {
    if (!this.running || document.hidden) return;

    const params: ViewParams = {
      ...this.opts.params,
    };
    if (this.lastVersion !== undefined) {
      params.since = this.lastVersion;
    }

    try {
      const answer = await this.client.view<T>(this.name, params);
      if (!this.running) return;

      this.currentInterval = this.opts.intervalMs;
      this.lastVersion = answer.version;
      this.setVersion(answer.version);
      if (answer.changed && answer.data !== undefined) {
        this.setData(() => answer.data);
      }
      this.setError(null);
      this.setLastOkAt(Date.now());
      this.scheduleTick(this.currentInterval);
    } catch (err) {
      if (!this.running) return;
      this.setError(
        err instanceof ApiError
          ? err
          : new ApiError({
              code: 'network',
              message: err instanceof Error ? err.message : 'network',
              status: 0,
            }),
      );
      this.currentInterval = Math.min(this.currentInterval * 2, this.opts.maxIntervalMs);
      this.scheduleTick(this.currentInterval);
    }
  }
}
