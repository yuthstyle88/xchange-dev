// F20b (docs/design/backdated-keying.md §15.4): a backdated sale carries its "ย้อนหลัง" mark on the Reports menu too:
// in the per-employee transaction list and as the last column of the employee CSV. Written before the implementation by
// the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a file of your own.
// The harness below is copied from Reports.extra.test.tsx (#469), trimmed to what these tests use.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { makeTransactionRow, type TransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { toast, Toast } from '../parts/toast';
import { menu } from './menu';
import { QUEUE_CARD_VIEW, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { meOf } from '../../../test-support/me';

// Amended for EX-9 (#1147) by the up-front test writer claude/709ec749: the export buttons show only to a `me`
// whose `permissions` hold `ReportsExport` (E-2: EXECUTIVE and FINANCE). `test-support/me.ts`'s table predates
// EX-1 #1070, so the permission is added here to the OWNER fixture (EXECUTIVE + ADMIN, #1030's shape).
const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as { permissions: string[] };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

const REPORTS = 'admin_reports';
const TRANSACTIONS = 'admin_transactions';
const TABS = ['ช่องทางชำระเงิน', 'สกุลเงินและส่วนต่าง', 'แยกพนักงาน'];
// OL-7 row 61: the employee tab's CSV button reads like every other CSV export; it is `#employee-csv`.
const EMPLOYEE_CSV = 'ส่งออกรายการ CSV';

const PER_EMPLOYEE_HEADERS = [
  'เลขรายการ / เวลา',
  'สถานะ',
  'ช่องทาง',
  'ประเภท',
  'จำนวนเงิน',
  'รับเข้า THB',
  'จ่ายออก THB',
];

interface Channel {
  method: string;
  count: number;
  received: string;
  paid: string;
  net: string;
}

interface EmployeeGroup {
  staff: string;
  count: number;
  completed: number;
  pending: number;
  cancelled: number;
  received: string;
  paid: string;
  net: string;
  fees: string;
  bankFees: string;
  methods: Channel[];
  currencies: { currency: string; received: string; paid: string }[];
}

interface ReportsData {
  period: { from: string; to: string };
  stats: { received: string; paid: string; fees: string; bankFees: string };
  byChannel: Channel[];
  byCurrency: { currency: string; buyAmount: string; sellAmount: string; principal: string; fees: string }[];
  reference: { referenced: number; unreferenced: number; value: string; spread: string; percent: string | null };
  byEmployee: EmployeeGroup[];
  employeeTotals: { people: number; completed: number; received: string; paid: string; net: string };
}

function ch(method: string, count: number, received: string, paid: string, net: string): Channel {
  return { method, count, received, paid, net };
}

function methods(): Channel[] {
  return ['Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer'].map(
    (m) => ch(m, 0, '0.00', '0.00', '0.00'),
  );
}

const GROUPS: EmployeeGroup[] = [
  {
    staff: 'dao',
    count: 2,
    completed: 2,
    pending: 0,
    cancelled: 0,
    received: '763.00',
    paid: '3025.00',
    net: '-2262.01',
    fees: '8.00',
    bankFees: '1.00',
    methods: methods(),
    currencies: [{ currency: 'EUR', received: '0.00', paid: '20.00' }],
  },
];

function report(over: Partial<ReportsData> = {}): ReportsData {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    stats: { received: '763.00', paid: '3025.00', fees: '8.00', bankFees: '1.00' },
    byChannel: methods(),
    byCurrency: [],
    reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: null },
    byEmployee: GROUPS,
    employeeTotals: { people: 1, completed: 2, received: '763.00', paid: '3025.00', net: '-2262.01' },
    ...over,
  };
}

function tx(n: number, over: Partial<TransactionRow> = {}) {
  return makeTransactionRow({
    id: `XC-20261006-${String(n).padStart(6, '0')}`,
    date: '2026-10-06',
    created: '2026-10-06T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '30.0000',
    unitSize: '1',
    principal: '3000.00',
    fee: '5.00',
    bankFee: '0.00',
    settlement: '2994.99',
    method: 'Cash',
    customer: `Customer ${n}`,
    customerId: `CUS-${n}`,
    staff: 'dao',
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

class FakeServer {
  requests: { view: string; params: URLSearchParams }[] = [];
  data: ReportsData = report();
  first: Record<string, TransactionRow[]> = { '': [], dao: [] };

  reads(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === REPORTS).map((r) => r.params);
  }

  walks(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === TRANSACTIONS).map((r) => r.params);
  }

  answer(url: string): Promise<Response> {
    const u = new URL(url, 'http://desk.test');
    const view = u.pathname.startsWith('/api/v3/views/') ? u.pathname.slice('/api/v3/views/'.length) : '';
    this.requests.push({ view, params: u.searchParams });
    const ok = (body: unknown) =>
      Promise.resolve(
        new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } }),
      );
    const p = u.searchParams;
    switch (view) {
      case REPORTS:
        return ok({ version: 1, changed: true, data: this.data });
      case TRANSACTIONS:
        return ok({ version: 1, changed: true, data: this.first[p.get('staff') ?? ''] ?? [] });
      case 'admin_branches':
        return ok({ version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'Silom #2' }] } });
      case 'transaction_documents':
        return ok({ version: 1, changed: true, data: { ref: p.get('ref'), files: [] } });
      case 'me':
        return ok({ version: 1, changed: true, data: EXPORTER_ME });
      case QUEUE_CARD_VIEW:
        return ok(queueCardUnchanged());
      default:
        return Promise.resolve(new Response('not found', { status: 404 }));
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
    vi.fn((input: unknown) => server.answer(String(input))),
  );
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
  setLang('th');
});

function mount(opts: { branch?: string; from?: string; to?: string } = {}): HTMLElement {
  const C = menu.Component;
  const [branch] = createSignal(opts.branch ?? 'MAIN');
  const [period] = createSignal({ from: opts.from ?? '2026-10-05', to: opts.to ?? '2026-10-06' });
  const [shown] = createSignal(true);
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

async function openTab(name: string): Promise<HTMLElement> {
  const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
  const tab = tabs.find((t) => (t.textContent ?? '').replace(/\s+/g, ' ').trim() === name);
  expect(tab).toBeDefined();
  tab!.click();
  await settle();
  const open = [...root.querySelectorAll<HTMLElement>('[role="tabpanel"]')].filter((p) => !p.hidden);
  expect(open.length).toBe(1);
  return open[0]!;
}

function tableWith(el: ParentNode, names: string[]): HTMLTableElement {
  const tables = [...el.querySelectorAll<HTMLTableElement>('table')];
  const found = tables.filter((t) => {
    const h = [...t.querySelectorAll('thead th')].map((th) => (th.textContent ?? '').replace(/\s+/g, ' ').trim());
    return JSON.stringify(h) === JSON.stringify(names);
  });
  expect(found.length, `one table headed ${names.join(' | ')}`).toBe(1);
  return found[0]!;
}


const table = import.meta.glob<{ th: Record<string, string>; en: Record<string, string> }>('./i18n.ts', { eager: true })['./i18n.ts']!;

describe('F20b: the Reports menu marks a backdated sale', () => {
  it('its own table words the mark: th ย้อนหลัง, en Backdated (W10)', () => {
    expect(table.th.backdated).toBe('ย้อนหลัง');
    expect(table.en.backdated).toBe('Backdated');
  });

  it('the per-employee list shows the mark on the backdated row only, beside its number', async () => {
    server.first.dao = [tx(3, { backdated: true }), tx(2)];
    mount();
    await vi.waitFor(() => expect(server.reads().length).toBeGreaterThan(0));
    await settle();
    const panel = await openTab(TABS[2]!);
    const dao = panel.querySelector('details')!;
    dao.open = true;
    dao.dispatchEvent(new Event('toggle'));
    await vi.waitFor(() => expect((dao.textContent ?? '').includes(tx(3).id)).toBe(true));

    const body = [...tableWith(dao, PER_EMPLOYEE_HEADERS).querySelectorAll('tbody tr')];
    expect(body).toHaveLength(2);
    const marked = body[0]!.querySelectorAll('[data-backdated="true"]');
    expect(marked).toHaveLength(1);
    expect((marked[0]!.textContent ?? '').trim()).toBe('ย้อนหลัง');
    expect(marked[0]!.closest('td')).toBe(body[0]!.querySelector('td'));
    expect(body[1]!.querySelector('[data-backdated]')).toBeNull();
  });

  it('the employee CSV ends with ย้อนหลัง: ใช่ for a backdated row, empty for a live one', async () => {
    const blobs: Blob[] = [];
    const origCreate = URL.createObjectURL;
    const origRevoke = URL.revokeObjectURL;
    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    try {
      server.first[''] = [tx(2, { backdated: true }), tx(1)];
      mount();
      await vi.waitFor(() => expect(server.reads().length).toBeGreaterThan(0));
      await settle();
      const panel = await openTab(TABS[2]!);
      const csvBtn = [...panel.querySelectorAll<HTMLButtonElement>('button')].find(
        (b) => (b.textContent ?? '').trim() === EMPLOYEE_CSV,
      );
      expect(csvBtn).toBeDefined();
      csvBtn!.click();
      await vi.waitFor(() => expect(blobs).toHaveLength(1));
      const lines = new TextDecoder().decode(new Uint8Array(await blobs[0]!.arrayBuffer()).slice(3)).split('\r\n');
      expect(lines).toHaveLength(3);
      const header = lines[0]!.split(',');
      expect(header).toHaveLength(13);
      expect(header[12]).toBe('"ย้อนหลัง"');
      // oldest first: tx(1) (live), then tx(2) (backdated)
      expect(lines[1]!.split(',')).toHaveLength(13);
      expect(lines[1]!.split(',')[1]).toBe(`"${tx(1).id}"`);
      expect(lines[1]!.split(',')[12]).toBe('""');
      expect(lines[2]!.split(',')[1]).toBe(`"${tx(2).id}"`);
      expect(lines[2]!.split(',')[12]).toBe('"ใช่"');
    } finally {
      URL.createObjectURL = origCreate;
      URL.revokeObjectURL = origRevoke;
    }
  });
});
