// XC Counter (docs/design/xc-counter-app.md §5.4): "The app re-binds after every sign-in and on each desk load".
// Skeleton by claude/bbe9ce8b; the body is CA-5's. It stays silent until then, because the desk mounts it.
//
// The app emits no sign-in event. `/login` signs in and then navigates (`screens/login/main.tsx:12`), so a sign-in
// reaches the desk as a new page load. Inside a loaded desk, the only signal is `session.me()`: it goes from undefined
// to an account, or to another account's id. A poll that answers the same id is not a sign-in.
import { createEffect, createMemo } from 'solid-js';
import type { ApiClient } from '../api/client';
import type { SessionStore } from '../session/session';
import { bindCounter, getXcCounter } from './bridge';

/**
 * CA-5: call once in a reactive owner. With `getXcCounter()` present, it calls `bindCounter(api)` once for each
 * account `session.me()` becomes. With no `xcCounter`, or while `me()` is undefined, it does nothing.
 */
export function useCounterBinding(api: ApiClient, session: Pick<SessionStore, 'me'>): void {
  const id = createMemo(() => session.me()?.id);
  createEffect(() => {
    if (id() && getXcCounter()) {
      void bindCounter(api);
    }
  });
}
