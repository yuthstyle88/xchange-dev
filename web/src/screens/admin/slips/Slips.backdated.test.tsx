// F20b (docs/design/backdated-keying.md §15.4): a backdated sale carries its "ย้อนหลัง" mark on the Slips menu, beside
// its number. Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken;
// add tests in a file of your own. The harness below is copied from Slips.test.tsx (#465), trimmed to what these tests use.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { makeTransactionRow, type TransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import type { AdminMenu } from '../menus';

interface Table {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuModule = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const tableModule = import.meta.glob<Table>('./i18n.ts', { eager: true });

function menu(): AdminMenu {
  const m = menuModule['./menu.ts'];
  expect(m, 'web/src/screens/admin/slips/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

function table(): Table {
  const m = tableModule['./i18n.ts'];
  expect(m, 'web/src/screens/admin/slips/i18n.ts exists').toBeDefined();
  return m!;
}

/** Each text the menu draws is a value of its own table (W10) — never another table's. */
function own(text: string): string {
  expect(Object.values(table().th), `"${text}" is a value of slips/i18n.ts th`).toContain(text);
  return text;
}

// ── rows ─────────────────────────────────────────────────────────────────────

function row(n: number, over: Partial<TransactionRow> = {}) {
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
    settlement: '3210.00',
    method: 'Bank Transfer',
    customer: `Customer ${n}`,
    customerId: `CX-${String(n).padStart(8, '0')}`,
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: `TRX-${n}`,
    slipUrl: `/files/slip-key-${n}`,
    ...over,
  });
}

// ── a fake server behind the real ApiClient ──────────────────────────────────

interface Read {
  params: URLSearchParams;
  respond: (answer: Answer) => void;
}

type Answer =
  | { rows: TransactionRow[] }
  | { status: number; code: string; message: string; field?: string };

let reads: Read[];
let auto: ((params: URLSearchParams) => Answer) | null;
let fetchSpy: ReturnType<typeof vi.fn>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function toResponse(answer: Answer): Response {
  if ('rows' in answer) return json(200, { version: 3, changed: true, data: answer.rows });
  const error: Record<string, string> = { code: answer.code, message: answer.message };
  if (answer.field) error.field = answer.field;
  return json(answer.status, { error });
}

function serve(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const view = url.pathname.replace(/^\/api\/v3\/views\//, '');
  if (url.pathname.startsWith('/api/v3/views/') && view === 'admin_slips') {
    return new Promise<Response>((resolve) => {
      const read: Read = { params: url.searchParams, respond: (a) => resolve(toResponse(a)) };
      reads.push(read);
      if (auto) read.respond(auto(url.searchParams));
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
  if (view === 'transaction_documents') {
    // the detail's receipt section: left loading, it is TransactionDetail's (V2-parts)
    return new Promise<Response>(() => {});
  }
  // the page-wide reads AppShell makes (`PAGE_WIDE`, `shared/testing/page-wide.ts`): answered, ignored
  return Promise.resolve(json(200, { version: 3, changed: false }));
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    for (let j = 0; j < 5; j++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

/** Waits (real time, up to 2 s, so a short debounce is allowed) until `ok` holds. */
async function until(ok: () => boolean, what: string): Promise<void> {
  const start = Date.now();
  while (!ok()) {
    if (Date.now() - start > 2000) throw new Error(`timed out waiting for: ${what}`);
    await settle();
  }
  await settle();
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement;

interface Mounted {
  setBranch: (b: string) => void;
  setPeriod: (p: { from: string; to: string }) => void;
  setShown: (s: boolean) => void;
}

function mount(opts: { shown?: boolean; branch?: string } = {}): Mounted {
  const m = menu();
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
  const [shown, setShown] = createSignal(opts.shown ?? true);
  root = document.createElement('div');
  document.body.appendChild(root);
  const C = m.Component;
  dispose = render(
    (): JSX.Element => (
      <>
        <C branch={branch} period={period} shown={shown} />
        <Dialog />
      </>
    ),
    root,
  );
  return { setBranch, setPeriod, setShown };
}

function bodyRows(): HTMLTableRowElement[] {
  return [...root.querySelectorAll<HTMLTableRowElement>('table tbody tr')];
}

function shownIds(): string[] {
  return bodyRows().map((tr) => {
    const id = [...tr.querySelectorAll('button')].find((b) => /^XC-/.test(b.textContent ?? ''));
    return id?.textContent ?? '';
  });
}

beforeEach(() => {
  reads = [];
  auto = null;
  fetchSpy = vi.fn(serve);
  vi.stubGlobal('fetch', fetchSpy);
  setLang('th');
});

afterEach(() => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

// ── the menu record ──────────────────────────────────────────────────────────


describe('F20b: the Slips menu marks a backdated sale', () => {
  it('its own table words the mark: th ย้อนหลัง, en Backdated (W10)', () => {
    expect(table().th.backdated).toBe('ย้อนหลัง');
    expect(table().en.backdated).toBe('Backdated');
  });

  it('shows the mark in the number cell of the backdated row only', async () => {
    auto = () => ({ rows: [row(2, { backdated: true }), row(1)] });
    mount();
    await until(() => shownIds().length === 2, 'two rows');
    const [marked, live] = bodyRows();
    const pills = marked!.querySelectorAll('[data-backdated="true"]');
    expect(pills).toHaveLength(1);
    expect((pills[0]!.textContent ?? '').trim()).toBe(own('ย้อนหลัง'));
    const idCell = [...marked!.querySelectorAll('td')].find((td) => (td.textContent ?? '').includes(row(2).id));
    expect(pills[0]!.closest('td')).toBe(idCell);
    expect(live!.querySelector('[data-backdated]')).toBeNull();
  });

  it('in English the mark reads Backdated', async () => {
    setLang('en');
    auto = () => ({ rows: [row(2, { backdated: true })] });
    mount();
    await until(() => shownIds().length === 1, 'one row');
    expect((bodyRows()[0]!.querySelector('[data-backdated="true"]')?.textContent ?? '').trim()).toBe('Backdated');
  });
});
