import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { sharedSession } from '../../../shared/session/session';
import { Reports } from './Reports';

const mockReportsData = {
  period: { from: '2026-10-05', to: '2026-10-06' },
  stats: { received: '1160.00', paid: '3309.00', fees: '71.00', bankFees: '17.50' },
  byChannel: [
    { method: 'Cash', count: 1, received: '1160.00', paid: '3309.00', net: '-2149.00' },
  ],
  byCurrency: [],
  reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: null },
  byEmployee: [
    {
      staff: 'alice',
      count: 1,
      completed: 1,
      pending: 0,
      cancelled: 0,
      received: '1160.00',
      paid: '3309.00',
      net: '-2149.00',
      fees: '0.00',
      bankFees: '0.00',
      methods: [],
      currencies: [],
    },
  ],
  employeeTotals: { people: 1, completed: 1, received: '1160.00', paid: '3309.00', net: '-2149.00' },
};

function tx(n: number) {
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
    customerId: 'CX-00000001',
    staff: 'alice',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
  };
}

describe('WE-23: screens/admin/reports/worded.test.tsx', () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  let branchesHandler: () => Promise<Response>;
  let reportsHandler: (params: URLSearchParams) => Promise<Response>;
  let transactionsHandler: (params: URLSearchParams) => Promise<Response>;

  function json(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe?.({ id: 'owner', permissions: ['ReportsExport'] } as any);

    branchesHandler = async () =>
      json(200, {
        version: 1,
        changed: true,
        data: {
          branches: [
            { id: 'MAIN', name: 'Main Branch' },
            { id: 'B02', name: 'Branch 2' },
          ],
        },
      });

    reportsHandler = async () =>
      json(200, {
        version: 1,
        changed: true,
        data: mockReportsData,
      });

    transactionsHandler = async () =>
      json(200, {
        version: 1,
        changed: true,
        data: [tx(1)],
      });

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), 'http://desk.test');
        const view = url.pathname.startsWith('/api/v3/views/')
          ? url.pathname.replace(/^\/api\/v3\/views\//, '')
          : null;
        if (view === 'admin_branches') {
          return branchesHandler();
        }
        if (view === 'admin_reports') {
          return reportsHandler(url.searchParams);
        }
        if (view === 'admin_transactions') {
          return transactionsHandler(url.searchParams);
        }
        return Promise.resolve(json(200, { version: 1, changed: false }));
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

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

  function mount(initialBranch = 'MAIN') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period, setPeriod] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
    const [shown, setShown] = createSignal(true);

    root = document.createElement('div');
    document.body.appendChild(root);

    dispose = render(
      (): JSX.Element => <Reports branch={branch} period={period} shown={shown} />,
      root,
    );

    return { setBranch, setPeriod, setShown };
  }

  function alertText(): string | null {
    const el = root.querySelector('[role="alert"]');
    return el ? (el.textContent ?? '').trim() : null;
  }

  async function switchToEmployeeTab(): Promise<void> {
    const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    const empTab = tabs[2];
    expect(empTab).toBeDefined();
    empTab!.click();
    await settle();
  }

  async function selectEmployee(name: string): Promise<void> {
    await switchToEmployeeTab();
    await until(() => root.querySelector('#employee-select') !== null, 'employee select');
    const select = root.querySelector<HTMLSelectElement>('#employee-select')!;
    select.value = name;
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
  }

  async function clickExportButton(): Promise<void> {
    await until(
      () =>
        [...root.querySelectorAll<HTMLButtonElement>('button')].some(
          (b) =>
            (b.textContent ?? '').includes('ส่งออกรายการ CSV') ||
            (b.textContent ?? '').includes('Export CSV'),
        ),
      'export button',
    );
    const btn = [...root.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) =>
        (b.textContent ?? '').includes('ส่งออกรายการ CSV') ||
        (b.textContent ?? '').includes('Export CSV'),
    )!;
    btn.click();
    await settle();
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: readBranches (admin_branches view call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: readBranches failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      branchesHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await until(() => alertText() !== null, 'alert is visible');

      expect(alertText()).toBe(th['error.network']);
      expect(alertText()).not.toContain('TypeError');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await until(() => alertText() !== null, 'alert in en');

      expect(alertText()).toBe(en['error.network']);
      expect(alertText()).not.toContain('TypeError');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      branchesHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'Internal branch DB down' },
          }),
        );
      mount();
      await until(() => alertText() !== null, 'alert is visible');

      expect(alertText()).toBe(th['error.serverError']);
      expect(alertText()).not.toContain('Internal branch DB down');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      branchesHandler = () =>
        Promise.resolve(
          json(400, {
            error: {
              code: 'customBranchError',
              message: 'ข้อความแจ้งเตือนสาขาเฉพาะ',
            },
          }),
        );
      mount();
      await until(() => alertText() !== null, 'alert is visible');

      expect(alertText()).toBe('ข้อความแจ้งเตือนสาขาเฉพาะ');
      expect(alertText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: readReports (admin_reports view call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: readReports failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      reportsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await until(() => alertText() !== null, 'alert is visible');

      expect(alertText()).toBe(th['error.network']);
      expect(alertText()).not.toContain('TypeError');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await until(() => alertText() !== null, 'alert in en');

      expect(alertText()).toBe(en['error.network']);
      expect(alertText()).not.toContain('TypeError');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      reportsHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'Internal reports DB error' },
          }),
        );
      mount();
      await until(() => alertText() !== null, 'alert is visible');

      expect(alertText()).toBe(th['error.serverError']);
      expect(alertText()).not.toContain('Internal reports DB error');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      reportsHandler = () =>
        Promise.resolve(
          json(400, {
            error: {
              code: 'customReportsRefusal',
              message: 'ข้อความปฏิเสธรายงานเฉพาะ',
            },
          }),
        );
      mount();
      await until(() => alertText() !== null, 'alert is visible');

      expect(alertText()).toBe('ข้อความปฏิเสธรายงานเฉพาะ');
      expect(alertText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 3: loadEmployeeTx (admin_transactions view call for staff)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 3: loadEmployeeTx failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount();
      await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');

      transactionsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await selectEmployee('alice');
      await until(() => alertText() !== null, 'employee tx alert');

      expect(alertText()).toBe(th['error.network']);
      expect(alertText()).not.toContain('TypeError');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await selectEmployee('alice');
      await until(() => alertText() !== null, 'employee tx alert in en');

      expect(alertText()).toBe(en['error.network']);
      expect(alertText()).not.toContain('TypeError');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      mount();
      await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');

      transactionsHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'Employee tx DB crash' },
          }),
        );
      await selectEmployee('alice');
      await until(() => alertText() !== null, 'employee tx alert');

      expect(alertText()).toBe(th['error.serverError']);
      expect(alertText()).not.toContain('Employee tx DB crash');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      mount();
      await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');

      transactionsHandler = () =>
        Promise.resolve(
          json(400, {
            error: {
              code: 'customTxRefusal',
              message: 'ไม่สามารถดูรายการพนักงานนี้ได้',
            },
          }),
        );
      await selectEmployee('alice');
      await until(() => alertText() !== null, 'employee tx alert');

      expect(alertText()).toBe('ไม่สามารถดูรายการพนักงานนี้ได้');
      expect(alertText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 4: exporting (admin_transactions walkPages export call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 4: exporting failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount();
      await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');

      transactionsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await clickExportButton();
      await until(() => alertText() !== null, 'export alert');

      expect(alertText()).toBe(th['error.network']);
      expect(alertText()).not.toContain('TypeError');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await clickExportButton();
      await until(() => alertText() !== null, 'export alert in en');

      expect(alertText()).toBe(en['error.network']);
      expect(alertText()).not.toContain('TypeError');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      mount();
      await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');

      transactionsHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'Export DB failure' },
          }),
        );
      await clickExportButton();
      await until(() => alertText() !== null, 'export alert');

      expect(alertText()).toBe(th['error.serverError']);
      expect(alertText()).not.toContain('Export DB failure');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      mount();
      await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');

      transactionsHandler = () =>
        Promise.resolve(
          json(400, {
            error: {
              code: 'customExportRefusal',
              message: 'ส่งออกไม่สำเร็จเฉพาะทาง',
            },
          }),
        );
      await clickExportButton();
      await until(() => alertText() !== null, 'export alert');

      expect(alertText()).toBe('ส่งออกไม่สำเร็จเฉพาะทาง');
      expect(alertText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // §4.3: Branch switch stale data handling
  // ──────────────────────────────────────────────────────────────────────────
  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const { setBranch } = mount('MAIN');
      await until(() => root.querySelector('.stat-card') !== null, 'MAIN data rendered');

      // MAIN data is displayed
      expect(root.textContent).toContain('1,160.00');

      // Switch to B02, where readReports fails
      reportsHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'Failed to read B02' },
          }),
        );
      setBranch('B02');
      await until(() => alertText() !== null, 'error alert on B02');

      expect(alertText()).toBe(th['error.serverError']);
      // Data from MAIN must not stay on screen
      expect(root.querySelectorAll('tbody tr').length).toBe(0);
      expect(root.textContent).not.toContain('1,160.00');
      expect(root.textContent).not.toContain('-2,149.00');
    });
  });
});
