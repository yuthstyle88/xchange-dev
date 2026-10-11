// SB-2 (session branch, web half): the account bar shows the session branch by name and is the
// one place a user switches it (design docs/design/blueprint/org-roles-branches.md §14.0 R1, R2;
// §14.4 "The account bar", "i18n"). Written before the implementation by the ticket author
// (claude/dell-claude-3). Do not edit, delete or weaken; add tests in a file of your own.
//
// What SB-1 provides, and this file assumes (design §14.3 `Me`, [required] JSON fields):
//
//   Me.branch     the effective session branch id (unchanged name)
//   Me.branchName that branch's name
//   Me.branches   S, the branches the user may switch to, {id, code, name}[], ordered by the server
//                 by code then id. A user with one branch gets a one-element list.
//
// `MeSB` below spells those fields out so this file type-checks on a main without SB-1; once SB-1
// has regenerated `common.ts`, `Me` already has them and the intersection changes nothing.
//
// The API this file pins ([proposed] names, [required] behaviour):
//
//   SessionStore.switchBranch?(branch: string): Promise<void>
//     posts the switch command (design §14.3 `POST /api/auth/session/branch`); rejects on any refusal.
//   AccountBar(props: { session?: SessionStore; navigate?: (url: string) => void })   (unchanged)
//     after a switch, and when the session ends as 'branchChanged', it calls navigate(<this page's
//     URL without the `branch` query parameter>) — "it reloads the page with any branch query
//     parameter removed" (§14.4).
//
// The shared texts (`web/src/shared/i18n/{th,en,zh}.ts`, §14.4 "i18n"):
//
//   'shell.account.branch'        'สาขา {branch}'   (unchanged; {branch} is now the name)
//   'shell.account.switchBranch'  'สลับสาขา'        the menu's accessible name
//   'shell.account.switchFailed'  'สลับสาขาไม่สำเร็จ'
//   'shell.session.branchChanged' the AppShell alert for EndReason 'branchChanged' (shown until the reload)
//
// Added by SB-2's test writer claude/5d45b017 on the lane controller's ruling 2 (#892
// issuecomment-6104783522, after claude/89bd61d9's gap issuecomment-6094634009): a session whose
// `me.branch` is missing or empty, or whose `branches` is empty, never shows the raw
// `สาขา {branch}`. The bar shows `branchName`, or no branch label at all when there is no name.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../api/client';
import type { Me } from '../api/generated/common';
import { fill, setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import type { EndReason, SessionStore } from '../session/session';
import { AccountBar } from './AccountBar';

type BranchRef = { id: string; code: string; name: string };
type MeSB = Me & { branchName: string; branches: BranchRef[] };

const TH = th as Record<string, string>;
const EN = en as Record<string, string>;
const ZH = zh as Record<string, string>;

// Ordered by code, as the server sends S. The names sort differently (ข < ค < ง would put B3 first
// by name), so a bar that re-sorts by name or by id shows a different order.
const HQ: BranchRef = { id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' };
const B2: BranchRef = { id: 'B2', code: 'XC-02', name: 'สาขาสอง' };
const B3: BranchRef = { id: 'A3', code: 'XC-03', name: 'ขอนแก่น' };

const ADMIN: MeSB = {
  kind: 'user',
  id: 'u-1',
  role: 'ADMIN',
  branch: 'B2',
  name: 'Somchai',
  username: 'admin1',
  branchName: 'สาขาสอง',
  branches: [HQ, B2, B3],
};
const TELLER: MeSB = {
  kind: 'user',
  id: 'u-7',
  role: 'TELLER',
  branch: 'B2',
  name: 'Nok',
  username: 'nok',
  branchName: 'สาขาสอง',
  branches: [B2],
};

let dispose: (() => void) | undefined;

beforeEach(() => {
  window.history.replaceState(null, '', '/ui/admin/?tab=x&branch=MAIN#cash');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
  window.history.replaceState(null, '', '/');
});

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeSession(initial: MeSB | undefined, switchBranch: (b: string) => Promise<void> = async () => undefined) {
  const [me, setMe] = createSignal<MeSB | undefined>(initial);
  const [ended, setEnded] = createSignal<EndReason | undefined>(undefined);
  const switches: string[] = [];
  const session = {
    me,
    ended,
    end: (r: EndReason) => setEnded(() => r),
    signOut: async () => undefined,
    dispose: () => undefined,
    switchBranch: (b: string) => {
      switches.push(b);
      return switchBranch(b);
    },
  };
  return { session: session as SessionStore, setMe, setEnded, switches };
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

function text(host: HTMLElement): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ');
}

function menuButton(host: HTMLElement): HTMLButtonElement | null {
  return host.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]');
}

function menu(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="menu"]');
}

function items(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[role="menuitemradio"]'));
}

function item(name: string): HTMLElement {
  const found = items().filter((i) => (i.textContent ?? '').includes(name));
  expect(found, `one menu item ${JSON.stringify(name)}`).toHaveLength(1);
  return found[0]!;
}

async function openMenu(host: HTMLElement): Promise<void> {
  const b = menuButton(host);
  expect(b, 'the branch text is a menu button').not.toBeNull();
  b!.click();
  await settle();
}

describe('SB-2 texts (§14.4 i18n)', () => {
  it('th: the branch line keeps its text; the switch and failure texts are the design’s', () => {
    expect(TH['shell.account.branch']).toBe('สาขา {branch}');
    expect(TH['shell.account.switchBranch']).toBe('สลับสาขา');
    expect(TH['shell.account.switchFailed']).toBe('สลับสาขาไม่สำเร็จ');
    expect((TH['shell.session.branchChanged'] ?? '').trim().length).toBeGreaterThan(0);
  });

  it('en and zh carry the same keys, not in Thai', () => {
    expect(EN['shell.account.switchBranch']).toBe('Switch branch');
    expect(EN['shell.account.switchFailed']).toBe('Could not switch branch');
    for (const table of [EN, ZH]) {
      for (const key of ['shell.account.switchBranch', 'shell.account.switchFailed', 'shell.session.branchChanged']) {
        expect((table[key] ?? '').trim().length, key).toBeGreaterThan(0);
        expect(table[key], key).not.toMatch(/[฀-๿]/);
      }
    }
  });
});

describe('SB-2 account bar: the branch name (R1)', () => {
  it('shows "สาขา <name>", not the id', async () => {
    const { session } = fakeSession(TELLER);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain(fill(TH['shell.account.branch']!, { branch: 'สาขาสอง' }));
    expect(text(host)).not.toContain(fill(TH['shell.account.branch']!, { branch: 'B2' }));
  });

  it('follows setLang', async () => {
    setLang('en');
    const { session } = fakeSession(TELLER);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain(fill(EN['shell.account.branch']!, { branch: 'สาขาสอง' }));
  });
});

describe('SB-2 account bar: the switch menu (R1, R2)', () => {
  it('one branch: no menu button, and the branch line is plain text', async () => {
    const { session } = fakeSession(TELLER);
    const { host } = mount(session);
    await settle();
    expect(menuButton(host)).toBeNull();
    expect(menu()).toBeNull();
  });

  it('several: a closed menu button; opened, a menu named สลับสาขา lists S in the given order, the current one checked', async () => {
    const { session } = fakeSession(ADMIN);
    const { host } = mount(session);
    await settle();
    const b = menuButton(host)!;
    expect(b).not.toBeNull();
    expect(b.getAttribute('aria-expanded')).toBe('false');
    expect((b.textContent ?? '').replace(/\s+/g, ' ')).toContain(fill(TH['shell.account.branch']!, { branch: 'สาขาสอง' }));
    expect(menu()).toBeNull();

    await openMenu(host);
    expect(b.getAttribute('aria-expanded')).toBe('true');
    expect(menu()?.getAttribute('aria-label')).toBe(TH['shell.account.switchBranch']);
    expect(items().map((i) => (i.textContent ?? '').trim())).toEqual(['สำนักงานใหญ่', 'สาขาสอง', 'ขอนแก่น']);
    expect(items().map((i) => i.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false']);
  });

  it('choosing another branch calls switchBranch(<its id>) once, then reloads this page without ?branch=', async () => {
    const pending = deferred<void>();
    const { session, switches } = fakeSession(ADMIN, () => pending.promise);
    const { host, navigate } = mount(session);
    await settle();
    await openMenu(host);
    item('ขอนแก่น').click();
    await settle();
    expect(switches).toEqual(['A3']);
    expect(navigate, 'no reload before the server answers').not.toHaveBeenCalled();

    pending.resolve();
    await settle();
    expect(navigate.mock.calls).toEqual([['/ui/admin/?tab=x#cash']]);
  });

  it('a page whose only query parameter is branch reloads with no query at all', async () => {
    window.history.replaceState(null, '', '/devices?branch=B2');
    const { session } = fakeSession(ADMIN);
    const { host, navigate } = mount(session);
    await settle();
    await openMenu(host);
    item('สำนักงานใหญ่').click();
    await settle();
    expect(navigate.mock.calls).toEqual([['/devices']]);
  });

  it('a refusal shows สลับสาขาไม่สำเร็จ in the bar, keeps the old name and does not reload', async () => {
    const { session, switches } = fakeSession(ADMIN, async () => {
      throw new ApiError({ code: 'outOfScope', message: 'refused', status: 403 });
    });
    const { host, navigate } = mount(session);
    await settle();
    await openMenu(host);
    item('ขอนแก่น').click();
    await settle();
    expect(switches).toEqual(['A3']);
    expect(text(host)).toContain(TH['shell.account.switchFailed']!);
    expect(text(host)).toContain(fill(TH['shell.account.branch']!, { branch: 'สาขาสอง' }));
    expect(navigate).not.toHaveBeenCalled();
  });

  it('Esc closes the menu without switching', async () => {
    const { session, switches } = fakeSession(ADMIN);
    const { host } = mount(session);
    await settle();
    await openMenu(host);
    expect(menu()).not.toBeNull();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle();
    expect(menu()).toBeNull();
    expect(menuButton(host)!.getAttribute('aria-expanded')).toBe('false');
    expect(switches).toEqual([]);
  });

  it('a click outside closes the menu without switching', async () => {
    const { session, switches } = fakeSession(ADMIN);
    const { host } = mount(session);
    const outside = document.createElement('p');
    document.body.appendChild(outside);
    await settle();
    await openMenu(host);
    expect(menu()).not.toBeNull();
    outside.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    expect(menu()).toBeNull();
    expect(switches).toEqual([]);
  });
});

describe('SB-2 account bar: switched in another tab (§14.4 session store)', () => {
  it("the session ending as 'branchChanged' reloads this page without ?branch=, once", async () => {
    const { session, setEnded } = fakeSession(ADMIN);
    const { navigate } = mount(session);
    await settle();
    expect(navigate).not.toHaveBeenCalled();
    setEnded('branchChanged' as unknown as EndReason);
    await settle();
    expect(navigate.mock.calls).toEqual([['/ui/admin/?tab=x#cash']]);
  });

  it("the session ending as 'expired' or 'switched' does not reload (AppShell's alert stays)", async () => {
    for (const reason of ['expired', 'switched'] as const) {
      const { session, setEnded } = fakeSession(ADMIN);
      const { navigate } = mount(session);
      await settle();
      setEnded(reason);
      await settle();
      expect(navigate, reason).not.toHaveBeenCalled();
      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
  });
});

describe('ruling 2: a missing or empty branch never shows the raw สาขา {branch}', () => {
  const template = TH['shell.account.branch']!;
  const prefix = template.split('{branch}')[0]!.trim();

  function assertNoRawBranch(host: HTMLElement, me: Partial<MeSB>): void {
    const shown = text(host);
    expect(shown, 'no unfilled placeholder').not.toContain('{branch}');
    expect(shown, 'no undefined').not.toContain('undefined');
    expect(shown, 'no null').not.toContain('null');
    if (me.branch) {
      expect(shown, 'never the raw id').not.toContain(fill(template, { branch: me.branch }));
    }
    if (me.branchName) {
      expect(shown, 'the name').toContain(fill(template, { branch: me.branchName }));
    } else {
      expect(shown, 'no branch label at all').not.toContain(prefix);
    }
    expect(menuButton(host), 'no switch menu without branches to switch to').toBeNull();
  }

  it('branch missing: no branch label', async () => {
    const me = { ...ADMIN, branchName: '', branches: [] } as Partial<MeSB>;
    delete me.branch;
    const { session } = fakeSession(me as MeSB);
    const { host } = mount(session);
    await settle();
    expect(text(host), 'the bar still renders the account').toContain('Somchai');
    assertNoRawBranch(host, me);
  });

  it('branch "": no branch label', async () => {
    const me: MeSB = { ...ADMIN, branch: '', branchName: '', branches: [] };
    const { session } = fakeSession(me);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain('Somchai');
    assertNoRawBranch(host, me);
  });

  it('a branch id with no name: no branch label, never the id', async () => {
    const me: MeSB = { ...ADMIN, branch: 'B2', branchName: '', branches: [] };
    const { session } = fakeSession(me);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain('Somchai');
    assertNoRawBranch(host, me);
  });

  it('branches []: the branch name, never the id, and no switch menu', async () => {
    const me: MeSB = { ...ADMIN, branch: 'B2', branchName: 'สาขาสอง', branches: [] };
    const { session } = fakeSession(me);
    const { host } = mount(session);
    await settle();
    expect(text(host)).toContain('Somchai');
    assertNoRawBranch(host, me);
  });
});
