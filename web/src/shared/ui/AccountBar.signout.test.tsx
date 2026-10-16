import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../api/client';
import type { Me } from '../api/generated/common';
import { setAudience, setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import type { SessionStore } from '../session/session';
import { meOf } from '../../test-support/me';
import { AccountBar } from './AccountBar';

type BranchRef = { id: string; code: string; name: string };
type MeSB = Me & { branchName: string; branches: BranchRef[] };

const HQ: BranchRef = { id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' };
const B2: BranchRef = { id: 'B2', code: 'XC-02', name: 'สาขาสอง' };

const ADMIN: MeSB = meOf({
  id: 'u-1',
  role: 'ADMIN',
  orgWide: true,
  branch: 'MAIN',
  name: 'Somchai',
  branchName: 'สำนักงานใหญ่',
  branches: [HQ, B2],
});

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setAudience('staff');
  setLang('th');
});

function fakeSession(
  initial: MeSB | undefined,
  opts: {
    signOut?: () => Promise<void>;
    switchBranch?: (branch: string) => Promise<void>;
  } = {},
) {
  const [me] = createSignal<MeSB | undefined>(initial);
  const calls = { signOut: 0, switches: [] as string[] };
  const session: SessionStore = {
    me,
    signOut: () => {
      calls.signOut += 1;
      return (opts.signOut ?? (async () => undefined))();
    },
    switchBranch: (branch: string) => {
      calls.switches.push(branch);
      return (opts.switchBranch ?? (async () => undefined))(branch);
    },
    dispose: () => undefined,
  };
  return { session, calls };
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

function signOutButton(host: HTMLElement, label: string = th['shell.account.signOut']): HTMLButtonElement {
  const btn = Array.from(host.querySelectorAll('button')).find(
    (b) => (b.getAttribute('aria-label') ?? b.textContent ?? '').trim() === label,
  );
  if (!btn) throw new Error(`no button named ${JSON.stringify(label)}`);
  return btn;
}

function text(host: HTMLElement): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ');
}

describe('AccountBar sign-out failure handling (#2162)', () => {
  it('row 1: logout succeeds -> navigate to /login, as today', async () => {
    const { session, calls } = fakeSession(ADMIN);
    const { host, navigate } = mount(session);
    await settle();

    signOutButton(host).click();
    await settle();

    expect(calls.signOut).toBe(1);
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith('/login');
    expect(text(host)).not.toContain(th['shell.account.signOutFailed']);
  });

  it('row 2: logout fails (fetch rejects, or a 5xx) -> the signOutFailed text is shown in the current language; still on the page; the button is usable again', async () => {
    for (const [lang, table, err] of [
      ['th', th, new TypeError('Failed to fetch')] as const,
      ['en', en, new ApiError({ code: 'INTERNAL_ERROR', message: 'HTTP 502', status: 502 })] as const,
      ['zh', zh, new ApiError({ code: 'INTERNAL_ERROR', message: 'HTTP 500', status: 500 })] as const,
    ]) {
      if (lang === 'zh') setAudience('customer');
      else setAudience('staff');
      setLang(lang);
      const { session, calls } = fakeSession(ADMIN, {
        signOut: async () => {
          throw err;
        },
      });
      const { host, navigate } = mount(session);
      await settle();

      const btn = signOutButton(host, table['shell.account.signOut']);
      btn.click();
      await settle();

      expect(calls.signOut).toBe(1);
      expect(text(host)).toContain(table['shell.account.signOutFailed']);
      expect(navigate).not.toHaveBeenCalled();
      expect(btn.disabled).toBe(false);

      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
  });

  it('row 3: fails, then a retry succeeds -> the error is cleared and the user is signed out', async () => {
    let attempt = 0;
    const { session, calls } = fakeSession(ADMIN, {
      signOut: async () => {
        attempt += 1;
        if (attempt === 1) {
          throw new ApiError({ code: 'INTERNAL_ERROR', message: 'HTTP 500', status: 500 });
        }
      },
    });
    const { host, navigate } = mount(session);
    await settle();

    const btn = signOutButton(host);
    btn.click();
    await settle();

    expect(calls.signOut).toBe(1);
    expect(text(host)).toContain(th['shell.account.signOutFailed']);
    expect(navigate).not.toHaveBeenCalled();
    expect(btn.disabled).toBe(false);

    // Retry sign-out
    btn.click();
    await settle();

    expect(calls.signOut).toBe(2);
    expect(text(host)).not.toContain(th['shell.account.signOutFailed']);
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith('/login');
  });

  it('row 4: a branch switch fails -> switchFailed, as today', async () => {
    const { session, calls } = fakeSession(ADMIN, {
      switchBranch: async () => {
        throw new ApiError({ code: 'outOfScope', message: 'refused', status: 403 });
      },
    });
    const { host, navigate } = mount(session);
    await settle();

    const menuBtn = host.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]');
    expect(menuBtn).not.toBeNull();
    menuBtn!.click();
    await settle();

    const b2Item = Array.from(host.querySelectorAll<HTMLElement>('[role="menuitemradio"]')).find(
      (el) => (el.textContent ?? '').trim() === 'สาขาสอง',
    );
    expect(b2Item).toBeDefined();
    b2Item!.click();
    await settle();

    expect(calls.switches).toEqual(['B2']);
    expect(text(host)).toContain(th['shell.account.switchFailed']);
    expect(text(host)).not.toContain(th['shell.account.signOutFailed']);
    expect(navigate).not.toHaveBeenCalled();
  });
});
