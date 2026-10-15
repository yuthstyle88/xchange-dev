import { ApiClient, ApiError } from './client';

export function keyedWrite<B, R>(
  api: ApiClient,
  method: string,
  path: string | ((body: B) => string),
): { send(body: B): Promise<R>; reset(): void } {
  let pending: { key: string; body: string } | null = null;

  return {
    async send(body: B): Promise<R> {
      const serialized = JSON.stringify(body);
      if (!pending || pending.body !== serialized) {
        pending = { key: crypto.randomUUID(), body: serialized };
      }
      const key = pending.key;
      const resolvedPath = typeof path === 'function' ? path(body) : path;
      try {
        const result = await api.command<B, R>(method, resolvedPath, body, key);
        pending = null;
        return result;
      } catch (err: unknown) {
        if (err instanceof ApiError && err.status < 500) {
          pending = null;
        }
        throw err;
      }
    },
    reset(): void {
      pending = null;
    },
  };
}
