import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

const menuModules = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = menuModules['./menu.ts'];
  expect(m, 'web/src/screens/admin/employees/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

const T = {
  heading: 'บัญชีพนักงาน',
  active: 'ใช้งาน',
  disabled: 'ปิดใช้งาน',
  disable: 'ปิดบัญชี',
  disableConfirm: 'ปิดบัญชีนี้และยกเลิกเซสชันทั้งหมด?',
  headers: ['รหัสบัญชี / ชื่อ', 'บทบาท', 'สาขา', 'สถานะ', ''],
} as const;

interface Employee {
  id: string;
  username: string;
  name: string;
  role: string;
  branchId: string;
  active: boolean;
  mustChange: boolean;
}

interface Ops {
  version: number;
  enabled: boolean;
  entity: { company: string; site: string; confirmedBy: string; at: string } | null;
}

interface Read {
  view: string;
  branch: string | null;
  since: string | null;
}

interface Post {
  method: string;
  marker: string | null;
  action: string;
  payload: Record<string, unknown>;
}

function people(): Employee[] {
  return [
    { id: 'e-1', username: 'front01', name: 'Somchai Front', role: 'FRONT', branchId: 'MAIN', active: true, mustChange: false },
    { id: 'e-2', username: 'fin01', name: 'Fah Finance', role: 'FINANCE', branchId: 'A1', active: true, mustChange: true },
    { id: 'e-3', username: 'old01', name: 'Old Backend', role: 'BACKEND', branchId: 'GONE', active: false, mustChange: false },
  ];
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  employees: Employee[] = people();
  branches = [
    { id: 'MAIN', name: 'Main Desk' },
    { id: 'A1', name: 'Alpha Desk' },
    { id: 'B2', name: 'Bravo Desk' },
  ];
  ops: Record<string, Ops> = {
    MAIN: { version: 7, enabled: false, entity: null },
    A1: { version: 7, enabled: true, entity: { company: 'Alpha Co.', site: 'Alpha Site', confirmedBy: 'owner', at: '2026-10-01T00:00:00.000Z' } },
    B2: { version: 4, enabled: false, entity: null },
  };
  refuse: Record<string, string> = {};
  reads: Read[] = [];
  posts: Post[] = [];
  postGate?: Promise<void>;

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const raw = String(input);
    const url = new URL(raw, 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && PAGE_WIDE.includes(url.pathname)) {
      if (url.pathname.endsWith('/me')) {
        return json({ version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } });
      }
      return json({ version: 1, changed: false });
    }
    const view = url.pathname.startsWith('/api/v3/views/') ? url.pathname.slice('/api/v3/views/'.length) : null;
    if (method === 'GET' && view === 'admin_employees') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      return json({ version: 3, changed: true, data: { employees: this.employees.map((e) => ({ ...e })) } });
    }
    if (method === 'GET' && view === 'admin_operations') {
      const branch = url.searchParams.get('branch');
      const since = url.searchParams.get('since');
      this.reads.push({ view, branch, since });
      const ops = this.ops[branch ?? 'MAIN'];
      if (!ops) return json({ error: { code: 'noSuchBranch', field: 'branch', message: 'ไม่พบสาขาที่เลือก กรุณารีเฟรช' } }, 400);
      if (since !== null && Number(since) === ops.version) return json({ version: ops.version, changed: false });
      return json({ version: ops.version, changed: true, data: { enabled: ops.enabled, entity: ops.entity && { ...ops.entity } } });
    }
    if (method === 'GET' && view === 'admin_branches') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      return json({ version: 1, changed: true, data: { branches: this.branches.map((b) => ({ ...b })) } });
    }
    if (method === 'POST' && url.pathname === '/api/employees') {
      const headers = new Headers(init?.headers);
      const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: Record<string, unknown> };
      this.posts.push({ method, marker: headers.get('x-xchange-request'), action: body.action, payload: body.payload });
      if (body.action === 'disable' && this.postGate) {
        await this.postGate;
      }
      const sentence = this.refuse[body.action];
      if (sentence !== undefined) return json({ error: sentence }, 400);
      return json(this.apply(body.action, body.payload));
    }
    return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  };

  private apply(action: string, payload: Record<string, unknown>): unknown {
    const p = payload as Record<string, string>;
    if (action === 'disable') {
      const e = this.employees.find((x) => x.id === p.id);
      if (e) e.active = false;
      return { saved: true };
    }
    return { ok: true };
  }
}

let server: Server;
let dispose: (() => void) | undefined;
let container: HTMLDivElement | undefined;
const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

async function mount(): Promise<void> {
  const menu = theMenu();
  const [branch] = createSignal('MAIN');
  const [shown] = createSignal(true);
  const period: Accessor<{ from: string; to: string }> = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  const Component = menu.Component;
  dispose = render(
    (): JSX.Element => (
      <>
        <Component branch={branch} period={period} shown={shown} />
        <Dialog />
        <Toast />
      </>
    ),
    container,
  );
  await settle();
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function page(): HTMLElement {
  expect(container, 'the menu is mounted').toBeDefined();
  return container!;
}

function buttons(root: ParentNode, text: string): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>('button')).filter((b) => norm(b.textContent) === text);
}

function button(root: ParentNode, text: string): HTMLButtonElement {
  const found = buttons(root, text);
  expect(found.length, `one button "${text}"`).toBe(1);
  return found[0]!;
}

function accountTable(): HTMLTableElement {
  const t = Array.from(page().querySelectorAll('table')).find((x) =>
    Array.from(x.querySelectorAll('th')).some((th) => norm(th.textContent) === T.headers[0]),
  );
  expect(t, 'the account table is shown').toBeDefined();
  return t!;
}

function rowOf(username: string): HTMLTableRowElement {
  const rows = Array.from(accountTable().querySelectorAll<HTMLTableRowElement>('tbody tr')).filter(
    (r) => norm(r.cells[0]?.textContent).includes(username),
  );
  expect(rows.length, `one row for ${username}`).toBe(1);
  return rows[0]!;
}

function toastText(): string {
  return norm(document.querySelector('.toast')?.textContent);
}

function readsSince(mark: number, view: string): Read[] {
  return server.reads.slice(mark).filter((r) => r.view === view);
}

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  await settle();
  closeDialog();
  if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
  dispose?.();
  dispose = undefined;
  container?.remove();
  container = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Admin Employees: Disable in-flight guard (#2665)', () => {
  it('Disable, confirm, click Disable again while the POST is in flight -> One POST; the second click shows no confirm and sends no request; no error toast', async () => {
    await mount();
    let resolvePost: (() => void) | undefined;
    const postGate = new Promise<void>((resolve) => {
      resolvePost = resolve;
    });
    server.postGate = postGate;

    const confirm = vi.fn(() => true);
    vi.stubGlobal('confirm', confirm);

    button(rowOf('front01'), T.disable).click();
    await settle();

    expect(confirm).toHaveBeenCalledTimes(1);
    expect(server.posts.filter((p) => p.action === 'disable')).toHaveLength(1);

    // Second click while in flight
    button(rowOf('front01'), T.disable).click();
    await settle();

    expect(confirm).toHaveBeenCalledTimes(1);
    expect(server.posts.filter((p) => p.action === 'disable')).toHaveLength(1);
    expect(toastText()).toBe('');

    resolvePost!();
    await settle();

    expect(server.posts.filter((p) => p.action === 'disable')).toHaveLength(1);
    expect(toastText()).toBe('');
    expect(norm(rowOf('front01').cells[3]?.textContent)).toBe(T.disabled);
  });

  it('While the disable is in flight -> Every row\'s Disable button is disabled', async () => {
    await mount();
    let resolvePost: (() => void) | undefined;
    const postGate = new Promise<void>((resolve) => {
      resolvePost = resolve;
    });
    server.postGate = postGate;

    vi.stubGlobal('confirm', vi.fn(() => true));

    expect(button(rowOf('front01'), T.disable).disabled).toBe(false);
    expect(button(rowOf('fin01'), T.disable).disabled).toBe(false);

    button(rowOf('front01'), T.disable).click();
    await settle();

    // While in flight, every row's Disable button is disabled
    expect(button(rowOf('front01'), T.disable).disabled).toBe(true);
    expect(button(rowOf('fin01'), T.disable).disabled).toBe(true);

    resolvePost!();
    await settle();

    // The remaining active row's button is enabled again
    expect(button(rowOf('fin01'), T.disable).disabled).toBe(false);
  });

  it('The disable answers 2xx -> The buttons are enabled again; the list re-reads (as today)', async () => {
    await mount();
    vi.stubGlobal('confirm', vi.fn(() => true));
    const mark = server.reads.length;

    button(rowOf('front01'), T.disable).click();
    await settle();

    expect(server.posts.filter((p) => p.action === 'disable')).toHaveLength(1);
    expect(readsSince(mark, 'admin_employees').length).toBeGreaterThanOrEqual(1);
    expect(norm(rowOf('front01').cells[3]?.textContent)).toBe(T.disabled);
    expect(button(rowOf('fin01'), T.disable).disabled).toBe(false);
  });

  it('The disable fails (network or 4xx) -> The worded toast as today; the buttons are enabled again', async () => {
    await mount();
    vi.stubGlobal('confirm', vi.fn(() => true));
    server.refuse.disable = 'ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน';

    button(rowOf('front01'), T.disable).click();
    await settle();

    expect(toastText()).toBe(server.refuse.disable);
    expect(button(rowOf('front01'), T.disable).disabled).toBe(false);
    expect(button(rowOf('fin01'), T.disable).disabled).toBe(false);
    expect(norm(rowOf('front01').cells[3]?.textContent)).toBe(T.active);
  });

  it('Confirm cancelled -> No POST; the buttons stay enabled', async () => {
    await mount();
    vi.stubGlobal('confirm', vi.fn(() => false));

    button(rowOf('front01'), T.disable).click();
    await settle();

    expect(server.posts.filter((p) => p.action === 'disable')).toHaveLength(0);
    expect(button(rowOf('front01'), T.disable).disabled).toBe(false);
    expect(button(rowOf('fin01'), T.disable).disabled).toBe(false);
  });
});
