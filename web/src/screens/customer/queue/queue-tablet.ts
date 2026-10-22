// BT-F2 (kiosk-shared-phone §7.3 T-D7, T-D8, T-D10, T-D11; §7.8 rows T12, T13, T15, T17, T18, T21; §7.9): the branch
// tablet on the /app/queue page. Skeleton by the XC FE controller claude/e8020026; the body is BT-F2's.
// BT-F1's shared/session/tablet.ts is the core; this is the page's one call into it. The page is the same on a phone,
// a home computer and the tablet (R-T5): pairing switches only where the page goes when a request ends.
// There is no idle timer, no sign-out call and no `signOutFailed` (R-T3; §7.9 "Becomes obsolete"): the server ends the
// tablet's customer session at the scan and the cancel (T-D7), and the 30-min cap covers the rest (Q-T1 A).
//
// `useQueueTablet(search, fetchFn)`, called once inside a component (it uses signals):
// - On creation: `pairFromFragment(new ApiClient(fetchFn))` (T-D10). `'paired'` ⇒ `location.reload()` once, and
//   `mode()` stays null (the reloaded page reads `me`). `'invalid'` or `'none'` ⇒ `readTabletMode(fetchFn)`, read
//   once (never polled), and `mode()` becomes its answer.
// - `mode()`: null until that settles; then the `TabletMode`.
// - `branch()`: the branch code the page must use. Paired ⇒ the tablet's `branchCode`, whatever `?branch=` says
//   (T-D11, T13). Otherwise (and while `mode()` is null) the trimmed `?branch=` of `search` ('' when absent).
// - `signInFirst()` (T-D8, T17): true on a paired tablet whose `search` does not carry the sign-in return mark
//   `from=sign-in`. The page then shows its sign-in link first, even when `my_profile` answers 200 (an old session
//   cookie), and shows nothing of the last customer. False on a phone, and while `mode()` is null.
// - `signInNext()`: the `next` of the page's sign-in link. Paired ⇒
//   `/app/queue?branch=<tablet branch>&from=sign-in` (so the person who just signed in gets the form, not sign-in
//   again). Otherwise `/app/queue<search>`, as today.
// - `requestEnded()`: the glue calls it when the open request ends: its status leaves `QUOTED` (`WAITING` after the
//   scan, `CANCELLED` after ยกเลิกคิว), a poll answers `401` after submit (the server deleted the tablet's session,
//   T-D7), or it expired. Paired ⇒ `location.assign('/app/queue?branch=<tablet branch>')` (the first page, T15, T18),
//   once per page: a second call does nothing. On a phone it does nothing (T12: the page stays on the status).
// - `done()` (เสร็จ): paired ⇒ the same as `requestEnded()`. On a phone (or while `mode()` is null) ⇒
//   `location.assign('/app/bookings')` once (U8a).
// - Never calls `/api/auth/logout`, arms no timer and adds no input listener (T21).
import { createSignal, type Accessor } from 'solid-js';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { pairFromFragment, readTabletMode, type TabletMode } from '../../../shared/session/tablet';

export interface QueueTablet {
  /** null until the pairing fragment and `GET /api/branch-tablet/me` settle. */
  mode: Accessor<TabletMode | null>;
  /** The branch code the page uses: the tablet's when paired (T-D11), else the page's `?branch=`. */
  branch: Accessor<string>;
  /** T-D8: show sign-in before anything else, whatever `my_profile` answers. */
  signInFirst: Accessor<boolean>;
  /** Where the sign-in link returns to (its `next`). */
  signInNext: Accessor<string>;
  /** The open request ended (left QUOTED, a 401 poll after submit, cancelled, expired). */
  requestEnded(): void;
  /** เสร็จ. */
  done(): void;
}

export function useQueueTablet(search: string, fetchFn: Fetch): QueueTablet {
  const params = new URLSearchParams(search);
  const pageBranch = params.get('branch')?.trim() ?? '';
  const fromSignIn = params.get('from') === 'sign-in';
  const [mode, setMode] = createSignal<TabletMode | null>(null);
  let ended = false;
  let doneCalled = false;

  void (async () => {
    const outcome = await pairFromFragment(new ApiClient(fetchFn));
    if (outcome === 'paired') {
      location.reload();
      return;
    }
    setMode(await readTabletMode(fetchFn));
  })();

  const branch = (): string => {
    const m = mode();
    return m?.paired ? m.branchCode : pageBranch;
  };

  const signInFirst = (): boolean => {
    const m = mode();
    return Boolean(m?.paired && !fromSignIn);
  };

  const signInNext = (): string => {
    const m = mode();
    return m?.paired ? `/app/queue?branch=${encodeURIComponent(m.branchCode)}&from=sign-in` : `/app/queue${search}`;
  };

  const requestEnded = (): void => {
    const m = mode();
    if (!m?.paired || ended) return;
    ended = true;
    location.assign(`/app/queue?branch=${encodeURIComponent(m.branchCode)}`);
  };

  const done = (): void => {
    if (mode()?.paired) {
      requestEnded();
      return;
    }
    if (doneCalled) return;
    doneCalled = true;
    location.assign('/app/bookings');
  };

  return {
    mode,
    branch,
    signInFirst,
    signInNext,
    requestEnded,
    done,
  };
}
