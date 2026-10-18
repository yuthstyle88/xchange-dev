import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { en, th } from './i18n';

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
  entity: { company: string; site: string; confirmedBy: string; at: string } | null;
}

interface Read {
  view: string;
  branch: string | null;
  since: string | null;
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

class FakeServer {
  employees: Employee[] = people();
  branches = [
    { id: 'MAIN', name: 'Main Desk' },
    { id: 'A1', name: 'Alpha Desk' },
    { id: 'B2', name: 'Bravo Desk' },
  ];
  ops: Record<string, Ops> = {
    MAIN: { version: 7, entity: null },
    A1: { version: 7, entity: { company: 'Alpha Co.', site: 'Alpha Site', confirmedBy: 'owner', at: '2026-10-01T00:00:00.000Z' } },
    B2: { version: 4, entity: null },
  };
  reads: Read[] = [];

  readsOf(view: string): Read[] {
    return this.reads.filter((r) => r.view === view);
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const raw = String(input);
    const url = new URL(raw, 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && url.pathname.endsWith('/me')) {
      return json({ version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } });
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
      return json({ version: ops.version, changed: true, data: { entity: ops.entity && { ...ops.entity } } });
    }
    if (method === 'GET' && view === 'admin_branches') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      return json({ version: 1, changed: true, data: { branches: this.branches.map((b) => ({ ...b })) } });
    }
    return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  };
}

const menuModules = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = menuModules['./menu.ts'];
  expect(m, 'menu.ts exists').toBeDefined();
  return m!.menu;
}

let server: FakeServer;
let dispose: (() => void) | undefined;
let container: HTMLDivElement | undefined;
const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

interface Mounted {
  setBranch: (b: string) => void;
  setShown: (s: boolean) => void;
}

async function mount(opts: { branch?: string; shown?: boolean } = {}): Promise<Mounted> {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [shown, setShown] = createSignal(opts.shown ?? true);
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
  return { setBranch, setShown };
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function page(): HTMLElement {
  expect(container, 'container exists').toBeDefined();
  return container!;
}

function filterSelect(): HTMLSelectElement {
  const select = page().querySelector<HTMLSelectElement>('header label select');
  expect(select, 'branch filter select exists in header').not.toBeNull();
  return select!;
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function rowOf(username: string): HTMLTableRowElement {
  const rows = Array.from(page().querySelectorAll<HTMLTableRowElement>('tbody tr'));
  const found = rows.find((r) => norm(r.textContent).includes(username));
  expect(found, `row for ${username}`).toBeDefined();
  return found!;
}

function bodyRows(): HTMLTableRowElement[] {
  return Array.from(page().querySelectorAll<HTMLTableRowElement>('tbody tr'));
}

const THAI = /[฀-๿]/;

beforeEach(() => {
  setLang('th');
  server = new FakeServer();
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
  vi.restoreAllMocks();
  setLang('th');
});

describe('Employees branch filter (#863)', () => {
  it('default "all" shows every row, the same as :550', async () => {
    await mount();
    const select = filterSelect();
    expect(select.value).toBe('');

    const options = Array.from(select.options).map((o) => [o.value, norm(o.textContent)]);
    expect(options).toEqual([
      ['', th.filterAll],
      ['MAIN', 'Main Desk'],
      ['A1', 'Alpha Desk'],
      ['B2', 'Bravo Desk'],
    ]);

    expect(bodyRows()).toHaveLength(3);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');
    expect(norm(rowOf('fin01').textContent)).toContain('Fah Finance');
    expect(norm(rowOf('old01').textContent)).toContain('Old Backend');
  });

  it('picking a branch shows only that branch’s rows', async () => {
    await mount();
    const select = filterSelect();

    choose(select, 'MAIN');
    await settle();
    expect(bodyRows()).toHaveLength(1);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');

    choose(select, 'A1');
    await settle();
    expect(bodyRows()).toHaveLength(1);
    expect(norm(rowOf('fin01').textContent)).toContain('Fah Finance');
  });

  it('back to "all" shows every row', async () => {
    await mount();
    const select = filterSelect();

    choose(select, 'MAIN');
    await settle();
    expect(bodyRows()).toHaveLength(1);

    choose(select, '');
    await settle();
    expect(bodyRows()).toHaveLength(3);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');
    expect(norm(rowOf('fin01').textContent)).toContain('Fah Finance');
    expect(norm(rowOf('old01').textContent)).toContain('Old Backend');
  });

  it('a branch with no accounts shows filterEmpty', async () => {
    await mount();
    const select = filterSelect();

    choose(select, 'B2');
    await settle();
    const rows = bodyRows();
    expect(rows).toHaveLength(1);
    expect(norm(rows[0]!.textContent)).toBe(th.filterEmpty);
    expect(rows[0]!.querySelector('td')?.getAttribute('colspan')).toBe('5');
  });

  it('picking makes no new admin_employees request', async () => {
    await mount();
    const select = filterSelect();
    const mark = server.readsOf('admin_employees').length;

    choose(select, 'MAIN');
    await settle();
    choose(select, 'B2');
    await settle();
    choose(select, '');
    await settle();

    expect(server.readsOf('admin_employees').length).toBe(mark);
  });

  it('switching the sidebar branch keeps the filter as it was', async () => {
    const m = await mount();
    const select = filterSelect();

    choose(select, 'MAIN');
    await settle();
    expect(select.value).toBe('MAIN');
    expect(bodyRows()).toHaveLength(1);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');

    m.setBranch('A1');
    await settle();
    expect(select.value).toBe('MAIN');
    expect(bodyRows()).toHaveLength(1);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');

    m.setBranch('B2');
    await settle();
    expect(select.value).toBe('MAIN');
    expect(bodyRows()).toHaveLength(1);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');
  });

  it('shows English texts when language is English', async () => {
    setLang('en');
    await mount();
    const select = filterSelect();
    expect(select.getAttribute('aria-label')).toBe(en.filterBranch);

    const label = select.closest('label');
    expect(label, 'label exists').not.toBeNull();
    expect(norm(label!.textContent)).toContain(en.filterBranch);

    const options = Array.from(select.options).map((o) => [o.value, norm(o.textContent)]);
    expect(options[0]).toEqual(['', en.filterAll]);

    choose(select, 'B2');
    await settle();
    const rows = bodyRows();
    expect(rows).toHaveLength(1);
    expect(norm(rows[0]!.textContent)).toBe(en.filterEmpty);

    // Verify no Thai in filter UI when language is English
    expect(THAI.test(norm(label!.textContent))).toBe(false);
    expect(THAI.test(norm(rows[0]!.textContent))).toBe(false);
  });

  it('one branch switch makes exactly one new admin_employees read, for the new branch', async () => {
    const m = await mount();
    const mark = server.readsOf('admin_employees').length;

    m.setBranch('A1');
    await settle();

    const next = server.readsOf('admin_employees').slice(mark);
    expect(next).toHaveLength(1);
    expect(next[0]?.branch).toBe('A1');
  });
});
