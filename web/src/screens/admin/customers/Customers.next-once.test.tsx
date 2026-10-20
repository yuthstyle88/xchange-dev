import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { menu } from './menu';

const T = {
  prev: 'ก่อนหน้า',
  next: 'ถัดไป',
  listTab: 'รายชื่อลูกค้าและประวัติ',
};

interface CustomerRow {
  id: string;
  name: string;
  nationality: string;
  created: string;
  member: boolean;
  completed: number;
  volume: string;
  lastAt: string | null;
}

function cx(n: number): string {
  return `CX-${String(n).padStart(8, '0')}`;
}

function customer(n: number, over: Partial<CustomerRow> = {}): CustomerRow {
  return {
    id: cx(n),
    name: `Customer ${n}`,
    nationality: 'THA',
    created: '2026-10-01T02:03:04.000Z',
    member: false,
    completed: 2,
    volume: '6420.00',
    lastAt: '2026-10-08T03:04:05.000Z',
    ...over,
  };
}

function customers(from: number, count: number): CustomerRow[] {
  return Array.from({ length: count }, (_, i) => customer(from + i));
}

type Refusal = { status: number; code: string; message: string; field?: string };
type ListAnswer = { rows: CustomerRow[] } | Refusal;

interface Read<A> {
  params: URLSearchParams;
  respond: (answer: A) => void;
}

let lists: Read<ListAnswer>[];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewResponse(answer: ListAnswer): Response {
  if ('rows' in answer) return json(200, { version: 3, changed: true, data: answer.rows });
  const error: Record<string, string> = { code: answer.code, message: answer.message };
  if (answer.field) error.field = answer.field;
  return json(answer.status, { error });
}

function serve(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const view = url.pathname.startsWith('/api/v3/views/')
    ? url.pathname.replace(/^\/api\/v3\/views\//, '')
    : null;
  if (view === 'admin_customers') {
    return new Promise<Response>((resolve) => {
      const read: Read<ListAnswer> = {
        params: url.searchParams,
        respond: (a) => resolve(viewResponse(a)),
      };
      lists.push(read);
    });
  }
  if (view === 'admin_branches') {
    return Promise.resolve(
      json(200, {
        version: 3,
        changed: true,
        data: { branches: [{ id: 'MAIN', name: 'Main Branch' }] },
      }),
    );
  }
  return Promise.resolve(json(200, { version: 3, changed: false }));
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    for (let j = 0; j < 5; j++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function until(ok: () => boolean, what: string): Promise<void> {
  const start = Date.now();
  while (!ok()) {
    if (Date.now() - start > 2000) throw new Error(`timed out waiting for: ${what}`);
    await settle();
  }
  await settle();
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(opts: { branch?: string } = {}): void {
  const [branch] = createSignal(opts.branch ?? 'MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  const C = menu.Component;
  dispose = render(
    (): JSX.Element => (
      <>
        <C branch={branch} period={period} shown={shown} />
        <Dialog />
      </>
    ),
    root,
  );
}

function listPanel(): HTMLElement {
  const tab = [...root.querySelectorAll<HTMLElement>('[role="tab"]')].find(
    (b) => (b.textContent ?? '').trim() === T.listTab,
  );
  const id = tab?.getAttribute('aria-controls');
  const panel = id ? document.getElementById(id) : null;
  return panel ?? root;
}

function bodyRows(scope: ParentNode = listPanel()): HTMLTableRowElement[] {
  return [...scope.querySelectorAll<HTMLTableRowElement>('table tbody tr')];
}

function shownIds(): string[] {
  return bodyRows().map((tr) => {
    const id = [...tr.querySelectorAll('button')].find((b) => /^CX-/.test(b.textContent ?? ''));
    return id?.textContent ?? '';
  });
}

function buttonIn(scope: ParentNode, label: string): HTMLButtonElement {
  const b = [...scope.querySelectorAll('button')].find(
    (x) => (x.textContent ?? '').trim() === label,
  );
  expect(b, `a button "${label}"`).toBeDefined();
  return b!;
}

function button(label: string): HTMLButtonElement {
  return buttonIn(listPanel(), label);
}

function pageText(scope: ParentNode = listPanel()): string {
  const m = (scope.textContent ?? '').match(/หน้า\s*\d+/);
  return m ? m[0].replace(/\s+/g, ' ') : '';
}

function lastList(): Read<ListAnswer> {
  const r = lists[lists.length - 1];
  expect(r, 'an admin_customers read').toBeDefined();
  return r!;
}

function sent<A>(r: Read<A>, key: string): string {
  return r.params.get(key) ?? '';
}

beforeEach(() => {
  lists = [];
  vi.stubGlobal('fetch', vi.fn(serve));
  setLang('th');
});

afterEach(() => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

describe('Admin Customers pagination in-flight (#2611)', () => {
  it('two clicks on Next before the answer: page label 2 and one admin_customers read with before = page 1 last id', async () => {
    mount();
    await until(() => lists.length === 1, 'initial page 1 read');
    lastList().respond({ rows: customers(1, 50) });
    await until(() => bodyRows().length === 50, 'page 1 rows drawn');
    expect(pageText()).toBe('หน้า 1');
    expect(button(T.prev).disabled).toBe(true);
    expect(button(T.next).disabled).toBe(false);

    button(T.next).click();
    await until(() => lists.length === 2, 'page 2 read started');
    expect(button(T.next).disabled).toBe(true);
    expect(button(T.prev).disabled).toBe(true);

    // Second click while loading
    button(T.next).click();
    await settle();

    // Verify still only one page 2 read dispatched and page label is 2
    expect(lists.length).toBe(2);
    expect(pageText()).toBe('หน้า 2');

    // Server answers page 2
    lastList().respond({ rows: customers(51, 50) });
    await until(() => shownIds()[0] === cx(51), 'page 2 rows drawn');
    expect(pageText()).toBe('หน้า 2');
    expect(sent(lists[1], 'before')).toBe(cx(50));
    expect(lists.length).toBe(2);
    expect(button(T.next).disabled).toBe(false);
    expect(button(T.prev).disabled).toBe(false);
  });

  it('next, then prev, after the answers: page 1 again, no before', async () => {
    mount();
    await until(() => lists.length === 1, 'initial page 1 read');
    lastList().respond({ rows: customers(1, 50) });
    await until(() => bodyRows().length === 50, 'page 1 rows drawn');

    // Click Next twice before answer arrives
    button(T.next).click();
    await until(() => lists.length === 2, 'page 2 read started');
    button(T.next).click();
    await settle();

    // Answer any in-flight reads with page 2
    for (const r of lists.slice(1)) {
      r.respond({ rows: customers(51, 50) });
    }
    await until(() => shownIds()[0] === cx(51), 'page 2 rows drawn');

    // Click Prev after answers arrive
    button(T.prev).click();
    await until(() => lists.length > 2, 'prev read started');
    const prevRead = lastList();
    expect(sent(prevRead, 'before')).toBe('');

    prevRead.respond({ rows: customers(1, 50) });
    await until(() => shownIds()[0] === cx(1), 'page 1 rows drawn');
    expect(pageText()).toBe('หน้า 1');
    expect(sent(lastList(), 'before')).toBe('');
  });

  it('a failed page read leaves the buttons usable again', async () => {
    mount();
    await until(() => lists.length === 1, 'initial page 1 read');
    lastList().respond({ rows: customers(1, 50) });
    await until(() => bodyRows().length === 50, 'page 1 rows drawn');

    button(T.next).click();
    await until(() => lists.length === 2, 'page 2 read started');
    lastList().respond({
      status: 403,
      code: 'roleNotAllowedHere',
      message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้',
    });
    await settle();

    expect(button(T.prev).disabled).toBe(false);
    button(T.prev).click();
    await until(() => lists.length === 3, 'retry/prev read started');
    expect(sent(lastList(), 'before')).toBe('');
  });
});
