import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { Reports } from './Reports';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';

function tx(n: number): TransactionRow {
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
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
  };
}

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

describe('Admin Reports export in-flight guard (#2632)', () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  let branchesHandler: () => Promise<Response>;
  let reportsHandler: (params: URLSearchParams) => Promise<Response>;
  let transactionsHandler: (params: URLSearchParams) => Promise<Response>;
  let transactionsCalls: URLSearchParams[] = [];

  let blobs: Blob[] = [];
  let names: string[] = [];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  function json(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe?.({ id: 'owner', permissions: ['ReportsExport'] } as any);

    blobs = [];
    names = [];
    transactionsCalls = [];

    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download);
    });

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
          transactionsCalls.push(url.searchParams);
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
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
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

  function exportButton(): HTMLButtonElement {
    const b = root.querySelector<HTMLButtonElement>('#export');
    expect(b, 'the export button #export').not.toBeNull();
    return b!;
  }

  function employeeCsvButton(): HTMLButtonElement {
    const b = root.querySelector<HTMLButtonElement>('#employee-csv');
    expect(b, 'the employee csv button #employee-csv').not.toBeNull();
    return b!;
  }

  async function switchToEmployeeTab(): Promise<void> {
    const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    const empTab = tabs[2];
    expect(empTab, 'employee tab').toBeDefined();
    empTab!.click();
    await settle();
  }

  it('two clicks on Export before it finishes: one walk, one download', async () => {
    mount();
    await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');
    const btn = exportButton();
    expect(btn.disabled).toBe(false);

    let finishTx!: (res: Response) => void;
    const txPromise = new Promise<Response>((resolve) => {
      finishTx = resolve;
    });
    transactionsHandler = () => txPromise;

    // First click
    btn.click();
    await until(() => transactionsCalls.length === 1, 'first walk started');
    expect(btn.disabled).toBe(true);

    // Second click while export in-flight
    btn.click();
    await settle();

    // Still only one walk dispatched
    expect(transactionsCalls.length).toBe(1);

    // Complete the walk
    finishTx(json(200, { version: 1, changed: true, data: [tx(1)] }));
    await until(() => names.length === 1, 'download completed');

    expect(transactionsCalls.length).toBe(1);
    expect(names.length).toBe(1);
    expect(btn.disabled).toBe(false);
  });

  it('two clicks on Download CSV: one walk, one download', async () => {
    mount();
    await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');
    await switchToEmployeeTab();
    const btn = employeeCsvButton();
    expect(btn.disabled).toBe(false);

    let finishTx!: (res: Response) => void;
    const txPromise = new Promise<Response>((resolve) => {
      finishTx = resolve;
    });
    transactionsHandler = () => txPromise;

    // First click
    btn.click();
    await until(() => transactionsCalls.length === 1, 'first employee walk started');
    expect(btn.disabled).toBe(true);

    // Second click while export in-flight
    btn.click();
    await settle();

    // Still only one walk dispatched
    expect(transactionsCalls.length).toBe(1);

    // Complete the walk
    finishTx(json(200, { version: 1, changed: true, data: [tx(1)] }));
    await until(() => names.length === 1, 'download completed');

    expect(transactionsCalls.length).toBe(1);
    expect(names.length).toBe(1);
    expect(btn.disabled).toBe(false);
  });

  it('one export running, click the other button: ignored', async () => {
    mount();
    await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');
    const exportBtn = exportButton();
    expect(exportBtn.disabled).toBe(false);

    let finishTx!: (res: Response) => void;
    const txPromise = new Promise<Response>((resolve) => {
      finishTx = resolve;
    });
    transactionsHandler = () => txPromise;

    // Start Export CSV
    exportBtn.click();
    await until(() => transactionsCalls.length === 1, 'export walk started');
    expect(exportBtn.disabled).toBe(true);

    // Switch to employee tab
    await switchToEmployeeTab();
    const empBtn = employeeCsvButton();
    expect(empBtn.disabled).toBe(true);

    // Click employee CSV button while Export CSV is in-flight
    empBtn.click();
    await settle();

    // Still only one walk dispatched
    expect(transactionsCalls.length).toBe(1);

    // Complete the first export
    finishTx(json(200, { version: 1, changed: true, data: [tx(1)] }));
    await until(() => names.length === 1, 'export download completed');

    expect(transactionsCalls.length).toBe(1);
    expect(names.length).toBe(1);
    expect(names[0]).toMatch(/^XCHANGE-LIVE-/);
    expect(empBtn.disabled).toBe(false);
  });

  it('a refused export (outOfScope): error line shown, buttons enabled again', async () => {
    mount();
    await until(() => root.querySelector('[role="tab"]') !== null, 'tabs loaded');
    const btn = exportButton();
    expect(btn.disabled).toBe(false);

    let finishTx!: (res: Response) => void;
    const txPromise = new Promise<Response>((resolve) => {
      finishTx = resolve;
    });
    transactionsHandler = () => txPromise;

    btn.click();
    await until(() => transactionsCalls.length === 1, 'export walk started');
    expect(btn.disabled).toBe(true);

    // Refusal
    finishTx(
      json(403, {
        error: {
          code: 'outOfScope',
          message: 'ไม่อยู่ในขอบเขตที่ได้รับอนุญาต',
        },
      }),
    );
    await until(() => alertText() !== null, 'error alert shown');
    expect(alertText()).toContain('ไม่อยู่ในขอบเขตที่ได้รับอนุญาต');

    // Buttons are enabled again
    expect(btn.disabled).toBe(false);
  });
});
