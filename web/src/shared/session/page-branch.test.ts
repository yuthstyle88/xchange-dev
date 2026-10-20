// SB-3 (#842): the one helper every page uses to pick its branch, design
// docs/design/blueprint/org-roles-branches.md §14.4 (♻️: one core, not four copies).
// Written before the implementation by the test-writer claude/5d45b017, on the FE lane controller
// claude/5edf7bb5's order. Do not edit, delete or weaken; add tests in a file of your own.
//
// The rule pinned here ([required], #842 §3 and §8):
//   - `?branch=` present and non-empty → that branch (a named in-scope branch still wins, §14.2);
//   - otherwise `session.me()?.branch`;
//   - otherwise `undefined`. Never the literal 'MAIN'.
// The accessor is reactive: when `me` changes, it follows.
// Assumed: `pageBranch(session: Pick<SessionStore, 'me'>): Accessor<string | undefined>` in
// `web/src/shared/session/page-branch.ts` (#842 §8).
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` fixture (`kind`/`role`/`username`) → `meOf(…)`
// (`web/src/test-support/me.ts`, design §3.3), after main's SB-3 tests reached this branch. Nothing else changes.
import { afterEach, describe, expect, it } from 'vitest';
import { createEffect, createRoot, createSignal } from 'solid-js';
import type { Me } from '../testing/fixtures';
import { pageBranch } from './page-branch';
import { meOf } from '../../test-support/me';

function me(branch: string) {
  return meOf({
    id: 'u-7',
    role: 'FRONT',
    branch,
    name: 'Nok',
    branchName: branch,
    branches: [{ id: branch, code: branch, name: branch }],
  });
}

function session(initial: Me | undefined) {
  const [get, set] = createSignal<Me | undefined>(initial);
  return { store: { me: get }, setMe: (m: Me | undefined) => set(() => m) };
}

afterEach(() => {
  window.history.replaceState(null, '', '/');
});

describe('SB-3 pageBranch', () => {
  it('?branch=B3 wins over me.branch', () => {
    window.history.replaceState(null, '', '/kiosk-queue.html?branch=B3');
    const { store } = session(me('B2'));
    expect(pageBranch(store)()).toBe('B3');
  });

  it('no ?branch= and me.branch = B2 → B2', () => {
    window.history.replaceState(null, '', '/kiosk-queue.html');
    const { store } = session(me('B2'));
    expect(pageBranch(store)()).toBe('B2');
  });

  it('an empty ?branch= counts as none → me.branch', () => {
    window.history.replaceState(null, '', '/kiosk-currencies.html?branch=');
    const { store } = session(me('B2'));
    expect(pageBranch(store)()).toBe('B2');
  });

  it("no ?branch= and me not loaded → undefined, never 'MAIN'", () => {
    window.history.replaceState(null, '', '/devices');
    const { store } = session(undefined);
    expect(pageBranch(store)()).toBeUndefined();
  });

  it('me arriving, then changing: the accessor follows it reactively', () => {
    window.history.replaceState(null, '', '/devices');
    const { store, setMe } = session(undefined);
    const seen: Array<string | undefined> = [];
    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      const branch = pageBranch(store);
      createEffect(() => seen.push(branch()));
    });
    setMe(me('B2'));
    setMe(me('B4'));
    dispose();
    expect(seen).toEqual([undefined, 'B2', 'B4']);
  });
});
