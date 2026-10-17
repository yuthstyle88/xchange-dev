// #2550: When a selected staff filter is absent from a newly loaded branch/period's byEmployee,
// the employee dropdown must keep showing that staff option as selected so the user can see the
// active filter and switch back to All staff.
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { QUEUE_CARD_VIEW, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { meOf } from '../../../test-support/me';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { toast, Toast } from '../parts/toast';

const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as {
    permissions: string[];
  };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/reports/menu.ts exists').toBeDefined();
  return m!.menu;
}

const REPORTS = 'admin_reports';
const TRANSACTIONS = 'admin_transactions';
const EMPLOYEE_TAB = 'แยกพนักงาน';

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

interface Totals {
  people: number;
  completed: number;
  received: string;
  paid: string;
  net: string;
}

interface ReportsData {
  period: { from: string; to: string };
  stats: { received: string; paid: string; fees: string; bankFees: string };
  byChannel: Channel[];
  byCurrency: {
    currency: string;
    buyAmount: string;
    sellAmount: string;
    principal: string;
    fees: string;
  }[];
  reference: {
    referenced: number;
    unreferenced: number;
    value: string;
    spread: string;
    percent: string | null;
  };
  byEmployee: EmployeeGroup[];
  employeeTotals: Totals;
}

function ch(method: string, count = 0, received = '0.00', paid = '0.00', net = '0.00'): Channel {
  return { method, count, received, paid, net };
}

function methods(): Channel[] {
  return ['Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer'].map((m) => ch(m));
}

const ANAN: EmployeeGroup = {
  staff: 'anan',
  count: 2,
  completed: 2,
  pending: 0,
  cancelled: 0,
  received: '402.00',
  paid: '59.00',
  net: '343.00',
  fees: '3.00',
  bankFees: '0.50',
  methods: methods(),
  currencies: [{ currency: 'USD', received: '2.00', paid: '10.00' }],
};

const DAO: EmployeeGroup = {
  staff: 'dao',
  count: 3,
  completed: 3,
  pending: 0,
  cancelled: 0,
  received: '763.00',
  paid: '3025.00',
  net: '-2262.01',
  fees: '8.00',
  bankFees: '1.00',
  methods: methods(),
  currencies: [{ currency: 'USD', received: '101.00', paid: '0.00' }],
};

const SOMCHAI: EmployeeGroup = {
  staff: 'somchai',
  count: 1,
  completed: 1,
  pending: 0,
  cancelled: 0,
  received: '0.00',
  paid: '2000.00',
  net: '-2000.00',
  fees: '0.00',
  bankFees: '0.00',
  methods: methods(),
  currencies: [{ currency: 'JPY', received: '10000.00', paid: '0.00' }],
};

const MAIN_TOTALS_ALL: Totals = {
  people: 3,
  completed: 6,
  received: '1165.00',
  paid: '5084.00',
  net: '-3919.01',
};

const MAIN_TOTALS_DAO: Totals = {
  people: 1,
  completed: 3,
  received: '763.00',
  paid: '3025.00',
  net: '-2262.01',
};

const B9_TOTALS_ALL: Totals = {
  people: 2,
  completed: 3,
  received: '402.00',
  paid: '2059.00',
  net: '-1657.00',
};

const NOBODY: Totals = {
  people: 0,
  completed: 0,
  received: '0.00',
  paid: '0.00',
  net: '0.00',
};

function makeReport(byEmployee: EmployeeGroup[], employeeTotals: Totals): ReportsData {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    stats: {
      received: employeeTotals.received,
      paid: employeeTotals.paid,
      fees: '0.00',
      bankFees: '0.00',
    },
    byChannel: methods(),
    byCurrency: [],
    reference: {
      referenced: 0,
      unreferenced: 0,
      value: '0.00',
      spread: '0.00',
      percent: null,
    },
    byEmployee,
    employeeTotals,
  };
}

function reply(status: number, body: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

describe('#2550 Admin reports: selected staff filter remains visible when absent from new data', () => {
  let reads: URLSearchParams[];
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let setBranch: Setter<string>;

  beforeEach(() => {
    setLang('th');
    reads = [];
    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const u = new URL(String(input), 'http://desk.test');
        const view = u.pathname.startsWith('/api/v3/views/')
          ? u.pathname.slice('/api/v3/views/'.length)
          : '';
        const p = u.searchParams;

        switch (view) {
          case REPORTS: {
            reads.push(p);
            const branch = p.get('branch') ?? 'MAIN';
            const staff = p.get('staff') ?? '';
            if (branch === 'MAIN') {
              const totals =
                staff === '' ? MAIN_TOTALS_ALL : staff === 'dao' ? MAIN_TOTALS_DAO : NOBODY;
              return reply(200, {
                version: 7,
                changed: true,
                data: makeReport([ANAN, DAO, SOMCHAI], totals),
              });
            }
            // Branch B9 has transactions only for anan and somchai (dao has none).
            const totals = staff === '' ? B9_TOTALS_ALL : NOBODY;
            return reply(200, {
              version: 8,
              changed: true,
              data: makeReport([ANAN, SOMCHAI], totals),
            });
          }
          case TRANSACTIONS:
            return reply(200, { version: 7, changed: true, data: [] });
          case 'admin_branches':
            return reply(200, {
              version: 7,
              changed: true,
              data: {
                branches: [
                  { id: 'MAIN', name: 'Silom #2' },
                  { id: 'B9', name: 'Ari' },
                ],
              },
            });
          case 'me':
            return reply(200, { version: 1, changed: true, data: EXPORTER_ME });
          case QUEUE_CARD_VIEW:
            return reply(200, queueCardUnchanged(7));
          default:
            return reply(404, { error: { code: 'notFound', message: 'not found' } });
        }
      }),
    );
    location.hash = '#reports';
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    closeDialog();
    toast('');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    location.hash = '';
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 5; i += 1) {
      await new Promise((r) => setTimeout(r, 10));
    }
  }

  function text(el: Element | null | undefined): string {
    return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  }

  async function mountEmployeeTab(): Promise<HTMLElement> {
    const C = theMenu().Component;
    const [branch, sb] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
    const [shown] = createSignal(true);
    setBranch = sb;
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
    await vi.waitFor(() => expect(reads.length).toBeGreaterThan(0));
    await settle();

    const tab = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(
      (b) => text(b) === EMPLOYEE_TAB,
    )!;
    tab.click();
    await settle();

    const openPanels = [...root.querySelectorAll<HTMLElement>('[role="tabpanel"]')].filter(
      (p) => !p.hidden,
    );
    return openPanels[0]!;
  }

  function employeeSelect(panel: HTMLElement): HTMLSelectElement {
    const s = panel.querySelector<HTMLSelectElement>('#employee-select');
    expect(s, 'employee select').not.toBeNull();
    return s!;
  }

  /** Change a <select> like a real user: only dispatches 'change' when the selected value changes. */
  function userPickSelect(s: HTMLSelectElement, nextValue: string): void {
    if (s.value === nextValue) return;
    s.value = nextValue;
    s.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('row 1: dao picked, new answer has no dao -> Dropdown shows dao (not All staff)', async () => {
    const panel = await mountEmployeeTab();
    const s = employeeSelect(panel);

    userPickSelect(s, 'dao');
    await vi.waitFor(() => expect(reads.length).toBe(2));
    await settle();
    expect(s.value).toBe('dao');

    // Switch to branch B9 where dao has no transactions.
    setBranch('B9');
    await vi.waitFor(() => expect(reads.length).toBe(3));
    await settle();

    expect(reads[2]!.get('branch')).toBe('B9');
    expect(reads[2]!.get('staff')).toBe('dao');
    expect([...s.options].map((o) => o.value)).toEqual(['', 'anan', 'somchai', 'dao']);
    expect(s.value).toBe('dao');
  });

  it("row 2: Then pick All staff -> Request sends no staff filter; totals are the branch's", async () => {
    const panel = await mountEmployeeTab();
    const s = employeeSelect(panel);

    userPickSelect(s, 'dao');
    await vi.waitFor(() => expect(reads.length).toBe(2));
    await settle();

    setBranch('B9');
    await vi.waitFor(() => expect(reads.length).toBe(3));
    await settle();

    // User sees 'dao' selected in the dropdown and picks All staff ('').
    userPickSelect(s, '');
    await vi.waitFor(() => expect(reads.length).toBe(4));
    await settle();

    const lastRead = reads[3]!;
    expect(lastRead.get('branch')).toBe('B9');
    expect(lastRead.has('staff')).toBe(false);
    expect(s.value).toBe('');
    expect(text(panel)).toContain(
      `รวม 2 คน · สำเร็จ 3 รายการ · รับเข้า ${formatMoney('402.00')} · จ่ายออก ${formatMoney('2059.00')} · สุทธิ ${formatMoney('-1657.00')} THB`,
    );
  });

  it('edge: selected staff is listed once when absent from answer and not duplicated when present', async () => {
    const panel = await mountEmployeeTab();
    const s = employeeSelect(panel);

    userPickSelect(s, 'dao');
    await vi.waitFor(() => expect(reads.length).toBe(2));
    await settle();

    // On MAIN (dao is present in byEmployee), dao appears exactly once.
    expect([...s.options].map((o) => o.value)).toEqual(['', 'anan', 'dao', 'somchai']);

    // On B9 (dao is absent from byEmployee), dao still appears exactly once.
    setBranch('B9');
    await vi.waitFor(() => expect(reads.length).toBe(3));
    await settle();
    expect([...s.options].map((o) => o.value)).toEqual(['', 'anan', 'somchai', 'dao']);

    // Back on MAIN (dao is present again), dao is not added twice.
    setBranch('MAIN');
    await vi.waitFor(() => expect(reads.length).toBe(4));
    await settle();
    expect([...s.options].map((o) => o.value)).toEqual(['', 'anan', 'dao', 'somchai']);
    expect(s.value).toBe('dao');
  });
});
