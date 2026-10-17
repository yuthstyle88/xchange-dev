import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';

export function command<B, R>(
  api: ApiClient,
  method: string,
  path: string,
): { send(body: B): Promise<R>; reset(): void } {
  return keyedWrite<B, R>(api, method, path);
}
