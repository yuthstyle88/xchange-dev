// #2177: Admin Transactions recovery after a failed refresh (§8 behaviour table, §9 edge cases).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import { toast, Toast } from '../parts/toast';
import { QUEUE_CARD_VIEW, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { meOf } from '../../../test-support/me';

const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as {
    permissions: string[];
  };
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

interface Seen {
  path: string;
  view: string;
  params: URLSearchParams;
  method: string;
}

class FakeServer {
  requests: Seen[] = [];
  version = 41;
  first: Record<string, TransactionRow[]> = { B1: [], B2: [] };
  older: Record<string, TransactionRow[]> = {};
  refusal: { status: number; body: unknown } | undefined;
  nextHandler: (() => Promise<Response>) | undefined;
  branches = [
    { id: 'B1', name: 'Silom #2' },
    { id: 'B2', name: 'Ari' },
  ];

  reads(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === VIEW).map((r) => r.params);
  }

  answer(url: string, init?: RequestInit): Promise<Response> {
    const u = new URL(url, 'http://desk.test');
    const view = u.pathname.startsWith('/api/v3/views/')
      ? u.pathname.slice('/api/v3/views/'.length)
      : '';
    this.requests.push({
      path: u.pathname,
      view,
      params: u.searchParams,
      method: (init?.method ?? 'GET').toUpperCase(),
    });
    const ok = (body: unknown) => reply(200, body);
    switch (view) {
      case VIEW: {
        if (this.nextHandler) {
          const h = this.nextHandler;
          this.nextHandler = undefined;
          return h();
        }
        if (this.refusal) return reply(this.refusal.status, this.refusal.body);
        const before = u.searchParams.get('before');
        if (before !== null) {
          return ok({ version: this.version, changed: true, data: this.older[before] ?? [] });
        }
        if (u.searchParams.get('since') === String(this.version)) {
          return ok({ version: this.version, changed: false });
        }
        const branch = u.searchParams.get('branch') ?? '';
        return ok({ version: this.version, changed: true, data: this.first[branch] ?? [] });
      }
      case 'admin_branches':
        return ok({ version: this.version, changed: true, data: { branches: this.branches } });
      case 'transaction_documents':
        return ok({
          version: this.version,
          changed: true,
          data: { ref: u.searchParams.get('ref'), files: [] },
        });
      case 'admin_shift_state':
        return ok({
          version: this.version,
          changed: true,
          data: { shiftOpen: true, deskUrl: '/desk?branch=B1' },
        });
      case 'me':
        return ok({ version: 1, changed: true, data: EXPORTER_ME });
      case QUEUE_CARD_VIEW:
        return ok(queueCardUnchanged(this.version));
      default:
        return reply(404, { error: { code: 'notFound', message: 'not found' } });
    }
  }
}

function reply(status: number, body: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

let server: FakeServer;
let root: HTMLElement;
let dispose: (() => void) | undefined;
let setBranch: Setter<string>;
let setPeriod: Setter<{ from: string; to: string }>;

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

function mount(
  opts: { branch?: string; from?: string; to?: string; shown?: boolean } = {},
): HTMLElement {
  const C = theMenu().Component;
  const [branch, sb] = createSignal(opts.branch ?? 'B1');
  const [period, sp] = createSignal({
    from: opts.from ?? '2026-10-01',
    to: opts.to ?? '2026-10-08',
  });
  const [shown] = createSignal(opts.shown ?? true);
  setBranch = sb;
  setPeriod = sp;
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

function shownIds(): string[] {
  return [...root.querySelectorAll('tbody tr')].map(
    (tr) => tr.querySelector('button')?.textContent ?? '',
  );
}

function labelText(label: HTMLLabelElement): string {
  const copy = label.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('input, select, textarea').forEach((c) => c.remove());
  return (copy.textContent ?? '').trim();
}

function control<T extends HTMLElement>(text: string): T {
  const labels = [...root.querySelectorAll('label')].filter((l) => labelText(l) === text);
  expect(labels.map((l) => l.textContent), `one label "${text}"`).toHaveLength(1);
  const label = labels[0]!;
  const c = (label.control ?? label.querySelector('input, select')) as T | null;
  expect(c, `the control labelled "${text}"`).toBeTruthy();
  return c!;
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

const SERVER_REFUSAL = {
  status: 500,
  body: { error: { code: 'serverError', message: 'raw server text' } },
};

describe('Admin Transactions recovery after a failed Refresh (#2177 §8, §9)', () => {
  it("row 1: Refresh fails, then the next Refresh succeeds and the server's data did not change -> branch rows are shown again, error is gone, count is not 0, and second refresh is sent without since", async () => {
    server.first.B1 = [tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(2).id, tx(1).id]));
    expect(root.textContent).toContain('พบ 2 รายการ');

    // First refresh fails: rows empty and error banner shows
    server.refusal = SERVER_REFUSAL;
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(server.reads()[1]!.get('since')).toBe('41');
    await vi.waitFor(() => expect(root.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง'));
    expect(shownIds()).toEqual([]);
    expect(root.textContent).toContain('พบ 0 รายการ');

    // Next refresh succeeds while server version (41) and data did not change
    server.refusal = undefined;
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(3));
    expect(
      server.reads()[2]!.has('since'),
      'recovery refresh after a failed read is sent without since',
    ).toBe(false);
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(2).id, tx(1).id]));
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent).not.toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(root.textContent).toContain('พบ 2 รายการ');
    expect(root.textContent).not.toContain('พบ 0 รายการ');
  });

  it('row 2: Refresh fails -> error shown through wordedError', async () => {
    server.first.B1 = [tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(2).id, tx(1).id]));

    server.refusal = SERVER_REFUSAL;
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    await vi.waitFor(() =>
      expect(root.querySelector('[role="alert"]')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง'),
    );
    expect(root.textContent).not.toContain('raw server text');
    expect(shownIds()).toEqual([]);
  });

  it('row 3: Refresh after a successful read -> sends since=<held version>; changed:false keeps the rows', async () => {
    server.first.B1 = [tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(2).id, tx(1).id]));

    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(server.reads()[1]!.get('since')).toBe('41');
    expect(shownIds()).toEqual([tx(2).id, tx(1).id]);
  });

  it('row 4: First read refused -> error and no rows', async () => {
    server.refusal = SERVER_REFUSAL;
    mount();
    await vi.waitFor(() => expect(server.reads().length).toBe(1));
    await vi.waitFor(() =>
      expect(root.querySelector('[role="alert"]')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง'),
    );
    expect(root.textContent).not.toContain('raw server text');
    expect(shownIds()).toEqual([]);
  });

  it('row 5: Filter, branch or period change -> read without since', async () => {
    server.first.B1 = [tx(1)];
    server.first.B2 = [tx(2)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(1).id]));

    choose(control<HTMLSelectElement>('ประเภทรายการ'), 'SELL');
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(server.reads()[1]!.get('type')).toBe('SELL');
    expect(server.reads()[1]!.has('since')).toBe(false);

    setPeriod({ from: '2026-09-01', to: '2026-09-30' });
    await vi.waitFor(() => expect(server.reads().length).toBe(3));
    expect(server.reads()[2]!.get('from')).toBe('2026-09-01');
    expect(server.reads()[2]!.has('since')).toBe(false);

    setBranch('B2');
    await vi.waitFor(() => expect(server.reads().length).toBe(4));
    expect(server.reads()[3]!.get('branch')).toBe('B2');
    expect(server.reads()[3]!.has('since')).toBe(false);
  });

  it('edge case (§9): two refreshes in flight -> stale failure does not reset the version of a newer read', async () => {
    server.first.B1 = [tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(2).id, tx(1).id]));

    let rejectFirstRefresh: () => void = () => {};
    server.nextHandler = () =>
      new Promise<Response>((resolve) => {
        rejectFirstRefresh = () =>
          resolve(
            new Response(JSON.stringify(SERVER_REFUSAL.body), {
              status: SERVER_REFUSAL.status,
              headers: { 'content-type': 'application/json' },
            }),
          );
      });

    // First refresh starts and hangs in flight
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(2));

    // Second refresh starts while first is in flight and succeeds with version 42
    server.version = 42;
    server.first.B1 = [tx(3), tx(2), tx(1)];
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(3));
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(3).id, tx(2).id, tx(1).id]));

    // Now the stale first refresh fails; fetchId !== currentFetchId must ignore it
    rejectFirstRefresh();
    await new Promise((r) => setTimeout(r, 20));
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(shownIds()).toEqual([tx(3).id, tx(2).id, tx(1).id]);

    // Subsequent refresh still uses version 42 from the newer successful read
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(4));
    expect(server.reads()[3]!.get('since')).toBe('42');
  });
});
