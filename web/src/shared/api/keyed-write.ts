import { ApiClient, ApiError } from './client';

export function keyedWrite<B, R>(
  api: ApiClient,
  method: string,
  path: string | ((body: B) => string),
): { send(body: B): Promise<R>; reset(): void } {
  const pending = new Map<string, string>();

  return {
    async send(body: B): Promise<R> {
      const serialized = JSON.stringify(body);
      let key = pending.get(serialized);
      if (!key) {
        key = crypto.randomUUID();
        pending.set(serialized, key);
      }
      const resolvedPath = typeof path === 'function' ? path(body) : path;
      try {
        const result = await api.command<B, R>(method, resolvedPath, body, key);
        if (pending.get(serialized) === key) {
          pending.delete(serialized);
        }
        return result;
      } catch (err: unknown) {
        // 409 idempotencyConflict means the first request with this key is still running,
        // not a refusal: keep the key so the retry gets the server's replay (#2437).
        const stillRunning =
          err instanceof ApiError && err.status === 409 && err.code === 'idempotencyConflict';
        if (err instanceof ApiError && err.status < 500 && !stillRunning) {
          if (pending.get(serialized) === key) {
            pending.delete(serialized);
          }
        }
        throw err;
      }
    },
    reset(): void {
      pending.clear();
    },
  };
}
