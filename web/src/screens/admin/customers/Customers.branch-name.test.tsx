// #2724: Admin Customers — after a branch switch the summary must reset branchName to the new branch id
// and ignore late `admin_branches` answers from an earlier branch.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { menu } from './menu';

const T = {
  waiting: 'รอข้อมูลจากเซิร์ฟเวอร์',
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
type BranchesAnswer = { branches: { id: string; name: string }[] } | Refusal;

interface Read<A> {
  params: URLSearchParams;
  respond: (answer: A) => void;
}

let lists: Read<ListAnswer>[];
let branchReads: Read<BranchesAnswer>[];
let autoList: ((params: URLSearchParams) => ListAnswer) | null;
let autoBranches: (() => BranchesAnswer) | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function listResponse(answer: ListAnswer): Response {
  if ('rows' in answer) return json(200, { version: 3, changed: true, data: answer.rows });
  const error: Record<string, string> = { code: answer.code, message: answer.message };
  if (answer.field) error.field = answer.field;
  return json(answer.status, { error });
}

function branchesResponse(answer: BranchesAnswer): Response {
  if ('branches' in answer) {
    return json(200, { version: 3, changed: true, data: { branches: answer.branches } });
  }
  const error: Record<string, string> = { code: answer.code, message: answer.message };
  if (answer.field) error.field = answer.field;
  return json(answer.status, { error });
}

const DEFAULT_BRANCHES = [
  { id: 'MAIN', name: 'Main Branch' },
  { id: 'B9', name: 'Nine' },
];

function serve(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const view = url.pathname.startsWith('/api/v3/views/')
    ? url.pathname.replace(/^\/api\/v3\/views\//, '')
    : null;
  if (view === 'admin_customers') {
    return new Promise<Response>((resolve) => {
      const read: Read<ListAnswer> = {
        params: url.searchParams,
        respond: (a) => resolve(listResponse(a)),
      };
      lists.push(read);
      if (autoList) read.respond(autoList(url.searchParams));
    });
  }
  if (view === 'admin_branches') {
    return new Promise<Response>((resolve) => {
      const read: Read<BranchesAnswer> = {
        params: url.searchParams,
        respond: (a) => resolve(branchesResponse(a)),
      };
      branchReads.push(read);
      if (autoBranches) read.respond(autoBranches());
    });
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

interface Mounted {
  setBranch: (b: string) => void;
}

function mount(opts: { branch?: string } = {}): Mounted {
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
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
  return { setBranch };
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

function summaryText(): string {
  return (listPanel().querySelector('[role="status"]')?.textContent ?? '').trim();
}

function lastList(): Read<ListAnswer> {
  const r = lists[lists.length - 1];
  expect(r, 'an admin_customers read').toBeDefined();
  return r!;
}

beforeEach(() => {
  lists = [];
  branchReads = [];
  autoList = null;
  autoBranches = null;
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

describe('Admin Customers branchName summary on branch switch (#2724)', () => {
  it('C1: MAIN shown as "Main Branch"; switch to B9; admin_branches fails; B9 rows load -> summary shows B9, not "Main Branch"', async () => {
    autoList = () => ({ rows: customers(1, 2) });
    let call = 0;
    autoBranches = () => {
      call++;
      if (call === 1) return { branches: DEFAULT_BRANCHES };
      return { status: 500, code: 'serverError', message: 'boom' };
    };

    const m = mount({ branch: 'MAIN' });
    await until(() => bodyRows().length === 2 && summaryText().includes('Main Branch'), 'MAIN summary');
    expect(summaryText()).toContain('สาขา Main Branch');

    m.setBranch('B9');
    await until(() => lists.length === 2 && bodyRows().length === 2, 'B9 rows drawn');

    expect(summaryText()).toContain('สาขา B9');
    expect(summaryText()).not.toContain('Main Branch');
  });

  it('C2: Switch MAIN -> B9 -> MAIN with B9 admin_branches held until after MAIN has landed -> summary shows "Main Branch"', async () => {
    autoList = () => ({ rows: customers(1, 2) });

    const m = mount({ branch: 'MAIN' });
    await until(() => branchReads.length === 1 && bodyRows().length === 2, 'initial MAIN read');
    branchReads[0].respond({ branches: DEFAULT_BRANCHES });
    await until(() => summaryText().includes('Main Branch'), 'initial MAIN summary');

    // Switch to B9; hold branchReads[1]
    m.setBranch('B9');
    await until(() => branchReads.length === 2 && lists.length === 2, 'B9 reads started');

    // Switch back to MAIN; let branchReads[2] (for MAIN) land first
    m.setBranch('MAIN');
    await until(() => branchReads.length === 3 && lists.length === 3, 'second MAIN reads started');
    branchReads[2].respond({ branches: DEFAULT_BRANCHES });
    await until(() => summaryText().includes('Main Branch'), 'second MAIN summary');

    // Now the stale B9 admin_branches read resolves late
    branchReads[1].respond({ branches: DEFAULT_BRANCHES });
    await settle();

    expect(summaryText()).toContain('สาขา Main Branch');
    expect(summaryText()).not.toContain('Nine');
  });

  it('C3: Switch to B9; admin_branches answers with B9 named "Nine" -> summary shows "Nine"', async () => {
    autoList = () => ({ rows: customers(1, 2) });
    autoBranches = () => ({ branches: DEFAULT_BRANCHES });

    const m = mount({ branch: 'MAIN' });
    await until(() => summaryText().includes('Main Branch'), 'MAIN summary');

    m.setBranch('B9');
    await until(() => lists.length === 2 && summaryText().includes('Nine'), 'B9 summary');

    expect(summaryText()).toContain('สาขา Nine');
  });

  it('C4: First mount; admin_branches fails -> summary shows the id, no error', async () => {
    autoList = () => ({ rows: customers(1, 2) });
    autoBranches = () => ({ status: 500, code: 'serverError', message: 'boom' });

    mount({ branch: 'MAIN' });
    await until(() => bodyRows().length === 2 && summaryText() !== '', 'MAIN rows and summary');

    expect(summaryText()).toContain('สาขา MAIN');
    expect(listPanel().querySelector('.refusal-message')).toBeNull();
  });

  it('C5: Switch while B9 list is loading -> no summary until the rows load', async () => {
    autoBranches = () => ({ branches: DEFAULT_BRANCHES });

    const m = mount({ branch: 'MAIN' });
    await until(() => lists.length === 1, 'initial MAIN read');
    lastList().respond({ rows: customers(1, 2) });
    await until(() => bodyRows().length === 2 && summaryText().includes('Main Branch'), 'MAIN drawn');

    m.setBranch('B9');
    await until(() => lists.length === 2, 'B9 read started');

    expect(listPanel().textContent).toContain(T.waiting);
    expect(listPanel().querySelector('[role="status"]')).toBeNull();

    lastList().respond({ rows: customers(1, 2) });
    await until(() => bodyRows().length === 2 && summaryText().includes('Nine'), 'B9 rows and summary');
    expect(summaryText()).toContain('สาขา Nine');
  });
});
