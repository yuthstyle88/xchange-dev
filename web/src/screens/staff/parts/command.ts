import { ApiClient } from '../../../shared/api/client';

export function command<B, R>(
  api: ApiClient,
  method: string,
  path: string
): { send(body: B): Promise<R> } {
  let key: string | null = null;
  return {
    async send(body: B): Promise<R> {
      if (!key) {
        key = crypto.randomUUID();
      }
      try {
        const result = await api.command<B, R>(method, path, body, key);
        key = null;
        return result;
      } catch (err) {
        throw err;
      }
    },
  };
}
