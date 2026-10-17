// #2593: Admin customers — keep "Load older" button visible when an older page read fails in history dialog.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { menu } from './menu';

const T = {
  showHistory: 'ดูประวัติ',
  loadOlder: 'โหลดรายการเก่ากว่า',
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
let autoHistory: ((params: URLSearchParams) => HistoryAnswer | Promise<HistoryAnswer>) | null;

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
      if (autoHistory) {
        Promise.resolve(autoHistory(url.searchParams)).then((a) => read.respond(a));
      }
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

function mount(): void {
  const [branch] = createSignal('MAIN');
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

function buttonIn(scope: ParentNode, label: string): HTMLButtonElement | undefined {
  return [...scope.querySelectorAll('button')].find(
    (x) => (x.textContent ?? '').trim() === label,
  );
}

function dialog(): HTMLDialogElement {
  const d = root.querySelector('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

function dialogOpen(): boolean {
  return root.querySelector('dialog')?.hasAttribute('open') ?? false;
}

async function openHistory(rowIndex = 0): Promise<void> {
  const tr = bodyRows()[rowIndex];
  expect(tr, `row ${rowIndex}`).toBeDefined();
  buttonIn(tr!, T.showHistory)?.click();
  await until(() => dialogOpen() && histories.length > 0, 'the history dialog and its read');
}

describe('Admin Customers: history dialog "Load older" error handling (#2593)', () => {
  beforeEach(() => {
    setLang('th');
    lists = [];
    histories = [];
    autoList = null;
    autoHistory = null;
    vi.stubGlobal('fetch', vi.fn(serve));
  });

  afterEach(async () => {
    closeDialog();
    await settle();
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  // Row 1: First page OK (full page), older page fails -> Rows kept, error shown, Load older still visible
  it('row 1: first page OK (full page), older page fails -> rows kept, error shown, Load older still visible', async () => {
    const page1 = Array.from({ length: 50 }, (_, i) => tx(i + 1));
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = (p) => {
      const before = p.get('before');
      if (before) {
        return { status: 500, code: 'serverError', message: 'internal error' };
      }
      return { rows: page1 };
    };

    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 50, 'page 1 of 50 history rows');

    const loadOlderBtn = buttonIn(dialog(), T.loadOlder);
    expect(loadOlderBtn, 'load-older button exists on full page').toBeDefined();

    // Click Load older -> fails
    loadOlderBtn!.click();
    await until(() => dialog().querySelector('.banner.error') !== null, 'error banner shown');

    // Rows are kept (still 50)
    expect(bodyRows(dialog()).length).toBe(50);
    // Error is shown
    expect(dialog().querySelector('.banner.error')).not.toBeNull();
    // Load older button is STILL visible
    const loadOlderBtnAfterFail = buttonIn(dialog(), T.loadOlder);
    expect(loadOlderBtnAfterFail, 'load-older button is still visible after failure').toBeDefined();
  });

  // Row 2: Then Load older succeeds -> Older rows appended
  it('row 2: then Load older succeeds -> older rows appended and error cleared', async () => {
    const page1 = Array.from({ length: 50 }, (_, i) => tx(i + 1));
    const page2 = [tx(51), tx(52)];
    let olderAttempts = 0;

    autoList = () => ({ rows: [customer(1)] });
    autoHistory = (p) => {
      const before = p.get('before');
      if (before) {
        olderAttempts += 1;
        if (olderAttempts === 1) {
          return { status: 500, code: 'serverError', message: 'internal error' };
        }
        return { rows: page2 };
      }
      return { rows: page1 };
    };

    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 50, 'page 1 of 50 history rows');

    // First older attempt fails
    const loadOlderBtn = buttonIn(dialog(), T.loadOlder)!;
    expect(loadOlderBtn).toBeDefined();
    loadOlderBtn.click();
    await until(() => dialog().querySelector('.banner.error') !== null, 'error banner shown');

    // Second older attempt succeeds
    const retryBtn = buttonIn(dialog(), T.loadOlder)!;
    expect(retryBtn, 'retry button exists').toBeDefined();
    retryBtn.click();
    await until(() => bodyRows(dialog()).length === 52, 'older rows appended');

    // Error banner is cleared
    expect(dialog().querySelector('.banner.error')).toBeNull();
  });

  // Row 3: First page fails -> Rows empty, no Load older (unchanged)
  it('row 3: first page fails -> rows empty, no Load older', async () => {
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = () => ({ status: 500, code: 'serverError', message: 'server error' });

    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => dialog().querySelector('.banner.error') !== null, 'first page error banner');

    expect(bodyRows(dialog()).length).toBe(0);
    expect(buttonIn(dialog(), T.loadOlder)).toBeUndefined();
  });

  // Edge case (§9): A stale answer (seq changed) still does nothing.
  it('edge case (§9): a stale answer (seq changed) still does nothing', async () => {
    const page1 = Array.from({ length: 50 }, (_, i) => tx(i + 1));
    const searchResults = [tx(101)];
    let delayedOlderRespond: ((a: HistoryAnswer) => void) | undefined;

    autoList = () => ({ rows: [customer(1)] });
    autoHistory = (p) => {
      const q = p.get('q');
      const before = p.get('before');
      if (before) {
        return new Promise<HistoryAnswer>((resolve) => {
          delayedOlderRespond = resolve;
        });
      }
      if (q === 'search-term') {
        return { rows: searchResults };
      }
      return { rows: page1 };
    };

    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 50, 'page 1 loaded');

    // Trigger loadOlder (seq 2, in-flight)
    const loadOlderBtn = buttonIn(dialog(), T.loadOlder)!;
    loadOlderBtn.click();
    await settle();

    // Now trigger search in history dialog (seq 3)
    const searchBox = dialog().querySelector<HTMLInputElement>('input[type="search"]')!;
    expect(searchBox).not.toBeNull();
    searchBox.value = 'search-term';
    searchBox.dispatchEvent(new Event('input', { bubbles: true }));
    await until(() => bodyRows(dialog()).length === 1, 'search results shown');

    // Now reject the stale older page request from seq 2
    delayedOlderRespond?.({ status: 500, code: 'serverError', message: 'delayed fail' });
    await settle();

    // The stale rejection must not set an error banner over seq 3's search results
    expect(bodyRows(dialog()).length).toBe(1);
    expect(dialog().querySelector('.banner.error')).toBeNull();
  });
});
