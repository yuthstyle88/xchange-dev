// SB-3 (#842) scaffold, written with the up-front tests: the signature only. The implementer
// replaces the body (design docs/design/blueprint/org-roles-branches.md §14.4).
import type { Accessor } from 'solid-js';
import type { SessionStore } from './session';

export function pageBranch(session: Pick<SessionStore, 'me'>): Accessor<string | undefined> {
  return () => {
    let queryBranch: string | undefined = undefined;
    if (typeof window !== 'undefined' && window.location) {
      try {
        const val = new URLSearchParams(window.location.search).get('branch');
        if (val && val.trim() !== '') {
          queryBranch = val.trim();
        }
      } catch {
        // ignore
      }
    }
    const meBranch = session.me()?.branch;
    return queryBranch || (meBranch && meBranch.trim() !== '' ? meBranch.trim() : undefined);
  };
}
