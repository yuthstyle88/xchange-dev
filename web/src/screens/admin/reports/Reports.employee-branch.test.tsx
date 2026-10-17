import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Reports } from './Reports';

interface DeferredResponse {
  params: URLSearchParams;
  resolveJson: (body: unknown, status?: number) => void;
}

function makeReport(received: string, period = { from: '2026-10-01', to: '2026-10-02' }) {
  return {
    period,
    stats: { received, paid: '50.00', fees: '10.00', bankFees: '2.00' },
    byChannel: [
      { method: 'Cash', count: 1, received, paid: '50.00', net: '50.00' },
    ],
    byCurrency: [],
    byEmployee: [
      {
        staff: 'dao',
        count: 1,
        completed: 1,
        pending: 0,
        cancelled: 0,
        received,
        paid: '25.00',
        net: '75.00',
        fees: '5.00',
        bankFees: '1.00',
        methods: [],
        currencies: [],
      },
    ],
    employeeTotals: { people: 1, completed: 1, received, paid: '25.00', net: '75.00' },
    reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: '0.00' },
  };
}

function makeTxRow(id: string, staff = 'dao') {
  return {
    id,
    date: '2026-10-05',
    created: '2026-10-05T08:00:00Z',
    status: 'COMPLETED',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '35.00',
    settlement: '3500.00',
    fee: '0.00',
    bankFee: '0.00',
    method: 'Cash',
    staff,
    customerName: 'Test Customer',
    customerIdType: 'PASSPORT',
    customerIdNumber: 'AA123456',
    branch: 'A',
  };
}

describe('Admin Reports employee transactions on branch and period switch (#2893)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let setBranch: Setter<string>;
  let setPeriod: Setter<{ from: string; to: string }>;

  let pendingTx: DeferredResponse[] = [];
  let txReads: URLSearchParams[] = [];

  beforeEach(() => {
    setLang('th');
    pendingTx = [];
    txReads = [];

    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);
        const u = new URL(url, 'http://desk.test');

        if (u.pathname === '/api/v3/views/admin_branches') {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: {
                  branches: [
                    { id: 'A', name: 'Branch A' },
                    { id: 'B', name: 'Branch B' },
                  ],
                },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (u.pathname === '/api/v3/views/admin_reports') {
          const from = u.searchParams.get('from') || '2026-10-01';
          const to = u.searchParams.get('to') || '2026-10-02';
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: makeReport('100.00', { from, to }),
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (u.pathname === '/api/v3/views/admin_transactions') {
          const params = new URLSearchParams(u.searchParams);
          txReads.push(params);
          return new Promise<Response>((resolve) => {
            pendingTx.push({
              params,
              resolveJson: (body: unknown, status = 200) => {
                resolve(
                  new Response(JSON.stringify(body), {
                    status,
                    headers: { 'content-type': 'application/json' },
                  }),
                );
              },
            });
          });
        }

        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mount(initialBranch = 'A', initialPeriod = { from: '2026-10-01', to: '2026-10-02' }): HTMLElement {
    const [branch, sBranch] = createSignal(initialBranch);
    const [period, sPeriod] = createSignal(initialPeriod);
    const [shown] = createSignal(true);
    setBranch = sBranch;
    setPeriod = sPeriod;
    dispose = render(
      () => <Reports branch={branch} period={period} shown={shown} />,
      root,
    );
    return root;
  }

  async function settle(): Promise<void> {
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
  }

  async function openEmployeeTabAndSelectDao(): Promise<void> {
    const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    tabs[2]!.click();
    await settle();

    const select = root.querySelector<HTMLSelectElement>('select')!;
    select.value = 'dao';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(pendingTx.length).toBeGreaterThanOrEqual(1));
  }

  // Row 1: Employee selected, rows of branch A shown; switch to branch B while B's read is in flight -> no branch A rows are shown under the employee
  it("Row 1: switch to branch B while B's read is in flight: no branch A rows are shown under the employee", async () => {
    mount('A', { from: '2026-10-01', to: '2026-10-02' });
    await openEmployeeTabAndSelectDao();

    // Answer branch A read
    const txA = pendingTx[pendingTx.length - 1]!;
    expect(txA.params.get('branch')).toBe('A');
    txA.resolveJson({
      version: 1,
      changed: true,
      data: [makeTxRow('A-1', 'dao')],
    });
    await settle();

    expect(root.querySelector('[data-employee-ref="A-1"]')).not.toBeNull();

    // Switch to branch B; B's read is now in flight
    const beforeCount = pendingTx.length;
    setBranch('B');
    await vi.waitFor(() => expect(pendingTx.length).toBeGreaterThan(beforeCount));
    const txB = pendingTx[pendingTx.length - 1]!;
    expect(txB.params.get('branch')).toBe('B');

    // While B's read is in flight, no branch A rows may be shown
    expect(root.querySelector('[data-employee-ref="A-1"]')).toBeNull();
  });

  // Row 2: As row 1, then B's read fails -> the error banner is shown and no branch A rows are shown
  it("Row 2: switch to branch B and B's read fails: error banner is shown and no branch A rows are shown", async () => {
    mount('A', { from: '2026-10-01', to: '2026-10-02' });
    await openEmployeeTabAndSelectDao();

    const txA = pendingTx[pendingTx.length - 1]!;
    txA.resolveJson({
      version: 1,
      changed: true,
      data: [makeTxRow('A-1', 'dao')],
    });
    await settle();

    expect(root.querySelector('[data-employee-ref="A-1"]')).not.toBeNull();

    const beforeCount = pendingTx.length;
    setBranch('B');
    await vi.waitFor(() => expect(pendingTx.length).toBeGreaterThan(beforeCount));
    const txB = pendingTx[pendingTx.length - 1]!;

    // B's read fails
    txB.resolveJson(
      {
        error: {
          code: 'INTERNAL_ERROR',
          message: 'เซิร์ฟเวอร์ขัดข้อง',
        },
      },
      500,
    );
    await settle();

    expect(root.querySelector('[role="alert"]')).not.toBeNull();
    expect(root.querySelector('[data-employee-ref="A-1"]')).toBeNull();
  });

  // Row 3: Employee selected, rows of period P1 shown; switch to period P2 while P2's read is in flight -> no P1 rows are shown
  it("Row 3: switch to period P2 while P2's read is in flight: no P1 rows are shown", async () => {
    mount('A', { from: '2026-10-01', to: '2026-10-02' });
    await openEmployeeTabAndSelectDao();

    const txP1 = pendingTx[pendingTx.length - 1]!;
    expect(txP1.params.get('from')).toBe('2026-10-01');
    txP1.resolveJson({
      version: 1,
      changed: true,
      data: [makeTxRow('P1-1', 'dao')],
    });
    await settle();

    expect(root.querySelector('[data-employee-ref="P1-1"]')).not.toBeNull();

    // Switch to P2; P2's read is now in flight
    const beforeCount = pendingTx.length;
    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingTx.length).toBeGreaterThan(beforeCount));
    const txP2 = pendingTx[pendingTx.length - 1]!;
    expect(txP2.params.get('from')).toBe('2026-10-05');

    // While P2's read is in flight, no P1 rows may be shown
    expect(root.querySelector('[data-employee-ref="P1-1"]')).toBeNull();
  });
});
