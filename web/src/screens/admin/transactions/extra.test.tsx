// Tests for #464 rework:
// (4) Decision 2: on branch switch filters stay, on period change data is emptied.
// (5) "Load more": fetch-id guard drops late page from previous branch after branch switch.
// (6) hasMore boundary: exactly limit (100) rows vs fewer (99).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { makeTransactionRow, type TransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { toast, Toast } from '../parts/toast';
import { QUEUE_CARD_VIEW, queueCardUnchanged } from '../../../shared/testing/page-wide';

const loaded = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = loaded['./menu.ts'];
  expect(m, 'web/src/screens/admin/transactions/menu.ts exists').toBeDefined();
  return m!.menu;
}

const VIEW = 'admin_transactions';

function tx(n: number, over: Partial<TransactionRow> = {}) {
  return makeTransactionRow({
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
  });
}

function txs(from: number, to: number) {
  const out = [];
  for (let n = from; n >= to; n--) out.push(tx(n));
  return out;
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
  branches = [
    { id: 'B1', name: 'Silom #2' },
    { id: 'B2', name: 'Ari' },
  ];
  held: { branch: string; release: () => void; gate: Promise<void> } | undefined;

  hold(branch: string): void {
    let release = () => {};
    const gate = new Promise<void>((r) => (release = r));
    this.held = { branch, release, gate };
  }

  reads(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === VIEW).map((r) => r.params);
  }

  answer(url: string, init?: RequestInit): Promise<Response> {
    const u = new URL(url, 'http://desk.test');
    const view = u.pathname.startsWith('/api/v3/views/') ? u.pathname.slice('/api/v3/views/'.length) : '';
    this.requests.push({ path: u.pathname, view, params: u.searchParams, method: (init?.method ?? 'GET').toUpperCase() });
    const ok = (body: unknown) => reply(200, body);
    switch (view) {
      case VIEW: {
        if (this.refusal) return reply(this.refusal.status, this.refusal.body);
        const before = u.searchParams.get('before');
        if (before !== null) return ok({ version: this.version, changed: true, data: this.older[before] ?? [] });
        if (u.searchParams.get('since') === String(this.version)) return ok({ version: this.version, changed: false });
        const branch = u.searchParams.get('branch') ?? '';
        const body = { version: this.version, changed: true, data: this.first[branch] ?? [] };
        if (this.held?.branch === branch) {
          return this.held.gate.then(() =>
            reply(200, { version: this.version, changed: true, data: this.first[branch] ?? [] }),
          );
        }
        return ok(body);
      }
      case 'admin_branches':
        return ok({ version: this.version, changed: true, data: { branches: this.branches } });
      case 'transaction_documents':
        return ok({ version: this.version, changed: true, data: { ref: u.searchParams.get('ref'), files: [] } });
      case 'admin_shift_state':
        return ok({ version: this.version, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B1' } });
      case 'me':
        return ok({ version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: null } });
      case QUEUE_CARD_VIEW:
        return ok(queueCardUnchanged(this.version));
      default:
        return reply(404, { error: { code: 'notFound', message: 'not found' } });
    }
  }
}

function reply(status: number, body: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } }),
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

function mount(opts: { branch?: string; from?: string; to?: string; shown?: boolean } = {}): HTMLElement {
  const C = theMenu().Component;
  const [branch, sb] = createSignal(opts.branch ?? 'B1');
  const [period, sp] = createSignal({ from: opts.from ?? '2026-10-01', to: opts.to ?? '2026-10-08' });
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

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 10));
}

function shownIds(): string[] {
  return [...root.querySelectorAll('tbody tr')].map((tr) => tr.querySelector('button')?.textContent ?? '');
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

function typeInto(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function button(text: string): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === text);
  expect(found, `one button "${text}"`).toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

function hasButton(text: string): boolean {
  return [...root.querySelectorAll('button')].some((b) => (b.textContent ?? '').trim() === text);
}

async function firstRead(): Promise<URLSearchParams> {
  await vi.waitFor(() => expect(server.reads().length).toBeGreaterThan(0));
  return server.reads()[0]!;
}

describe('V2-transactions rework tests (#464)', () => {
  it('Decision 2: on a later branch switch the search/type/status inputs stay', async () => {
    server.first.B1 = [tx(1)];
    server.first.B2 = [tx(2)];
    mount({ branch: 'B1' });
    await firstRead();

    const searchInput = control<HTMLInputElement>('ค้นหารายการ');
    const typeSelect = control<HTMLSelectElement>('ประเภทรายการ');
    const statusSelect = control<HTMLSelectElement>('สถานะ');

    typeInto(searchInput, 'SOMCHAI');
    choose(typeSelect, 'BUY');
    choose(statusSelect, 'COMPLETED');

    setBranch('B2');
    await vi.waitFor(() =>
      expect(server.reads().some((p) => p.get('branch') === 'B2')).toBe(true),
    );

    expect(searchInput.value).toBe('SOMCHAI');
    expect(typeSelect.value).toBe('BUY');
    expect(statusSelect.value).toBe('COMPLETED');
  });

  it('Decision 2: on a period change the data is emptied while the new period loads', async () => {
    server.first.B1 = [tx(10), tx(9)];
    mount({ branch: 'B1', from: '2026-10-01', to: '2026-10-08' });
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(10).id, tx(9).id]));

    // Hold subsequent B1 read
    server.hold('B1');
    server.first.B1 = [tx(20)];
    setPeriod({ from: '2026-10-09', to: '2026-10-15' });

    // Wait for the new request to be fired
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    await settle();

    // While held, rows must be emptied
    expect(shownIds(), 'data is emptied on a period change while waiting for new rows').toEqual([]);

    server.held?.release();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(20).id]));
  });

  it('Load more: drops late page from previous branch after branch switch', async () => {
    server.first.B1 = txs(100, 1);
    server.first.B2 = [tx(999)];

    let releaseOlder = () => {};
    const olderGate = new Promise<void>((r) => (releaseOlder = r));

    const originalAnswer = server.answer.bind(server);
    server.answer = (url: string, init?: RequestInit) => {
      const u = new URL(url, 'http://desk.test');
      if (u.searchParams.get('before') === tx(1).id) {
        return olderGate.then(() =>
          reply(200, {
            version: server.version,
            changed: true,
            data: [tx(200, { id: 'XC-B1-LATE' })],
          }),
        );
      }
      return originalAnswer(url, init);
    };

    mount({ branch: 'B1' });
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));
    expect(hasButton('โหลดเพิ่ม')).toBe(true);

    // Click load more - this initiates the delayed fetch
    button('โหลดเพิ่ม').click();

    // Switch branch to B2 while B1 load more is pending
    setBranch('B2');
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(999).id]));

    // Now resolve the late page from B1
    releaseOlder();
    await settle();

    // The late B1 page must be dropped, showing only B2
    expect(shownIds()).toEqual([tx(999).id]);
  });

  it('hasMore boundary: exactly limit (100) rows shows load more, while fewer (99) does not', async () => {
    // 99 rows: fewer than 100 limit -> no "โหลดเพิ่ม"
    server.first.B1 = txs(99, 1);
    mount({ branch: 'B1' });
    await vi.waitFor(() => expect(shownIds()).toHaveLength(99));
    expect(hasButton('โหลดเพิ่ม'), '99 rows must not show load more').toBe(false);

    // 100 rows: exactly 100 limit -> shows "โหลดเพิ่ม"
    server.first.B2 = txs(100, 1);
    setBranch('B2');
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));
    expect(hasButton('โหลดเพิ่ม'), '100 rows must show load more').toBe(true);
  });
});
