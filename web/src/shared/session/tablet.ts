// BT-F1 (kiosk-shared-phone §7.3 T-D6, T-D10; §7.7; §7.8 rows T6, T7, T10, T12 FE side; §7.9): the branch tablet
// mark, read from the server. Skeleton by the XC FE controller claude/e8020026; the bodies are BT-F1's.
// It replaces QW-3's `?shared=1` + sessionStorage mark, its sign-out and its idle timer (§7.9 "Becomes obsolete").
// There is no `isSharedTablet`, no `signOutAndRestart`, no `armTabletIdle` and no `idle.ts` any more.
//
// The tablet token is the HttpOnly cookie `__Host-xc_tablet` (T-D3). Page script never sees it: the browser sends it
// on every same-origin request, and the server answers `me` from it.
//
// `readTabletMode(fetchFn)`:
// - one `GET /api/branch-tablet/me` (`credentials: 'same-origin'`), read once by a person's page load, never polled
//   (T-D6).
// - `200 {paired: true, tabletId, branchCode, name}` with three non-empty strings ⇒ that, as `TabletMode`.
// - Anything else ⇒ `{paired: false}`, and it never throws: `{paired: false}` (a revoked or unknown cookie, T6/T7), a
//   non-2xx answer (a 404 while the BE is not deployed, a 5xx), a network failure, a body that is not JSON, or a
//   `paired: true` with a field missing or empty. Fail-open to the normal customer flow (T12).
// - `?shared=1` and `sessionStorage` have no effect (T12): the only input is the server's answer.
// - It takes a `Fetch`, not an `ApiClient`: callers pass `fetchFn` and `readTabletMode` builds an `ApiClient` to call `publicGet`.
//
// `pairFromFragment(api, loc?, history?)` (T-D10):
// - `loc.hash` holds `pair=<CODE>` (`#pair=ABCDE-12345`, read as URLSearchParams of the hash) ⇒ one
//   `api.post('/api/branch-tablet/pair', {code})` (through ApiClient: never a raw fetch with POST), and the fragment
//   is always stripped with `history.replaceState(null, '', loc.pathname + loc.search)`, whether the pair worked or
//   not, so the code never stays in the address bar or the history.
// - `200` ⇒ `'paired'` (the server set the cookie). Any refusal (`400 PAIRING_CODE_INVALID`, `429 RATE_LIMITED`) or a
//   network failure ⇒ `'invalid'`. It never throws.
// - No `pair=` in the hash (or an empty one) ⇒ `'none'`: no request, and the URL is left alone.
// - It does not reload. T-D10's reload is the caller's (`useQueueTablet`, BT-F2): on `'paired'` the page reloads,
//   so it starts again as the paired tablet's first page (T-D8), with `me` read afresh.
// - `loc` and `history` default to `window.location` and `window.history`; tests inject them.
import { ApiClient, type Fetch } from '../api/client';

/** The page's one flag (T-D6). */
export type TabletMode = { paired: false } | { paired: true; tabletId: string; branchCode: string; name: string };

export type PairOutcome = 'paired' | 'invalid' | 'none';

function nonEmpty(v: unknown): v is string {
  return typeof v === 'string' && v.length > 0;
}

export async function readTabletMode(fetchFn: Fetch): Promise<TabletMode> {
  try {
    const api = new ApiClient(fetchFn);
    const body = await api.publicGet<Record<string, unknown> | null>('/api/branch-tablet/me');
    if (body?.paired === true && nonEmpty(body.tabletId) && nonEmpty(body.branchCode) && nonEmpty(body.name)) {
      return { paired: true, tabletId: body.tabletId, branchCode: body.branchCode, name: body.name };
    }
  } catch {
    // fail-open to the normal customer flow (T12)
  }
  return { paired: false };
}

export async function pairFromFragment(
  api: ApiClient,
  loc: Pick<Location, 'hash' | 'pathname' | 'search'> = window.location,
  history: Pick<History, 'replaceState'> = window.history,
): Promise<PairOutcome> {
  const code = new URLSearchParams(loc.hash.replace(/^#/, '')).get('pair');
  if (!code) return 'none';
  history.replaceState(null, '', loc.pathname + loc.search);
  try {
    await api.post('/api/branch-tablet/pair', { code });
    return 'paired';
  } catch {
    return 'invalid';
  }
}
