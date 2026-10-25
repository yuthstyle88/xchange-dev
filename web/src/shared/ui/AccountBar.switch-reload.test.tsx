// #5087 (P19): switching branch on a hash-routed screen must leave the bar on the new branch and the page
// reloaded. Up-front tests written before the fix by the test writer claude/studio-claude-5 on the order of
// the lane controller claude/8be3a451. Do not edit, delete or weaken; add tests in a file of your own.
//
// Root cause (main 2bd6d41a): `handleSwitch` (AccountBar.tsx:88-96) navigates to `cleanUrl()`, which keeps the
// hash; on `/#approvals` that is the current URL, so `window.location.assign` is a same-document fragment
// navigation and nothing reloads. `switchBranch` (session.ts:191-193) drops the `Me` the route answers.
//
// These tests mount the bar on a real `createSessionStore` over a fake api, so the bar's branch line follows
// whatever the store holds after the switch (row 1's first half needs the store fix, session.ts).
//
// The API this file pins:
//
//   AccountBar(props: { session?: SessionStore; navigate?: (url: string) => void; reload?: () => void })
//     `reload` [proposed] name: the seam for `window.location.reload()`; the default calls it.
//     After a switch: when `cleanUrl()` equals the current URL ⇒ reload(), and no navigate;
//     otherwise ⇒ navigate(cleanUrl()) as today, and no reload.
//
// Rows of the #5087 behaviour table:
//   1. switch on `/#approvals` (no `?branch=`) — the bar shows the new branch name, and a full reload is requested
//   2. switch on `/?branch=OLD#approvals` — navigates to the URL without `branch` (as today)
//   4. POST fails — error `shell.account.switchFailed`, no navigation, `me` unchanged (as today)
// Row 3 (`switchBranch` applies the answer) is in ../session/session.switch-me.test.ts.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ViewAnswer } from '../api/client';
import { makeBranchRef, type BranchRef, type Me } from '../testing/fixtures';
import { fill } from '../i18n';
import { th } from '../i18n/th';
import { createSessionStore, type EndingSessionStore } from '../session/session';
import { AccountBar } from './AccountBar';
import { meOf } from '../../test-support/me';

type MeSB = Me & { branchName: string; branches: BranchRef[] };

const TH = th as Record<string, string>;

const HQ = makeBranchRef({ id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' });
const KM = makeBranchRef({ id: 'KM', code: 'XC-02', name: 'Kammuay-test' });

const atKM: MeSB = meOf({
  id: 'u-1',
  role: 'ADMIN',
  orgWide: true,
  branch: 'KM',
  name: 'Somchai',
  branchName: 'Kammuay-test',
  branches: [HQ, KM],
});
const atMain: MeSB = { ...atKM, branch: 'MAIN', branchName: 'สำนักงานใหญ่' };

let dispose: (() => void) | undefined;
let store: EndingSessionStore | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  store?.dispose();
  store = undefined;
  document.body.innerHTML = '';
  window.history.replaceState(null, '', '/');
});

/** A store over a fake api: `me` answers `atKM`; the switch POST answers `atMain` (the route's whole body), or fails. */
function realStore(postFails?: Error) {
  const posted: Array<{ path: string; body: unknown }> = [];
  const api = {
    post: async <B, R>(path: string, body: B): Promise<R> => {
      posted.push({ path, body });
      if (postFails) throw postFails;
      return atMain as unknown as R;
    },
    view: async <U,>(): Promise<ViewAnswer<U>> => ({ version: 1, changed: true, data: atKM }) as never,
  };
  store = createSessionStore(api);
  return { store, posted };
}

function mount(session: EndingSessionStore) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  const reload = vi.fn<() => void>();
  // `reload` is the [proposed] prop; typed through Parameters so this file also type-checks before the fix.
  const props = { session, navigate, reload } as Parameters<typeof AccountBar>[0];
  dispose = render(() => <AccountBar {...props} />, host);
  return { host, navigate, reload };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function text(host: HTMLElement): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ');
}

async function switchTo(host: HTMLElement, name: string) {
  const button = host.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]');
  expect(button, 'the branch menu button').not.toBeNull();
  button!.click();
  await settle();
  const item = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitemradio"]')).find(
    (el) => (el.textContent ?? '').trim() === name,
  );
  expect(item, `menu item ${name}`).toBeDefined();
  item!.click();
  await settle();
}

const branchLine = (name: string) => fill(TH['shell.account.branch']!, { branch: name });

describe('#5087 account bar: switching branch on a hash-routed screen', () => {
  it('row 1: on /#approvals the bar shows the new branch name and a full reload is requested (no fragment-only navigate)', async () => {
    window.history.replaceState(null, '', '/#approvals');
    const { store, posted } = realStore();
    const { host, navigate, reload } = mount(store);
    await settle();
    expect(text(host)).toContain(branchLine('Kammuay-test'));

    await switchTo(host, 'สำนักงานใหญ่');

    expect(posted).toEqual([{ path: '/api/auth/session/branch', body: { branch: 'MAIN' } }]);
    expect(text(host), 'the bar follows the Me the switch answered').toContain(branchLine('สำนักงานใหญ่'));
    expect(text(host)).not.toContain(branchLine('Kammuay-test'));
    expect(reload, 'cleanUrl() equals the current URL, so the page must reload').toHaveBeenCalledTimes(1);
    expect(navigate, 'navigating to the same URL with a hash does not reload').not.toHaveBeenCalled();
  });

  it('row 2: on /?branch=OLD#approvals it navigates to /#approvals (as today) and does not reload', async () => {
    window.history.replaceState(null, '', '/?branch=KM#approvals');
    const { store } = realStore();
    const { host, navigate, reload } = mount(store);
    await settle();

    await switchTo(host, 'สำนักงานใหญ่');

    expect(navigate.mock.calls).toEqual([['/#approvals']]);
    expect(reload).not.toHaveBeenCalled();
  });

  it('row 4: a refused POST shows shell.account.switchFailed, neither navigates nor reloads, and keeps me', async () => {
    window.history.replaceState(null, '', '/#approvals');
    const refusal = new ApiError({ code: 'outOfScope', message: 'refused', status: 403 });
    const { store } = realStore(refusal);
    const { host, navigate, reload } = mount(store);
    await settle();

    await switchTo(host, 'สำนักงานใหญ่');

    expect(text(host)).toContain(TH['shell.account.switchFailed']!);
    expect(navigate).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
    expect(store.me()?.branch).toBe('KM');
    expect(text(host)).toContain(branchLine('Kammuay-test'));
  });
});
