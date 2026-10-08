import { createSignal, type Accessor } from 'solid-js';
import type { ApiClient, ViewParams } from '../api/client';

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
    this._data = data;
    this.setData = setData;
    this._version = version;
    this.setVersion = setVersion;
  }

  get data(): Accessor<T | undefined> {
    return this._data;
  }

  get version(): Accessor<number | undefined> {
    return this._version;
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
      this.scheduleTick(this.currentInterval);
    } catch {
      if (!this.running) return;
      this.currentInterval = Math.min(this.currentInterval * 2, this.opts.maxIntervalMs);
      this.scheduleTick(this.currentInterval);
    }
  }
}
