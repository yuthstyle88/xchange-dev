import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createEffect, onCleanup, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import { lang, setLang } from '../../shared/i18n';
import { en } from '../../shared/i18n/en';
import { th } from '../../shared/i18n/th';
import { meOf } from '../../test-support/me';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../shared/testing/page-wide';
import { AdminShell } from './AdminShell';
import { menu as approvalsMenu } from './approvals/menu';
import type { AdminMenu, AdminMenuProps } from './menus';

const REFRESH = 'รีเฟรช';
const ADD_BRANCH = '＋ สาขา';
const LIVE_ONLINE = 'เซิร์ฟเวอร์ · ข้อมูลจริง';
const LIVE_OFFLINE = 'เซิร์ฟเวอร์ · ยังไม่พร้อมใช้งาน';

interface Probe {
  mounts: number;
  cleanups: number;
  branch: string[];
}

let probes: Record<string, Probe> = {};

function makeOverviewMenu(): AdminMenu {
  const isTh = () => lang() === 'th';
  const Component = (props: AdminMenuProps): JSX.Element => {
    const p = probes['overview']!;
    p.mounts++;
    onCleanup(() => {
      p.cleanups++;
    });
    createEffect(() => p.branch.push(props.branch()));
    return <section data-fixture-menu="overview">body of overview</section>;
  };
  return {
    id: 'overview',
    label: () => (isTh() ? 'ภาพรวม' : 'EN overview'),
    icon: '▦',
    order: 1,
    group: 'desk',
    needs: 'all',
    heading: {
      eyebrow: () => 'DAILY OVERVIEW',
      title: () => (isTh() ? 'ภาพรวมหน้าร้าน' : 'EN title overview'),
      subtitle: () => (isTh() ? 'ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก' : 'EN subtitle overview'),
    },
    dated: true,
    Component,
  };
}

interface Branch {
  id: string;
  name: string;
}

interface Read {
  name: string;
  branch?: string;
}

const BRANCHES: Branch[] = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
];

interface World {
  session: string;
  meError: unknown;
  branchesFail: boolean;
  reads: Read[];
}

let world: World;

function refOf(id: string): { id: string; code: string; name: string } {
  return { id, code: id, name: BRANCHES.find((b) => b.id === id)?.name ?? id };
}

function meFor() {
  const b = world.session;
  return meOf({
    id: 'owner',
    role: 'OWNER',
    branch: b,
    name: 'Owner',
    branchName: refOf(b).name,
    branches: BRANCHES.map((x) => refOf(x.id)),
  });
}

const fakeApi = {
  async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
    world.reads.push({ name, branch: q.branch });
    if (name === 'me') {
      if (world.meError) throw world.meError;
      return { version: 3, changed: true, data: meFor() as unknown as T };
    }
    if (name === 'admin_branches') {
      if (world.branchesFail) {
        throw new ApiError({ code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง', status: 500 });
      }
      return {
        version: 3,
        changed: true,
        data: { branches: BRANCHES.map((b) => ({ ...b })) } as T,
      };
    }
    throw new Error(`unexpected view ${name}`);
  },
  async page<R>(_name: string): Promise<R[]> {
    return [];
  },
  async act<B, R>(_area: string, _action: string, _body: B): Promise<R> {
    throw new Error('unexpected act');
  },
  async post<B, R>(_path: string, _body: B): Promise<R> {
    throw new Error('unexpected post');
  },
} as unknown as ApiClient;

function reads(name: string): Read[] {
  return world.reads.filter((r) => r.name === name);
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(menus: AdminMenu[] = [makeOverviewMenu(), approvalsMenu]): Promise<HTMLDivElement> {
  window.history.replaceState(null, '', '/ui/admin/#overview');
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <AdminShell api={fakeApi} menus={menus} />, container);
  await settle();
  return container;
}

function spoken(el: Element): string {
  const copy = el.cloneNode(true) as Element;
  copy.querySelectorAll('[aria-hidden="true"]').forEach((n) => n.remove());
  return (copy.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(document.body.querySelectorAll('button')).filter(
    (b) => spoken(b) === label || (b.textContent ?? '').trim() === label,
  );
  expect(found.length, `button ${JSON.stringify(label)}`).toBeGreaterThanOrEqual(1);
  return found[0]!;
}

function noticeText(): string {
  return (container.querySelector('.notice-bar')?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function hasSidebarLink(label: string): boolean {
  return Array.from(container.querySelectorAll('.admin-nav a')).some((a) => spoken(a) === label);
}

beforeEach(() => {
  probes = { overview: { mounts: 0, cleanups: 0, branch: [] } };
  world = {
    session: 'MAIN',
    meError: null,
    branchesFail: false,
    reads: [],
  };
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = String(input);
      if (url.includes('/health/ready')) {
        return jsonResponse({
          version: '3.0.0',
          build: 'abcdef1234567',
          channel: 'staging',
          builtAt: '2026-10-08T00:00:00Z',
        });
      }
      if (url.startsWith('/api/v3/views/me')) {
        return jsonResponse({ version: 1, changed: true, data: meFor() });
      }
      if (url.startsWith(QUEUE_CARD_PATH)) {
        return jsonResponse(queueCardUnchanged());
      }
      return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
    }),
  );
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
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  container?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('AdminShell when the initial me read fails (#2358)', () => {
  it('shows offline plus the worded error (never online) when me 500s or fails on the network', async () => {
    world.meError = new ApiError({ status: 500, code: 'dbDown', message: 'database down' });
    await mount();
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.serverError']);
    expect(noticeText()).not.toContain(LIVE_ONLINE);
    expect(button(ADD_BRANCH).disabled).toBe(true);

    dispose?.();
    container.remove();

    world.meError = new TypeError('Failed to fetch');
    await mount();
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.network']);
    expect(noticeText()).not.toContain(LIVE_ONLINE);
  });

  it('re-reads me on Refresh after a failed me load, showing permissioned menus and passing the branch to menus', async () => {
    world.session = 'B1';
    world.meError = new TypeError('Failed to fetch');
    await mount();

    expect(reads('me')).toHaveLength(1);
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(hasSidebarLink(approvalsMenu.label())).toBe(false);
    expect(probes['overview']!.branch).toEqual(['']);

    world.meError = null;
    button(REFRESH).click();
    await settle();

    expect(reads('me')).toHaveLength(2);
    expect(reads('admin_branches').map((r) => r.branch)).toEqual(['B1']);
    expect(noticeText()).toContain(LIVE_ONLINE);
    expect(noticeText()).not.toContain(LIVE_OFFLINE);
    expect(hasSidebarLink(approvalsMenu.label())).toBe(true);
    expect(probes['overview']!.branch.at(-1)).toBe('B1');
  });

  it('stays offline with the worded error shown when me fails twice (on mount and on Refresh)', async () => {
    world.meError = new ApiError({ status: 500, code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' });
    await mount();
    expect(reads('me')).toHaveLength(1);
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.serverError']);

    button(REFRESH).click();
    await settle();

    expect(reads('me')).toHaveLength(2);
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.serverError']);
    expect(noticeText()).not.toContain(LIVE_ONLINE);
    expect(button(ADD_BRANCH).disabled).toBe(true);
  });

  it("passes no 'MAIN' fallback to any menu or read while offline with no me answer", async () => {
    world.meError = new ApiError({ status: 503, code: 'unavailable', message: 'unavailable' });
    await mount();

    expect(probes['overview']!.branch).not.toContain('MAIN');
    expect(world.reads.filter((r) => r.name !== 'me').map((r) => r.branch)).not.toContain('MAIN');
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).not.toContain(LIVE_ONLINE);
  });

  it('shows offline plus the refusal on branches failure after me recovers, and recovers to online on the next Refresh', async () => {
    world.meError = new TypeError('Failed to fetch');
    world.branchesFail = true;
    await mount();
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.network']);

    // First Refresh: me succeeds, admin_branches fails
    world.meError = null;
    button(REFRESH).click();
    await settle();
    expect(reads('me')).toHaveLength(2);
    expect(reads('admin_branches')).toHaveLength(1);
    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.serverError']);
    expect(button(ADD_BRANCH).disabled).toBe(true);

    // Second Refresh: me already answered so only admin_branches is re-read, and it recovers
    world.branchesFail = false;
    button(REFRESH).click();
    await settle();
    expect(reads('me')).toHaveLength(2);
    expect(reads('admin_branches')).toHaveLength(2);
    expect(noticeText()).toContain(LIVE_ONLINE);
    expect(noticeText()).not.toContain(LIVE_OFFLINE);
    expect(button(ADD_BRANCH).disabled).toBe(false);
  });

  it('shows the worded sign-in text (not online) when me fails with 401 signIn', async () => {
    world.meError = new ApiError({ status: 401, code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' });
    await mount();

    expect(noticeText()).toContain(LIVE_OFFLINE);
    expect(noticeText()).toContain(th['error.signIn']);
    expect(noticeText()).not.toContain(LIVE_ONLINE);

    setLang('en');
    await settle();
    expect(noticeText()).toContain(en['error.signIn']);
  });
});
