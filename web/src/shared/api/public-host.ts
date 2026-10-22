// PH-0 (docs/design/web-public-get.md): the one reader of `GET /api/public/host`, the organisation this host serves.
// Every screen that needs the org calls `readHostOrg(api)` instead of its own raw `fetch` and its own error handling.
//
// `readHostOrg(api)`:
// - one `api.publicGet<{ org?: string }>('/api/public/host')`;
// - `org` a non-empty string ⇒ that string;
// - `org` missing, not a string, or empty ⇒ rejects with `ApiError { code: 'notFound', status: 404,
//   message: 'host org not found' }` (the shape `queue-api.ts` `host()` already used);
// - a refusal or a network failure ⇒ rejects with what `publicGet` rejected with, unchanged (an `ApiError` from
//   `parseError`, or the `TypeError` of `fetch`). It never retries and never words the failure: the caller does both.
import { type ApiClient, ApiError } from './client';

/** The org of the request's host. */
export async function readHostOrg(api: Pick<ApiClient, 'publicGet'>): Promise<string> {
  const res = await api.publicGet<{ org?: unknown } | null>('/api/public/host');
  if (typeof res?.org !== 'string' || res.org === '') {
    throw new ApiError({ code: 'notFound', status: 404, message: 'host org not found' });
  }
  return res.org;
}
