import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import { menu } from './menu';

const T = {
  waiting: 'รอข้อมูลจากเซิร์ฟเวอร์',
  showHistory: 'ดูประวัติ',
  emptyBranch: 'ยังไม่มีข้อมูลลูกค้าในสาขานี้',
  historyEmpty: 'ไม่มีรายการที่ตรงกับคำค้น หรือยังไม่มีประวัติแลกเงิน',
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

function tx(n: number, over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: `XC-20261008-${String(n).padStart(6, '0')}`,
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    unitSize: '1',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3210.00',
    method: 'Cash',
    customer: 'Customer 1',
    customerId: cx(1),
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
}

type Refusal = { status: number; code: string; message: string; field?: string };
type ListAnswer = { rows: CustomerRow[] } | Refusal;
type HistoryAnswer = { rows: TransactionRow[] } | Refusal;

interface Read<A> {
  params: URLSearchParams;
  respond: (answer: A) => void;
}

let lists: Read<ListAnswer>[];
let histories: Read<HistoryAnswer>[];
let autoList: ((params: URLSearchParams) => ListAnswer) | null;
let autoHistory: ((params: URLSearchParams) => HistoryAnswer) | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewResponse(answer: ListAnswer | HistoryAnswer): Response {
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
      if (autoList) read.respond(autoList(url.searchParams));
    });
  }
  if (view === 'admin_customer_history') {
    return new Promise<Response>((resolve) => {
      const read: Read<HistoryAnswer> = {
        params: url.searchParams,
        respond: (a) => resolve(viewResponse(a)),
      };
      histories.push(read);
      if (autoHistory) read.respond(autoHistory(url.searchParams));
    });
  }
  if (view === 'admin_branches') {
    return Promise.resolve(
      json(200, {
        version: 3,
        changed: true,
        data: { branches: [{ id: 'MAIN', name: 'Main Branch' }, { id: 'B9', name: 'Nine' }] },
      }),
    );
  }
  if (url.pathname === '/api/v2/bookings') {
    return Promise.resolve(json(200, { items: [], hasMore: false, page: 0 }));
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

function searchInput(scope: ParentNode = listPanel()): HTMLInputElement {
  const input = scope.querySelector<HTMLInputElement>('input[type="search"]');
  expect(input, 'the search input').not.toBeNull();
  return input!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function dialog(): HTMLDialogElement {
  const d = root.querySelector('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

function dialogOpen(): boolean {
  return root.querySelector('dialog')?.hasAttribute('open') ?? false;
}

function lastList(): Read<ListAnswer> {
  const r = lists[lists.length - 1];
  expect(r, 'an admin_customers read').toBeDefined();
  return r!;
}

function lastHistory(): Read<HistoryAnswer> {
  const r = histories[histories.length - 1];
  expect(r, 'an admin_customer_history read').toBeDefined();
  return r!;
}

async function openHistory(rowIndex = 0): Promise<void> {
  const tr = bodyRows()[rowIndex];
  expect(tr, `row ${rowIndex}`).toBeDefined();
  buttonIn(tr!, T.showHistory).click();
  await until(() => dialogOpen() && histories.length > 0, 'the history dialog and its read');
}

beforeEach(() => {
  lists = [];
  histories = [];
  autoList = null;
  autoHistory = null;
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

describe('Admin Customers loading states (#2313)', () => {
  it('switches branch and shows waiting (not emptyBranch or summary) before the answer arrives', async () => {
    const m = mount();
    await until(() => lists.length === 1, 'initial MAIN read');
    lastList().respond({ rows: customers(1, 2) });
    await until(() => bodyRows().length === 2, 'MAIN rows drawn');

    m.setBranch('B9');
    await until(() => lists.length === 2, 'B9 read started');

    const panelText = listPanel().textContent ?? '';
    expect(panelText).toContain(T.waiting);
    expect(panelText).not.toContain(T.emptyBranch);
    expect(listPanel().querySelector('[role="status"]')).toBeNull();
  });

  it('opens customer history and shows waiting (not historyEmpty) before the answer arrives', async () => {
    autoList = () => ({ rows: [customer(1)] });
    mount();
    await until(() => bodyRows().length === 1, 'one customer row');

    await openHistory(0);

    const dText = dialog().textContent ?? '';
    expect(dText).toContain(T.waiting);
    expect(dText).not.toContain(T.historyEmpty);
  });

  it('shows emptyBranch only after the branch read answers with no rows (not while waiting)', async () => {
    const m = mount();
    await until(() => lists.length === 1, 'initial MAIN read');
    lastList().respond({ rows: customers(1, 2) });
    await until(() => bodyRows().length === 2, 'MAIN rows drawn');

    m.setBranch('B9');
    await until(() => lists.length === 2, 'B9 read started');
    expect(listPanel().textContent).toContain(T.waiting);
    expect(listPanel().textContent).not.toContain(T.emptyBranch);

    lastList().respond({ rows: [] });
    await until(() => (listPanel().textContent ?? '').includes(T.emptyBranch), 'emptyBranch after answer');
    expect(listPanel().textContent).toContain(T.emptyBranch);
    expect(listPanel().textContent).not.toContain(T.waiting);
  });

  it('shows historyEmpty when a history search answers with no match (and waiting while initial history loads)', async () => {
    autoList = () => ({ rows: [customer(1)] });
    mount();
    await until(() => bodyRows().length === 1, 'one customer row');

    await openHistory(0);
    expect(dialog().textContent).toContain(T.waiting);
    expect(dialog().textContent).not.toContain(T.historyEmpty);

    lastHistory().respond({ rows: [tx(1)] });
    await until(() => bodyRows(dialog()).length === 1, 'history row drawn');

    type(searchInput(dialog()), 'usd');
    await until(() => histories.length === 2, 'search read started');
    lastHistory().respond({ rows: [] });
    await until(() => (dialog().textContent ?? '').includes(T.historyEmpty), 'historyEmpty after empty search');
    expect(dialog().textContent).toContain(T.historyEmpty);
    expect(dialog().textContent).not.toContain(T.waiting);
  });

  it('shows the error banner and neither historyEmpty nor waiting when the history read is refused', async () => {
    autoList = () => ({ rows: [customer(1)] });
    mount();
    await until(() => bodyRows().length === 1, 'one customer row');

    await openHistory(0);
    expect(dialog().textContent).toContain(T.waiting);
    expect(dialog().textContent).not.toContain(T.historyEmpty);

    lastHistory().respond({
      status: 403,
      code: 'roleNotAllowedHere',
      message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้',
    });
    const expectedErr = errorText('roleNotAllowedHere', 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้');
    await until(() => (dialog().textContent ?? '').includes(expectedErr), 'history error banner');
    expect(dialog().querySelector('.banner.error')?.textContent).toContain(expectedErr);
    expect(dialog().textContent).not.toContain(T.historyEmpty);
    expect(dialog().textContent).not.toContain(T.waiting);
  });

  it('keeps the shown rows on a refresh tick without flashing to waiting, while hiding stale summary during load', async () => {
    mount();
    await until(() => lists.length === 1, 'initial MAIN read');
    lastList().respond({ rows: customers(1, 2) });
    await until(() => bodyRows().length === 2, 'MAIN rows drawn');
    expect(listPanel().querySelector('[role="status"]')).not.toBeNull();

    requestRefresh();
    await until(() => lists.length === 2, 'refresh read started');

    expect(shownIds()).toEqual([cx(1), cx(2)]);
    expect(listPanel().textContent).not.toContain(T.waiting);
    expect(listPanel().querySelector('[role="status"]')).toBeNull();

    lastList().respond({ rows: customers(1, 2) });
    await until(() => listPanel().querySelector('[role="status"]') !== null, 'summary restored after refresh');
    expect(shownIds()).toEqual([cx(1), cx(2)]);
  });
});
