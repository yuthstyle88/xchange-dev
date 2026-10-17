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

function txs(from: number, to: number, branchPrefix = ''): TransactionRow[] {
  const out: TransactionRow[] = [];
  for (let n = from; n >= to; n--) {
    out.push(
      tx(n, branchPrefix ? { id: `XC-${branchPrefix}-${String(n).padStart(6, '0')}` } : {}),
    );
  }
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
  olderHandler?: (params: URLSearchParams) => Promise<Response> | Response;
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
          if (this.olderHandler) return Promise.resolve(this.olderHandler(u.searchParams));
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

interface MountResult {
  el: HTMLElement;
  setBranch: (b: string) => void;
  setPeriod: (p: { from: string; to: string }) => void;
}

function mount(opts: { branch?: string; from?: string; to?: string; shown?: boolean } = {}): MountResult {
  const C = theMenu().Component;
  const [branch, setBranch] = createSignal(opts.branch ?? 'B1');
  const [period, setPeriod] = createSignal({ from: opts.from ?? '2026-10-01', to: opts.to ?? '2026-10-08' });
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
  return { el: root, setBranch, setPeriod };
}

function shownIds(): string[] {
  return [...root.querySelectorAll('tbody tr')].map((tr) => tr.querySelector('button')?.textContent ?? '');
}

function button(text: string): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === text);
  expect(found, `one button "${text}"`).toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

describe('#2642 Transactions load-more across branch/period switch', () => {
  it('Load more in flight, switch branch, new first page answers, old page still pending -> New branch Load more is enabled', async () => {
    server.first.B1 = txs(250, 151, 'B1');
    server.first.B2 = txs(350, 251, 'B2');
    const b1Gate = deferred<Response>();

    server.olderHandler = (params) => {
      if (params.get('branch') === 'B1') {
        return b1Gate.promise;
      }
      return reply(200, { version: server.version, changed: true, data: [] });
    };

    const m = mount({ branch: 'B1' });
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    const loadMoreB1 = button('โหลดเพิ่ม');
    expect(loadMoreB1.disabled).toBe(false);

    // Click Load more on B1 (starts in-flight request)
    loadMoreB1.click();
    expect(loadMoreB1.disabled, 'B1 Load more disabled while in flight').toBe(true);

    // Switch branch to B2 while B1 page is still pending
    m.setBranch('B2');
    await vi.waitFor(() => {
      const ids = shownIds();
      expect(ids).toHaveLength(100);
      expect(ids[0]).toContain('XC-B2-');
    });

    // The new branch's Load more button must be enabled even though B1 is still pending
    const loadMoreB2 = button('โหลดเพิ่ม');
    expect(loadMoreB2.disabled, 'B2 Load more must be enabled').toBe(false);
  });

  it('Then click Load more on the new branch -> One page request for the new branch with its own last id', async () => {
    server.first.B1 = txs(250, 151, 'B1');
    server.first.B2 = txs(350, 251, 'B2');
    const b1Gate = deferred<Response>();

    server.olderHandler = (params) => {
      if (params.get('branch') === 'B1') {
        return b1Gate.promise;
      }
      return reply(200, { version: server.version, changed: true, data: txs(250, 241, 'B2') });
    };

    const m = mount({ branch: 'B1' });
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    // Start load-more on B1
    button('โหลดเพิ่ม').click();

    // Switch branch to B2
    m.setBranch('B2');
    await vi.waitFor(() => {
      expect(shownIds()[0]).toContain('XC-B2-');
    });

    const b2LastId = shownIds()[99]!;
    expect(b2LastId).toContain('XC-B2-000251');

    // Click Load more on B2
    const loadMoreB2 = button('โหลดเพิ่ม');
    loadMoreB2.click();

    // Verify exactly one page request for B2 with its own last id
    const b2OlderReads = server.olderReads().filter((r) => r.get('branch') === 'B2');
    expect(b2OlderReads, 'one page request for B2').toHaveLength(1);
    expect(b2OlderReads[0]!.get('before')).toBe(b2LastId);
  });

  it('Old branch page answers late -> Dropped; rows of new branch unchanged; new branch in-flight guard not cleared', async () => {
    server.first.B1 = txs(250, 151, 'B1');
    server.first.B2 = txs(350, 251, 'B2');
    const b1Gate = deferred<Response>();
    const b2Gate = deferred<Response>();

    server.olderHandler = (params) => {
      if (params.get('branch') === 'B1') {
        return b1Gate.promise;
      }
      if (params.get('branch') === 'B2') {
        return b2Gate.promise;
      }
      return reply(200, { version: server.version, changed: true, data: [] });
    };

    const m = mount({ branch: 'B1' });
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    // Start load-more on B1
    button('โหลดเพิ่ม').click();

    // Switch branch to B2
    m.setBranch('B2');
    await vi.waitFor(() => {
      expect(shownIds()[0]).toContain('XC-B2-');
    });

    // Start load-more on B2
    const loadMoreB2 = button('โหลดเพิ่ม');
    loadMoreB2.click();
    expect(loadMoreB2.disabled, 'B2 load more is in flight').toBe(true);

    // Old branch B1 answers late with rows
    b1Gate.resolve(
      new Response(
        JSON.stringify({ version: server.version, changed: true, data: txs(150, 141, 'B1') }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    await new Promise((r) => setTimeout(r, 50));

    // B1 rows are dropped, rows remain only B2 rows
    const currentIds = shownIds();
    expect(currentIds).toHaveLength(100);
    expect(currentIds.every((id) => id.includes('XC-B2-'))).toBe(true);

    // B2 in-flight guard must NOT have been cleared by B1's stale answer
    expect(loadMoreB2.disabled, 'B2 in-flight guard remains active').toBe(true);
  });

  it('period change behaves like a branch switch (edge case §9)', async () => {
    server.first.B1 = txs(250, 151, 'P1');
    const p1Gate = deferred<Response>();

    server.olderHandler = () => p1Gate.promise;

    const m = mount({ branch: 'B1', from: '2026-10-01', to: '2026-10-08' });
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));

    const loadMoreBtn = button('โหลดเพิ่ม');
    loadMoreBtn.click();
    expect(loadMoreBtn.disabled).toBe(true);

    // Update server data for new period
    server.first.B1 = txs(350, 251, 'P2');

    // Switch period while load more is in flight
    m.setPeriod({ from: '2026-10-09', to: '2026-10-16' });
    await vi.waitFor(() => {
      const ids = shownIds();
      expect(ids).toHaveLength(100);
      expect(ids[0]).toContain('XC-P2-');
    });

    // New period's Load more must be enabled
    const loadMoreP2 = button('โหลดเพิ่ม');
    expect(loadMoreP2.disabled, 'Load more is enabled on period switch').toBe(false);
  });
});
