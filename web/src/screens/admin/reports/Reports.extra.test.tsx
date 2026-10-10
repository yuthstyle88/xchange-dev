// V2-reports (#469): extra specification tests for controller's decisions:
// 1. v2's 7 columns in the per-employee transaction list:
//    `เลขรายการ / เวลา`, `สถานะ`, `ช่องทาง`, `ประเภท`, `จำนวนเงิน`, `รับเข้า THB`, `จ่ายออก THB`
// 2. Header-only employee CSV download with no rows (when no transactions), with NO toast.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { closeDialog, Dialog } from '../parts/Dialog';
import { formatBangkokTime } from '../parts/TransactionTable';
import { toast, Toast } from '../parts/toast';
import { menu } from './menu';

const REPORTS = 'admin_reports';
const TRANSACTIONS = 'admin_transactions';
const TABS = ['ช่องทางชำระเงิน', 'สกุลเงินและส่วนต่าง', 'แยกพนักงาน'];
const EMPLOYEE_CSV = 'ดาวน์โหลด CSV';

const EMPLOYEE_CSV_HEADERS = [
  'พนักงาน',
  'เลขรายการ',
  'วันที่',
  'สถานะ',
  'ช่องทาง',
  'ประเภท',
  'สกุลเงิน',
  'จำนวน',
  'รับเข้า THB',
  'จ่ายออก THB',
  'ค่าบริการ THB',
  'ค่าธรรมเนียมธนาคาร THB',
];

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

function tx(n: number, over: Partial<TransactionRow> = {}): TransactionRow {
  return {
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
  };
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
        return ok({ version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: null } });
      case 'kiosk_notifications':
        return ok({ version: 1, changed: false });
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

function rows(table: HTMLTableElement): string[][] {
  return [...table.querySelectorAll('tbody tr')].map((tr) =>
    [...tr.querySelectorAll('td')].map((td) => (td.textContent ?? '').replace(/\s+/g, ' ').trim()),
  );
}

function quoted(cells: string[]): string {
  return cells.map((c) => `"${c.replaceAll('"', '""')}"`).join(',');
}

describe('V2-reports extra: controller decisions (#469)', () => {
  it("renders exactly v2's 7 columns in the per-employee transaction list (Decision 1)", async () => {
    server.first.dao = [
      tx(3, {
        type: 'SELL',
        currency: 'EUR',
        amount: '20.00',
        method: 'SCB Transfer',
        settlement: '763.00',
      }),
      tx(2, {
        type: 'BUY',
        currency: 'USD',
        amount: '100.00',
        method: 'Cash',
        settlement: '2994.99',
      }),
    ];

    mount();
    await vi.waitFor(() => expect(server.reads().length).toBeGreaterThan(0));
    await settle();

    const panel = await openTab(TABS[2]!);
    const dao = panel.querySelector('details')!;
    expect(dao).not.toBeNull();
    dao.open = true;
    dao.dispatchEvent(new Event('toggle'));

    await vi.waitFor(() => expect(server.walks().some((q) => q.get('staff') === 'dao')).toBe(true));
    await vi.waitFor(() => expect((dao.textContent ?? '').includes(tx(3).id)).toBe(true));

    const table = tableWith(dao, PER_EMPLOYEE_HEADERS);
    expect(table).not.toBeNull();

    const tableRows = rows(table);
    expect(tableRows).toHaveLength(2);
    expect(tableRows[0]).toEqual([
      `${tx(3).id}${tx(3).date} ${formatBangkokTime(tx(3).created)}`,
      'สำเร็จ',
      'โอนธนาคาร',
      'ร้านขาย',
      `${formatMoney('20.00')} EUR`,
      formatMoney('763.00'),
      '—',
    ]);
    expect(tableRows[1]).toEqual([
      `${tx(2).id}${tx(2).date} ${formatBangkokTime(tx(2).created)}`,
      'สำเร็จ',
      'เงินสด',
      'ร้านรับซื้อ',
      `${formatMoney('100.00')} USD`,
      '—',
      formatMoney('2994.99'),
    ]);
  });

  it('downloads a header-only employee CSV with no rows and NO toast when transactions are empty (Decision 2)', async () => {
    let blobs: Blob[] = [];
    let names: string[] = [];
    const origCreate = URL.createObjectURL;
    const origRevoke = URL.revokeObjectURL;

    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download);
    });

    try {
      server.first.dao = [];
      server.first[''] = [];

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
      await settle();

      // Verify NO toast was shown
      const toastEl = root.querySelector('.toast[role="status"]');
      expect((toastEl?.textContent ?? '').trim()).toBe('');

      // Verify CSV filename and contents (header only)
      expect(names).toEqual(['XC-Employees-2026-10-05-2026-10-06.csv']);

      const bytes = new Uint8Array(await blobs[0]!.arrayBuffer());
      expect([...bytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf]);
      const csvContent = new TextDecoder().decode(bytes.slice(3));
      const lines = csvContent.split('\r\n');
      expect(lines).toHaveLength(1);
      expect(lines[0]).toBe(quoted(EMPLOYEE_CSV_HEADERS));
    } finally {
      URL.createObjectURL = origCreate;
      URL.revokeObjectURL = origRevoke;
    }
  });

  it('downloads employee CSV with staff name and no hardcoded fallback (Decision 6)', async () => {
    let blobs: Blob[] = [];
    const origCreate = URL.createObjectURL;
    const origRevoke = URL.revokeObjectURL;

    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {});

    try {
      server.first.dao = [
        tx(2, {
          staff: 'dao',
          type: 'BUY',
          currency: 'USD',
          amount: '100.00',
          method: 'Cash',
          settlement: '2994.99',
        }),
      ];
      server.first[''] = server.first.dao;

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
      await settle();

      const bytes = new Uint8Array(await blobs[0]!.arrayBuffer());
      const csvContent = new TextDecoder().decode(bytes.slice(3));
      const lines = csvContent.split('\r\n');
      expect(lines).toHaveLength(2);
      expect(lines[0]).toBe(quoted(EMPLOYEE_CSV_HEADERS));
      expect(lines[1]).toContain('"dao"');
      expect(lines[1]).not.toContain('ไม่ระบุพนักงาน');
    } finally {
      URL.createObjectURL = origCreate;
      URL.revokeObjectURL = origRevoke;
    }
  });

  it('downloads employee CSV with empty staff without rendering fallback text (Decision 6)', async () => {
    let blobs: Blob[] = [];
    const origCreate = URL.createObjectURL;
    const origRevoke = URL.revokeObjectURL;

    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {});

    try {
      server.first[''] = [
        tx(3, {
          staff: '',
          type: 'BUY',
          currency: 'USD',
          amount: '100.00',
          method: 'Cash',
          settlement: '2994.99',
        }),
      ];

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
      await settle();

      const bytes = new Uint8Array(await blobs[0]!.arrayBuffer());
      const csvContent = new TextDecoder().decode(bytes.slice(3));
      const lines = csvContent.split('\r\n');
      expect(lines).toHaveLength(2);
      expect(lines[0]).toBe(quoted(EMPLOYEE_CSV_HEADERS));
      expect(lines[1]).not.toContain('ไม่ระบุพนักงาน');
      expect(lines[1].startsWith('"",')).toBe(true);
    } finally {
      URL.createObjectURL = origCreate;
      URL.revokeObjectURL = origRevoke;
    }
  });
});


