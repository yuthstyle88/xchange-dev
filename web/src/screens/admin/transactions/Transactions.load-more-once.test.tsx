import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { toast, Toast } from '../parts/toast';
import { QUEUE_CARD_VIEW, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { meOf } from '../../../test-support/me';

const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as { permissions: string[] };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

const loaded = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = loaded['./menu.ts'];
  expect(m, 'web/src/screens/admin/transactions/menu.ts exists').toBeDefined();
  return m!.menu;
}

const VIEW = 'admin_transactions';

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
    settlement: '3209.99',
    method: 'Cash',
    customer: `Customer ${n}`,
    customerId: `CUS-${n}`,
    staff: 'front',
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

function txs(from: number, to: number): TransactionRow[] {
  const out: TransactionRow[] = [];
  for (let n = from; n >= to; n--) out.push(tx(n));
  return out;
}

interface Seen {
  path: string;
  view: string;
  params: URLSearchParams;
  method: string;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function reply(status: number, body: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } }),
  );
}

class FakeServer {
  requests: Seen[] = [];
  version = 41;
  first: Record<string, TransactionRow[]> = { B1: [], B2: [] };
  older: Record<string, TransactionRow[]> = {};
  olderHandler?: (before: string) => Promise<Response> | Response;
  branches = [
    { id: 'B1', name: 'Silom #2' },
    { id: 'B2', name: 'Ari' },
  ];

  reads(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === VIEW).map((r) => r.params);
  }

  olderReads(): URLSearchParams[] {
    return this.requests
      .filter((r) => r.view === VIEW && r.params.get('before') !== null)
      .map((r) => r.params);
  }

  answer(url: string, init?: RequestInit): Promise<Response> {
    const u = new URL(url, 'http://desk.test');
    const view = u.pathname.startsWith('/api/v3/views/') ? u.pathname.slice('/api/v3/views/'.length) : '';
    this.requests.push({ path: u.pathname, view, params: u.searchParams, method: (init?.method ?? 'GET').toUpperCase() });
    const ok = (body: unknown) => reply(200, body);
    switch (view) {
      case VIEW: {
        const before = u.searchParams.get('before');
        if (before !== null) {
          if (this.olderHandler) return Promise.resolve(this.olderHandler(before));
          return ok({ version: this.version, changed: true, data: this.older[before] ?? [] });
        }
        if (u.searchParams.get('since') === String(this.version)) return ok({ version: this.version, changed: false });
        const branch = u.searchParams.get('branch') ?? '';
        return ok({ version: this.version, changed: true, data: this.first[branch] ?? [] });
      }
      case 'admin_branches':
        return ok({ version: this.version, changed: true, data: { branches: this.branches } });
      case 'transaction_documents':
        return ok({ version: this.version, changed: true, data: { ref: u.searchParams.get('ref'), files: [] } });
      case 'admin_shift_state':
        return ok({ version: this.version, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B1' } });
      case 'me':
        return ok({ version: 1, changed: true, data: EXPORTER_ME });
      case QUEUE_CARD_VIEW:
        return ok(queueCardUnchanged(this.version));
      default:
        return reply(404, { error: { code: 'notFound', message: 'not found' } });
    }
  }
}

let server: FakeServer;
let root: HTMLElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  server = new FakeServer();
  vi.stubGlobal(
    'fetch',
    vi.fn((input: unknown, init?: RequestInit) => server.answer(String(input), init)),
  );
  location.hash = '#transactions';
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  closeDialog();
  toast('');
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.restoreAllMocks();
  location.hash = '';
  setLang('th');
});

function mount(opts: { branch?: string; from?: string; to?: string; shown?: boolean } = {}): HTMLElement {
  const C = theMenu().Component;
  const [branch] = createSignal(opts.branch ?? 'B1');
  const [period] = createSignal({ from: opts.from ?? '2026-10-01', to: opts.to ?? '2026-10-08' });
  const [shown] = createSignal(opts.shown ?? true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () => (
      <>
        <C branch={branch} period={period} shown={shown} />
        <Dialog />
        <Toast />
      </>
    ),
    root,
  );
  return root;
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 10));
}

function shownIds(): string[] {
  return [...root.querySelectorAll('tbody tr')].map((tr) => tr.querySelector('button')?.textContent ?? '');
}

function button(text: string): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === text);
  expect(found, `one button "${text}"`).toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

describe('Transactions load-more single-flight guard (#2606)', () => {
  it('row 1: Two clicks before the first page answers -> Exactly one admin_transactions page request; each row appears once', async () => {
    server.first.B1 = txs(250, 151);
    const gate = deferred<Response>();
    server.olderHandler = () => gate.promise;

    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    const loadMoreBtn = button('โหลดเพิ่ม');
    expect(loadMoreBtn.disabled).toBe(false);

    // Double-click "โหลดเพิ่ม" before the request answers
    loadMoreBtn.click();
    loadMoreBtn.click();

    // Exactly one page request is sent and the button is disabled while in flight
    expect(server.olderReads()).toHaveLength(1);
    expect(loadMoreBtn.disabled).toBe(true);

    // Resolve the in-flight request
    gate.resolve(reply(200, { version: server.version, changed: true, data: txs(150, 51) }));
    await settle();

    // Rows must not be duplicated: each row appears once
    expect(shownIds()).toHaveLength(200);
    expect(shownIds()).toEqual(txs(250, 51).map((r) => r.id));
    expect(loadMoreBtn.disabled).toBe(false);
  });

  it('row 2: A click after the page answered -> A second request with the new last id (paging still works)', async () => {
    server.first.B1 = txs(250, 151);
    const firstGate = deferred<Response>();
    server.olderHandler = (before) => {
      if (before === tx(151).id) return firstGate.promise;
      return reply(200, { version: server.version, changed: true, data: txs(50, 21) });
    };

    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    const loadMoreBtn = button('โหลดเพิ่ม');
    loadMoreBtn.click();

    expect(loadMoreBtn.disabled).toBe(true);
    expect(server.olderReads()).toHaveLength(1);

    // Click again while first page is in flight (ignored)
    loadMoreBtn.click();
    expect(server.olderReads()).toHaveLength(1);

    // First page answers
    firstGate.resolve(reply(200, { version: server.version, changed: true, data: txs(150, 51) }));
    await settle();

    expect(shownIds()).toHaveLength(200);
    expect(loadMoreBtn.disabled).toBe(false);

    // Click after the page answered: triggers a second request with the new last id
    loadMoreBtn.click();
    await settle();

    expect(server.olderReads()).toHaveLength(2);
    expect(server.olderReads()[1]!.get('before')).toBe(tx(51).id);
    expect(shownIds()).toHaveLength(230);
    expect(shownIds()).toEqual(txs(250, 21).map((r) => r.id));
  });

  it('edge case: A failed load clears loadingMore so the button works again', async () => {
    server.first.B1 = txs(250, 151);
    const failGate = deferred<Response>();
    server.olderHandler = () => failGate.promise;

    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    const loadMoreBtn = button('โหลดเพิ่ม');
    loadMoreBtn.click();

    expect(loadMoreBtn.disabled).toBe(true);

    // Request fails
    failGate.resolve(reply(500, { error: { code: 'serverError', message: 'db error' } }));
    await settle();

    // Button is re-enabled so user can retry
    expect(loadMoreBtn.disabled).toBe(false);

    // Retry succeeds
    server.olderHandler = undefined;
    server.older[tx(151).id] = txs(150, 51);
    loadMoreBtn.click();
    await settle();

    expect(shownIds()).toHaveLength(200);
  });
});
