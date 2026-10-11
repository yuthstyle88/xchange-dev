// V1-account-bar (#402 §7, §8, §9, §10.2 V1/V6): every v3 screen says who is signed in and has a
// sign-out button, from V0-B's SessionStore. Ported from
// xchange-v2-source@4d0bdbb:src/devices-public/account-bar.js — line 8 `{name} · {username} · {role}`,
// lines 43-44 `สาขา {branch}`, line 12 the button `ออกจากระบบ` (logout, then /login).
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/shared/ui/AccountBar.tsx`:
//
//   export function AccountBar(props: { session?: SessionStore; navigate?: (url: string) => void }): JSX.Element;
//
// These tests always hand `session` and `navigate` in; the defaults are not pinned here.
// `Me` is V0-B's, widened by this ticket with `name: string | null` and `username: string | null`
// (web/src/shared/api/generated/common.ts, regenerated from api/src/presentation/dto/v3/common.rs).
//
// The bar is a shared component, so its texts are shared `shell.*` keys of
// web/src/shared/i18n/th.ts and en.ts (design W10 point 1: the shared tables hold the
// shared-component keys `field.*`, `form.*`, `shell.*`, `error.*`):
//
//   'shell.account.branch':  'สาขา {branch}'
//   'shell.account.signOut': 'ออกจากระบบ'
//
// The `{name} · {username} · {role}` line is the account's own data, not a translated text.
// When the account's row is gone (`name` and `username` null) the line is the role alone.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import { fill, setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import type { SessionStore } from '../session/session';
import { AccountBar } from './AccountBar';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
});

const OWNER: Me = { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Somchai', username: 'owner1', branchName: 'MAIN', branches: [] };
const NOK: Me = { kind: 'staff', id: 'employee-7', role: 'FRONT', branch: 'B2', name: 'Nok', username: 'nok', branchName: 'B2', branches: [] };

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A fake SessionStore: `me` is a signal the test sets; `signOut` answers through the function given. */
function fakeSession(initial: Me | undefined, signOut: () => Promise<void> = async () => undefined) {
  const [me, setMe] = createSignal<Me | undefined>(initial);
  const calls = { signOut: 0 };
  const session: SessionStore = {
    me,
    signOut: () => {
      calls.signOut += 1;
      return signOut();
    },
    dispose: () => undefined,
  };
  return { session, setMe, calls };
}

function mount(session: SessionStore) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  dispose = render(() => <AccountBar session={session} navigate={navigate} />, host);
  return { host, navigate };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

/** The button whose accessible text is `name`, found by name, not by position. */
function buttonNamed(host: HTMLElement, name: string): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find(
    (b) => (b.getAttribute('aria-label') ?? b.textContent ?? '').trim() === name,
  );
}

function signOutButton(host: HTMLElement): HTMLButtonElement {
  const b = buttonNamed(host, th['shell.account.signOut']);
  if (!b) throw new Error(`no button named ${JSON.stringify(th['shell.account.signOut'])}`);
  return b;
}

function text(host: HTMLElement): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ');
}

describe('AccountBar texts (#402, W6, W10)', () => {
  it("the shared th table carries v2's Thai verbatim", () => {
    expect(th['shell.account.branch']).toBe('สาขา {branch}');
    expect(th['shell.account.signOut']).toBe('ออกจากระบบ');
  });

  it('the shared en table has both keys, in English, with the same placeholder', () => {
    expect(en['shell.account.branch']).toMatch(/\{branch\}/);
    expect(en['shell.account.branch']).not.toMatch(/[฀-๿]/);
    expect(en['shell.account.signOut'].trim().length).toBeGreaterThan(0);
    expect(en['shell.account.signOut']).not.toMatch(/[฀-๿]/);
  });
});

describe('AccountBar (#402)', () => {
  it('renders nothing while me() is undefined (nobody signed in, or not loaded yet)', async () => {
    const { session } = fakeSession(undefined);
    const { host } = mount(session);
    await settle();
    expect(host.textContent).toBe('');
    expect(host.querySelector('button')).toBeNull();
  });

  it('the owner: "Somchai · owner1 · OWNER", the branch line, and the sign-out button (V1, V6)', async () => {
    const { session } = fakeSession(OWNER);
    const { host, navigate } = mount(session);
    await settle();
    expect(text(host)).toContain('Somchai · owner1 · OWNER');
    expect(text(host)).toContain(fill(th['shell.account.branch'], { branch: 'MAIN' }));
    const button = signOutButton(host);
    expect(button.disabled).toBe(false);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('a staff member: "Nok · nok · FRONT" and "สาขา B2" (V1, V6)', async () => {
    const { session } = fakeSession(NOK);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain('Nok · nok · FRONT');
    expect(text(host)).toContain(fill(th['shell.account.branch'], { branch: 'B2' }));
    expect(text(host)).not.toContain('Somchai');
    signOutButton(host);
  });

  it('an account whose row is gone (name and username null) shows the role alone', async () => {
    const { session } = fakeSession({ ...OWNER, name: null, username: null });
    const { host } = mount(session);
    await settle();
    const shown = text(host);
    expect(shown).toContain('OWNER');
    expect(shown).not.toContain('null');
    expect(shown).not.toContain('·');
    expect(shown).toContain(fill(th['shell.account.branch'], { branch: 'MAIN' }));
    signOutButton(host);
  });

  it('sign-out: the button is disabled while pending, then navigate("/login") once', async () => {
    const pending = deferred<void>();
    const { session, calls } = fakeSession(NOK, () => pending.promise);
    const { host, navigate } = mount(session);
    await settle();
    const button = signOutButton(host);
    button.click();
    await settle();
    expect(calls.signOut).toBe(1);
    expect(button.disabled).toBe(true);
    expect(navigate).not.toHaveBeenCalled();

    pending.resolve();
    await settle();
    expect(navigate.mock.calls).toEqual([['/login']]);
  });

  it('pressed twice quickly: signOut is called once', async () => {
    const pending = deferred<void>();
    const { session, calls } = fakeSession(NOK, () => pending.promise);
    const { host, navigate } = mount(session);
    await settle();
    const button = signOutButton(host);
    button.click();
    button.click();
    await settle();
    button.click();
    await settle();
    expect(calls.signOut).toBe(1);
    pending.resolve();
    await settle();
    expect(calls.signOut).toBe(1);
    expect(navigate.mock.calls).toEqual([['/login']]);
  });

  it('signOut rejects: nothing navigates and the button is enabled again', async () => {
    const { session, calls } = fakeSession(OWNER, async () => {
      throw new Error('network down');
    });
    const { host, navigate } = mount(session);
    await settle();
    const button = signOutButton(host);
    button.click();
    await settle();
    expect(calls.signOut).toBe(1);
    expect(navigate).not.toHaveBeenCalled();
    expect(button.disabled).toBe(false);
  });

  it('follows me(): it appears when someone signs in and disappears when me() becomes undefined', async () => {
    const { session, setMe } = fakeSession(undefined);
    const { host } = mount(session);
    await settle();
    expect(host.textContent).toBe('');

    setMe(NOK);
    await settle();
    expect(text(host)).toContain('Nok · nok · FRONT');
    signOutButton(host);

    setMe(undefined);
    await settle();
    expect(host.textContent).toBe('');
    expect(host.querySelector('button')).toBeNull();
  });

  it('its texts follow setLang (W10)', async () => {
    setLang('en');
    const { session } = fakeSession(NOK);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain(fill(en['shell.account.branch'], { branch: 'B2' }));
    expect(buttonNamed(host, en['shell.account.signOut'])).toBeDefined();
    expect(text(host)).toContain('Nok · nok · FRONT');

    setLang('th');
    await settle();
    expect(text(host)).toContain(fill(th['shell.account.branch'], { branch: 'B2' }));
    signOutButton(host);
  });
});
