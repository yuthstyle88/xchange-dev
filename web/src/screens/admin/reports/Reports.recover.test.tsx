import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { requestRefresh } from '../parts/refresh';
import { Reports } from './Reports';

interface RefusalMock {
  status: number;
  code: string;
  message: string;
}

const VALID_REPORT = {
  period: { from: '2026-10-05', to: '2026-10-06' },
  stats: { received: '100.00', paid: '50.00', fees: '10.00', bankFees: '2.00' },
  byChannel: [
    { method: 'Cash', count: 1, received: '100.00', paid: '50.00', net: '50.00' },
  ],
  byCurrency: [],
  byEmployee: [
    {
      staff: 'anan',
      count: 1,
      completed: 1,
      pending: 0,
      cancelled: 0,
      received: '100.00',
      paid: '50.00',
      net: '50.00',
      fees: '10.00',
      bankFees: '2.00',
      methods: [],
      currencies: [],
    },
  ],
  employeeTotals: { people: 1, completed: 1, received: '100.00', paid: '50.00', net: '50.00' },
  reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: '0.00' },
};

describe('Admin Reports recovery from failed refresh, filter change, and branch read (#2178)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let setBranch: Setter<string>;
  let setPeriod: Setter<{ from: string; to: string }>;

  let branchesRefusal: RefusalMock | null = null;
  let branchesDelayMs = 0;
  let reportsRefusal: RefusalMock | null = null;
  let reportVersion = 1;
  let forceUnchanged = false;

  let branchReads: URLSearchParams[] = [];
  let reportReads: URLSearchParams[] = [];

  beforeEach(() => {
    setLang('th');
    branchesRefusal = null;
    branchesDelayMs = 0;
    reportsRefusal = null;
    reportVersion = 1;
    forceUnchanged = false;
    branchReads = [];
    reportReads = [];

    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);
        const u = new URL(url, 'http://desk.test');

        if (u.pathname === '/api/v3/views/admin_branches') {
          branchReads.push(u.searchParams);
          if (branchesRefusal) {
            return Promise.resolve(
              new Response(
                JSON.stringify({
                  error: {
                    code: branchesRefusal.code,
                    message: branchesRefusal.message,
                  },
                }),
                {
                  status: branchesRefusal.status,
                  headers: { 'content-type': 'application/json' },
                },
              ),
            );
          }
          const makeBranchesResponse = () =>
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
            );
          if (branchesDelayMs > 0) {
            const d = branchesDelayMs;
            return new Promise<Response>((resolve) => {
              setTimeout(() => resolve(makeBranchesResponse()), d);
            });
          }
          return Promise.resolve(makeBranchesResponse());
        }

        if (u.pathname === '/api/v3/views/admin_reports') {
          reportReads.push(u.searchParams);
          if (reportsRefusal) {
            return Promise.resolve(
              new Response(
                JSON.stringify({
                  error: {
                    code: reportsRefusal.code,
                    message: reportsRefusal.message,
                  },
                }),
                {
                  status: reportsRefusal.status,
                  headers: { 'content-type': 'application/json' },
                },
              ),
            );
          }
          const since = u.searchParams.get('since');
          const changed = !forceUnchanged && since !== String(reportVersion);
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: reportVersion,
                changed,
                data: changed ? VALID_REPORT : undefined,
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
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

  function mount(): HTMLElement {
    const [branch, sBranch] = createSignal('MAIN');
    const [period, sPeriod] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
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

  it('Row 1: Refresh fails, the next Refresh answers changed:false — alert gone, figures kept', async () => {
    mount();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(1);
      expect(root.textContent).toContain(formatMoney('100.00'));
    });
    expect(root.querySelector('[role="alert"]')).toBeNull();

    // First refresh fails
    reportsRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-refresh-fail',
    };
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(2);
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.serverError']);
    });
    expect(root.textContent).toContain(formatMoney('100.00'));

    // Next refresh answers changed:false
    reportsRefusal = null;
    forceUnchanged = true;
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(3);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });
    expect(root.textContent).toContain(formatMoney('100.00'));
  });

  it('Row 2: A filter change fails, then Refresh — full read without since; report shown, alert gone', async () => {
    mount();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(1);
      expect(root.textContent).toContain(formatMoney('100.00'));
    });

    // Filter change (searchStaff on employee tab) fails
    reportsRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-filter-fail',
    };
    const searchInput = root.querySelector<HTMLInputElement>('#employee-search')!;
    searchInput.value = 'anan';
    searchInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      expect(reportReads.length).toBe(2);
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.serverError']);
    });
    expect(root.querySelector('#workspace-reports-0')!.textContent).not.toContain(
      formatMoney('100.00'),
    );

    // Next Refresh must do a full read without since
    reportsRefusal = null;
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(3);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });

    expect(reportReads[2]!.get('staffQ')).toBe('anan');
    expect(reportReads[2]!.has('since')).toBe(false);
    expect(root.querySelector('#workspace-reports-0')!.textContent).toContain(
      formatMoney('100.00'),
    );
  });

  it('Row 3: admin_branches fails, then a Refresh where both reads succeed — alert gone', async () => {
    branchesRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-branch-fail',
    };
    mount();
    await vi.waitFor(() => {
      expect(branchReads.length).toBe(1);
      expect(reportReads.length).toBe(1);
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.serverError']);
    });

    // Branches recover on the next refresh tick (while reports answers changed:false with since=1)
    branchesRefusal = null;
    requestRefresh();
    await vi.waitFor(() => {
      expect(branchReads.length).toBe(2);
      expect(reportReads.length).toBe(2);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });
  });

  it('Row 3 (delayed branches): admin_branches fails, then a Refresh where admin_branches answers after admin_reports changed:false — alert gone', async () => {
    branchesRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-branch-fail',
    };
    mount();
    await vi.waitFor(() => {
      expect(branchReads.length).toBe(1);
      expect(reportReads.length).toBe(1);
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.serverError']);
    });

    // Delay admin_branches past the immediate admin_reports (changed:false) answer
    branchesRefusal = null;
    branchesDelayMs = 40;
    forceUnchanged = true;
    requestRefresh();

    await vi.waitFor(() => {
      expect(branchReads.length).toBe(2);
      expect(reportReads.length).toBe(2);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });
  });

  it('Row 4: admin_branches fails — alert shows the wordedError text and re-reads branches on refresh', async () => {
    branchesRefusal = {
      status: 403,
      code: 'roleNotAllowed',
      message: 'RAW-branch-refused',
    };
    mount();
    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.roleNotAllowed']);
      expect(alert!.textContent).not.toContain('RAW-branch-refused');
    });

    // On refresh tick when branchFailed is true, readBranches() is called again
    requestRefresh();
    await vi.waitFor(() => {
      expect(branchReads.length).toBe(2);
    });
    expect(root.querySelector('[role="alert"]')!.textContent).toContain(
      th['error.roleNotAllowed'],
    );
  });

  it('Row 5: Branch or period change — reads without since; a refresh tick sends the held since unless the change failed', async () => {
    reportVersion = 7;
    mount();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(1);
      expect(root.textContent).toContain(formatMoney('100.00'));
    });
    expect(reportReads[0]!.has('since')).toBe(false);

    // Branch change reads without since
    setBranch('B9');
    await vi.waitFor(() => expect(reportReads.length).toBe(2));
    await settle();
    expect(reportReads[1]!.get('branch')).toBe('B9');
    expect(reportReads[1]!.has('since')).toBe(false);

    // Refresh tick sends the held since=7
    requestRefresh();
    await vi.waitFor(() => expect(reportReads.length).toBe(3));
    await settle();
    expect(reportReads[2]!.get('branch')).toBe('B9');
    expect(reportReads[2]!.get('since')).toBe('7');

    // Period change fails -> resets held since so next refresh reads without since
    reportsRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-period-fail',
    };
    setPeriod({ from: '2026-10-01', to: '2026-10-03' });
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(4);
      expect(root.querySelector('[role="alert"]')).not.toBeNull();
    });
    expect(reportReads[3]!.has('since')).toBe(false);

    reportsRefusal = null;
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(5);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });
    expect(reportReads[4]!.has('since')).toBe(false);
  });

  it('Edge 1: Branches recover but the report read fails on the same refresh — alert shows report error, not stale branch text', async () => {
    branchesRefusal = {
      status: 403,
      code: 'roleNotAllowed',
      message: 'RAW-branch-refused',
    };
    mount();
    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.roleNotAllowed']);
    });

    // Same refresh: branches recover, but report read fails with serverError
    branchesRefusal = null;
    reportsRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-report-fail',
    };
    requestRefresh();
    await vi.waitFor(() => {
      expect(branchReads.length).toBe(2);
      expect(reportReads.length).toBe(2);
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(th['error.serverError']);
      expect(alert!.textContent).not.toContain(th['error.roleNotAllowed']);
    });

    // Next refresh: report read answers changed:false, and because branches already recovered, alert clears
    reportsRefusal = null;
    forceUnchanged = true;
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(3);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });
  });

  it('Edge 2: The figures do not flash empty on a refresh that only answers changed:false after a failure', async () => {
    mount();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(1);
      expect(root.textContent).toContain(formatMoney('100.00'));
    });

    reportsRefusal = {
      status: 500,
      code: 'serverError',
      message: 'RAW-refresh-fail',
    };
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(2);
      expect(root.querySelector('[role="alert"]')).not.toBeNull();
    });

    let sawEmptyReceived = false;
    const observer = new MutationObserver(() => {
      const panelText = root.querySelector('#workspace-reports-0')?.textContent ?? '';
      if (!panelText.includes(formatMoney('100.00'))) {
        sawEmptyReceived = true;
      }
    });
    observer.observe(root, { childList: true, subtree: true, characterData: true });

    reportsRefusal = null;
    forceUnchanged = true;
    requestRefresh();
    await vi.waitFor(() => {
      expect(reportReads.length).toBe(3);
      expect(root.querySelector('[role="alert"]')).toBeNull();
    });
    observer.disconnect();

    expect(sawEmptyReceived).toBe(false);
    expect(root.querySelector('#workspace-reports-0')!.textContent).toContain(
      formatMoney('100.00'),
    );
  });
});
