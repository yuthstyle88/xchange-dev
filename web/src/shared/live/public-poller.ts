// RD-2: PublicPoller (docs/design/blueprint/rate-display.md §3, §4.2, D4).

import { createSignal, type Accessor } from 'solid-js';

/** What the poller knows about its route. */
export type PublicPollState = 'loading' | 'ready' | 'notFound' | 'error';

/** The `fetch` the poller calls; the browser's `fetch`, or a fake in tests. */
export type PublicFetch = (input: string, init?: RequestInit) => Promise<Response>;

/**
 * Polls one public route with `If-None-Match` (fleet rule 🔁): a `304` keeps the same `data()` object,
 * an error doubles the interval up to `maxIntervalMs`, and nothing is requested while the tab is hidden.
 */
export class PublicPoller<T> {
  private readonly url: string;
  private readonly fetch: PublicFetch;
  private readonly intervalMs: number;
  private readonly maxIntervalMs: number;

  private readonly _data: Accessor<T | undefined>;
  private readonly setData: (v: T | undefined | ((prev: T | undefined) => T | undefined)) => void;
  private readonly _state: Accessor<PublicPollState>;
  private readonly setState: (v: PublicPollState | ((prev: PublicPollState) => PublicPollState)) => void;
  private readonly _lastOkAt: Accessor<number | undefined>;
  private readonly setLastOkAt: (v: number | undefined | ((prev: number | undefined) => number | undefined)) => void;

  private running = false;
  private currentInterval: number;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private etag: string | undefined;
  private onVisibilityChange: (() => void) | undefined;

  constructor(url: string, fetch: PublicFetch, intervalMs: number, maxIntervalMs: number) {
    this.url = url;
    this.fetch = fetch;
    this.intervalMs = intervalMs;
    this.maxIntervalMs = maxIntervalMs;
    this.currentInterval = intervalMs;

    const [data, setData] = createSignal<T | undefined>(undefined);
    const [state, setState] = createSignal<PublicPollState>('loading');
    const [lastOkAt, setLastOkAt] = createSignal<number | undefined>(undefined);

    this._data = data;
    this.setData = setData;
    this._state = state;
    this.setState = setState;
    this._lastOkAt = lastOkAt;
    this.setLastOkAt = setLastOkAt;
  }

  /** The last body a `200` carried, or `undefined` before the first one. */
  data(): T | undefined {
    return this._data();
  }

  state(): PublicPollState {
    return this._state();
  }

  /** When the route last answered `200` or `304` (`Date.now()` milliseconds), or `undefined` before that. */
  lastOkAt(): number | undefined {
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

    const init: RequestInit =
      this.etag !== undefined
        ? { headers: { 'If-None-Match': this.etag } }
        : {};

    let res: Response;
    try {
      res = await this.fetch(this.url, init);
    } catch {
      this.handleError();
      return;
    }

    if (!this.running) return;

    if (res.status === 200) {
      let body: T;
      try {
        body = (await res.json()) as T;
      } catch {
        this.handleError();
        return;
      }
      if (!this.running) return;

      const newEtag = res.headers.get('ETag');
      this.etag = newEtag ?? undefined;
      this.setData(() => body);
      this.setState('ready');
      this.setLastOkAt(Date.now());
      this.currentInterval = this.intervalMs;
      if (!document.hidden) {
        this.scheduleTick(this.currentInterval);
      }
    } else if (res.status === 304) {
      const newEtag = res.headers.get('ETag');
      if (newEtag !== null) {
        this.etag = newEtag;
      }
      this.setState('ready');
      this.setLastOkAt(Date.now());
      this.currentInterval = this.intervalMs;
      if (!document.hidden) {
        this.scheduleTick(this.currentInterval);
      }
    } else if (res.status === 404) {
      this.setState('notFound');
      this.stop();
    } else {
      this.handleError();
    }
  }

  private handleError(): void {
    if (!this.running) return;
    this.setState('error');
    this.currentInterval = Math.min(this.currentInterval * 2, this.maxIntervalMs);
    if (!document.hidden) {
      this.scheduleTick(this.currentInterval);
    }
  }
}
