import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { requestRefresh } from '../parts/refresh';
import { Reports } from './Reports';

interface DeferredResponse {
  params: URLSearchParams;
  resolveJson: (body: unknown, status?: number) => void;
}

function makeReport(received: string, period = { from: '2026-10-05', to: '2026-10-06' }) {
  return {
    period,
    stats: { received, paid: '50.00', fees: '10.00', bankFees: '2.00' },
    byChannel: [
      { method: 'Cash', count: 1, received, paid: '50.00', net: '50.00' },
    ],
    byCurrency: [],
    byEmployee: [
      {
        staff: 'anan',
        count: 1,
        completed: 1,
        pending: 0,
        cancelled: 0,
        received,
        paid: '50.00',
        net: '50.00',
        fees: '10.00',
        bankFees: '2.00',
        methods: [],
        currencies: [],
      },
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
    employeeTotals: { people: 2, completed: 2, received, paid: '75.00', net: '125.00' },
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
    branch: 'MAIN',
  };
}

describe('Admin Reports read-order & stale response guards (#2272)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let setBranch: Setter<string>;
  let setPeriod: Setter<{ from: string; to: string }>;

  let pendingReports: DeferredResponse[] = [];
  let pendingTx: DeferredResponse[] = [];
  let reportReads: URLSearchParams[] = [];
  let txReads: URLSearchParams[] = [];

  beforeEach(() => {
    setLang('th');
    pendingReports = [];
    pendingTx = [];
    reportReads = [];
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
                    { id: 'MAIN', name: 'Silom #2' },
                    { id: 'B9', name: 'Ari' },
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
          const params = new URLSearchParams(u.searchParams);
          reportReads.push(params);
          return new Promise<Response>((resolve) => {
            pendingReports.push({
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
    setLang('th');
  });

  function mount(initialPeriod = { from: '2026-10-01', to: '2026-10-02' }): HTMLElement {
    const [branch, sBranch] = createSignal('MAIN');
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

  it("Period P1 then P2, P1 answers last: P2's figures are shown, next Refresh sends P2's version as since, and changed:false keeps P2's figures", async () => {
    mount({ from: '2026-10-01', to: '2026-10-02' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));

    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(2));

    const p1Req = pendingReports[0]!;
    const p2Req = pendingReports[1]!;
    expect(p1Req.params.get('from')).toBe('2026-10-01');
    expect(p2Req.params.get('from')).toBe('2026-10-05');
    expect(p2Req.params.has('since')).toBe(false);

    // P2 answers first with version 22 and 222.00
    p2Req.resolveJson({
      version: 22,
      changed: true,
      data: makeReport('222.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    await settle();
    expect(root.textContent).toContain(formatMoney('222.00'));

    // Stale P1 answers last with version 11 and 111.00
    p1Req.resolveJson({
      version: 11,
      changed: true,
      data: makeReport('111.00', { from: '2026-10-01', to: '2026-10-02' }),
    });
    await settle();

    // P2's figures must still be shown
    expect(root.textContent).toContain(formatMoney('222.00'));
    expect(root.textContent).not.toContain(formatMoney('111.00'));

    // Next Refresh sends P2's version (22) as since, not P1's (11)
    requestRefresh();
    await vi.waitFor(() => expect(pendingReports.length).toBe(3));
    const refreshReq = pendingReports[2]!;
    expect(refreshReq.params.get('from')).toBe('2026-10-05');
    expect(refreshReq.params.get('since')).toBe('22');

    // Refresh then changed:false keeps P2's figures
    refreshReq.resolveJson({
      version: 22,
      changed: false,
    });
    await settle();
    expect(root.textContent).toContain(formatMoney('222.00'));
  });

  it("Staff S1 then S2 quickly, S1 answers last: S2's figures are shown and next Refresh sends S2's version as since", async () => {
    mount({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));
    pendingReports[0]!.resolveJson({
      version: 5,
      changed: true,
      data: makeReport('500.00'),
    });
    await settle();

    const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    tabs[2]!.click();
    await settle();

    const select = root.querySelector<HTMLSelectElement>('select')!;
    select.value = 'anan';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(pendingReports.length).toBe(2));

    select.value = 'dao';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(pendingReports.length).toBe(3));

    const s1Req = pendingReports[1]!;
    const s2Req = pendingReports[2]!;
    expect(s1Req.params.get('staff')).toBe('anan');
    expect(s1Req.params.has('since')).toBe(false);
    expect(s2Req.params.get('staff')).toBe('dao');
    expect(s2Req.params.has('since')).toBe(false);

    // S2 (dao) answers first with version 30
    s2Req.resolveJson({
      version: 30,
      changed: true,
      data: makeReport('763.00'),
    });
    await settle();

    // S1 (anan) answers last with version 15
    s1Req.resolveJson({
      version: 15,
      changed: true,
      data: makeReport('100.00'),
    });
    await settle();

    expect(root.textContent).toContain(formatMoney('763.00'));
    expect(root.textContent).not.toContain(formatMoney('100.00'));

    requestRefresh();
    await vi.waitFor(() => expect(pendingReports.length).toBe(4));
    expect(pendingReports[3]!.params.get('staff')).toBe('dao');
    expect(pendingReports[3]!.params.get('since')).toBe('30');
  });

  it('A filter change resets lastVersion so a refresh tick before the new filter answers does not send the old since', async () => {
    mount({ from: '2026-10-01', to: '2026-10-02' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));
    pendingReports[0]!.resolveJson({
      version: 7,
      changed: true,
      data: makeReport('100.00', { from: '2026-10-01', to: '2026-10-02' }),
    });
    await settle();

    // Change period to P2 (in flight, has not answered yet)
    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(2));
    expect(pendingReports[1]!.params.has('since')).toBe(false);

    // A refresh tick fires before P2's initial read answers
    requestRefresh();
    await vi.waitFor(() => expect(pendingReports.length).toBe(3));
    expect(pendingReports[2]!.params.get('from')).toBe('2026-10-05');
    expect(pendingReports[2]!.params.has('since')).toBe(false);

    // Tick refresh (req 3) answers first, then initial P2 read (req 2) answers later
    pendingReports[2]!.resolveJson({
      version: 9,
      changed: true,
      data: makeReport('999.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    await settle();
    pendingReports[1]!.resolveJson({
      version: 8,
      changed: true,
      data: makeReport('888.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    await settle();

    expect(root.textContent).toContain(formatMoney('999.00'));
    expect(root.textContent).not.toContain(formatMoney('888.00'));
  });

  it("A refresh tick that fails while a new period's read is in flight does not drop the new period's answer", async () => {
    mount({ from: '2026-10-01', to: '2026-10-02' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));
    pendingReports[0]!.resolveJson({
      version: 7,
      changed: true,
      data: makeReport('100.00', { from: '2026-10-01', to: '2026-10-02' }),
    });
    await settle();
    expect(root.textContent).toContain(formatMoney('100.00'));

    // Hold P2's load read, then fire a refresh tick that answers 500
    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(2));

    requestRefresh();
    await vi.waitFor(() => expect(pendingReports.length).toBe(3));
    expect(pendingReports[2]!.params.has('since')).toBe(false);

    pendingReports[2]!.resolveJson(
      {
        error: {
          code: 'INTERNAL_ERROR',
          message: 'เซิร์ฟเวอร์ขัดข้อง',
        },
      },
      500,
    );
    await settle();

    // P2's held load read answers 200 with 888.00
    pendingReports[1]!.resolveJson({
      version: 8,
      changed: true,
      data: makeReport('888.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    await settle();

    expect(root.textContent).toContain(formatMoney('888.00'));
    expect(root.textContent).not.toContain(formatMoney('100.00'));
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it('An older read fails after a newer success: no alert is shown and figures and version are kept', async () => {
    mount({ from: '2026-10-01', to: '2026-10-02' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));

    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(2));

    // Newer read (P2) succeeds first
    pendingReports[1]!.resolveJson({
      version: 25,
      changed: true,
      data: makeReport('250.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    await settle();
    expect(root.textContent).toContain(formatMoney('250.00'));
    expect(root.querySelector('[role="alert"]')).toBeNull();

    // Older read (P1) fails later
    pendingReports[0]!.resolveJson(
      {
        error: {
          code: 'SERVICE_UNAVAILABLE',
          message: 'database temporarily unavailable',
        },
      },
      503,
    );
    await settle();

    // No alert, P2's figures and version are still intact
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent).toContain(formatMoney('250.00'));

    requestRefresh();
    await vi.waitFor(() => expect(pendingReports.length).toBe(3));
    expect(pendingReports[2]!.params.get('since')).toBe('25');
  });

  it('The from > to early return does not bump reportSeq or invalidate an in-flight read', async () => {
    mount({ from: '2026-10-01', to: '2026-10-02' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));

    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(2));

    // Invalid period (from > to) triggers early return and must not bump reportSeq
    setPeriod({ from: '2026-10-10', to: '2026-10-06' });
    await settle();
    expect(pendingReports.length).toBe(2);

    // P2 resolves, then P1 resolves late
    pendingReports[1]!.resolveJson({
      version: 42,
      changed: true,
      data: makeReport('420.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    await settle();
    pendingReports[0]!.resolveJson({
      version: 10,
      changed: true,
      data: makeReport('105.00', { from: '2026-10-01', to: '2026-10-02' }),
    });
    await settle();

    expect(root.textContent).toContain(formatMoney('420.00'));
    expect(root.textContent).not.toContain(formatMoney('105.00'));
  });

  it('loadEmployeeTx ignores stale answers and stale errors when period, branch, or staff changes quickly', async () => {
    mount({ from: '2026-10-01', to: '2026-10-02' });
    await vi.waitFor(() => expect(pendingReports.length).toBe(1));
    pendingReports[0]!.resolveJson({
      version: 1,
      changed: true,
      data: makeReport('300.00', { from: '2026-10-01', to: '2026-10-02' }),
    });
    await settle();

    // Switch to Employee tab and select dao
    const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    tabs[2]!.click();
    await settle();

    const select = root.querySelector<HTMLSelectElement>('select')!;
    select.value = 'dao';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => {
      expect(pendingReports.length).toBe(2);
      expect(pendingTx.length).toBeGreaterThanOrEqual(1);
    });
    const txP1 = pendingTx[0]!;

    // Quickly change period to P2 before P1's admin_transactions answers
    const countBeforeP2 = pendingTx.length;
    setPeriod({ from: '2026-10-05', to: '2026-10-06' });
    await vi.waitFor(() => {
      expect(pendingReports.length).toBe(3);
      expect(pendingTx.length).toBeGreaterThan(countBeforeP2);
    });
    const txP2 = pendingTx[countBeforeP2]!;

    pendingReports[2]!.resolveJson({
      version: 2,
      changed: true,
      data: makeReport('300.00', { from: '2026-10-05', to: '2026-10-06' }),
    });
    pendingReports[1]!.resolveJson({
      version: 1,
      changed: true,
      data: makeReport('300.00', { from: '2026-10-01', to: '2026-10-02' }),
    });
    await settle();

    expect(txP1.params.get('from')).toBe('2026-10-01');
    expect(txP2.params.get('from')).toBe('2026-10-05');

    // P2's admin_transactions answers first, then P1's stale admin_transactions answers last
    txP2.resolveJson({
      version: 1,
      changed: true,
      data: [makeTxRow('XC-P2-NEW', 'dao')],
    });
    await settle();
    expect(root.textContent).toContain('XC-P2-NEW');

    txP1.resolveJson({
      version: 1,
      changed: true,
      data: [makeTxRow('XC-P1-OLD', 'dao')],
    });
    await settle();

    expect(root.textContent).toContain('XC-P2-NEW');
    expect(root.textContent).not.toContain('XC-P1-OLD');

    // Now change branch MAIN -> B9 -> MAIN, then B9's older admin_transactions fails after MAIN succeeds
    const countBeforeB9 = pendingTx.length;
    setBranch('B9');
    await vi.waitFor(() => expect(pendingTx.length).toBeGreaterThan(countBeforeB9));
    const txB9 = pendingTx[pendingTx.length - 1]!;

    const countBeforeMain = pendingTx.length;
    setBranch('MAIN');
    await vi.waitFor(() => expect(pendingTx.length).toBeGreaterThan(countBeforeMain));
    const txMain = pendingTx[pendingTx.length - 1]!;

    txMain.resolveJson({
      version: 1,
      changed: true,
      data: [makeTxRow('XC-MAIN-LATEST', 'dao')],
    });
    await settle();

    txB9.resolveJson(
      {
        error: {
          code: 'SERVICE_UNAVAILABLE',
          message: 'database temporarily unavailable',
        },
      },
      503,
    );
    await settle();

    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent).toContain('XC-MAIN-LATEST');
  });
});
