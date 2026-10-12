// EX-4 (#1186): A01 reaches the admin page the way every menu does, through the shell's discovery of
// `./*/menu.ts` (`web/src/screens/admin/menus.ts`), and only for a user who holds `OverviewRead` (ticket §7
// [required], §9 "a TELLER's menu list"; #1030 §3.10). Written before the implementation by the test-writer
// claude/5d45b017, on the FE lane controller's order (claude/5edf7bb5). Do not edit, delete or weaken.
//
// The page is mounted as `admin/main.tsx` mounts it: `AdminShell` with the real `menus` list. Nothing is
// hand-built here, so the tests go red when the folder is not discovered (no `executive/menu.ts`, a menu id
// that does not match the folder, a `needs` the shell does not honour).
//
// The `me` answers are written in #1030's shape (`roles` + `permissions`), the shape the shell reads once
// #1030 lands; the permission lists are the relevant subset of `RolePermissions::of`
// (`api/crates/xc-auth/src/domain/access/role_permissions.rs`): the EXECUTIVE holds `OverviewRead`, the
// TELLER does not. The real menus' screens build their own `ApiClient` on the global `fetch`; it is stubbed
// with an unchanged answer so no screen errors.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { AdminShell } from '../AdminShell';
import { menus } from '../menus';

const BRANCHES = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
];

interface MeShape {
  id: string;
  name: string;
  roles: { role: string; branchId: string | null }[];
  permissions: string[];
  branch: string;
  branchName: string;
  branches: { id: string; name: string }[];
}

const EXECUTIVE: MeShape = {
  id: 'exec-1',
  name: 'Executive',
  roles: [{ role: 'EXECUTIVE', branchId: null }],
  permissions: ['ApprovalsDecide', 'OverviewRead', 'RatesDraft', 'SignedIn'],
  branch: 'MAIN',
  branchName: 'สำนักงานใหญ่',
  branches: BRANCHES,
};

const TELLER: MeShape = {
  id: 'teller-1',
  name: 'Teller',
  roles: [{ role: 'TELLER', branchId: 'MAIN' }],
  permissions: ['ExchangeTransact', 'QueueOperate', 'ShiftOperate', 'SignedIn', 'TransactionRead'],
  branch: 'MAIN',
  branchName: 'สำนักงานใหญ่',
  branches: BRANCHES,
};

let me: MeShape;

const fakeApi = {
  async view<T>(name: string, _q: ViewParams = {}): Promise<ViewAnswer<T>> {
    if (name === 'me') return { version: 3, changed: true, data: me as unknown as T };
    if (name === 'admin_branches') {
      return { version: 3, changed: true, data: { branches: BRANCHES.map((b) => ({ ...b })) } as T };
    }
    return { version: 1, changed: false };
  },
  async page<R>(): Promise<R[]> {
    return [];
  },
  async act<B, R>(area: string, action: string, _body: B): Promise<R> {
    throw new Error(`the shell calls no v3 action ${area}/${action}`);
  },
  async post<B, R>(_path: string, _body: B): Promise<R> {
    throw new Error('the menu tests post nothing');
  },
};

const api = fakeApi as unknown as ApiClient;

function unchangedFetch(): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify({ version: 1, changed: false, versions: {} }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(url: string): Promise<void> {
  window.history.replaceState(null, '', url);
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <AdminShell api={api} menus={menus} />, container);
  await settle();
}

function navLinks(): HTMLAnchorElement[] {
  return [...container.querySelectorAll<HTMLAnchorElement>('a[href="#executive"]')];
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(unchangedFetch));
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    })),
  );
  setLang('th');
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  container?.remove();
  vi.unstubAllGlobals();
  window.history.replaceState(null, '', '/');
  setLang('th');
  await settle();
});

describe('EX-4 A01: discovered by the admin page, shown by permission (#1186 §7, §9)', () => {
  it('the real menu list (./*/menu.ts) holds executive', () => {
    expect(menus.map((m) => m.id)).toContain('executive');
  });

  it('an EXECUTIVE (OverviewRead) sees A01 in the nav, and #executive opens it', async () => {
    me = EXECUTIVE;
    await mount('/ui/admin/#executive');
    expect(navLinks().length, 'a nav link to #executive').toBeGreaterThan(0);
    const exec = menus.find((m) => m.id === 'executive');
    expect(exec, 'the executive menu').toBeDefined();
    setLang('th');
    const label = exec!.label();
    expect(navLinks().some((a) => (a.textContent ?? '').includes(label)), `the nav link reads ${label}`).toBe(true);
    const active = container.querySelector('[aria-current="page"]');
    expect(active?.getAttribute('href'), 'the opened menu is A01').toBe('#executive');
  });

  it('a TELLER (no OverviewRead) has no A01 in the nav, and #executive does not open it', async () => {
    me = TELLER;
    await mount('/ui/admin/#executive');
    expect(navLinks().length).toBe(0);
    const active = container.querySelector('[aria-current="page"]');
    expect(active?.getAttribute('href') ?? '').not.toBe('#executive');
  });
});
