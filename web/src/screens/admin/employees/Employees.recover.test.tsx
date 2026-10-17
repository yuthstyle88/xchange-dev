import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import type { AdminBranchItem } from '../../../shared/api/generated/admin_branches';
import type { AdminEmployeeRow } from '../../../shared/api/generated/admin_employees';
import type { AdminOperationsData } from '../../../shared/api/generated/admin_operations';
import { Employees } from './Employees';
import { Dialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import { Toast, toast } from '../parts/toast';

const SAMPLE_EMPLOYEES: AdminEmployeeRow[] = [
  {
    id: 'emp-1',
    name: 'Somchai Prasert',
    username: 'somchai',
    role: 'TELLER',
    branchId: 'MAIN',
    active: true,
    mustChange: false,
  },
];

const SAMPLE_BRANCHES: AdminBranchItem[] = [
  { id: 'MAIN', name: 'Main Branch' },
  { id: 'B2', name: 'Second Branch' },
];

const SAMPLE_OPERATIONS: AdminOperationsData = {
  entity: {
    company: '108 Exchange Co., Ltd.',
    site: 'Headquarters',
  },
};

describe('Admin Employees branch-read recovery on refresh (#2179)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let employeesResponse: { status: number; body: unknown } | Error | null = null;
  let operationsResponse: { status: number; body: unknown } | Error | null = null;
  let branchesResponse: { status: number; body: unknown } | Error | null = null;
  let employeesReads = 0;
  let operationsReads: { branch: string | null; since: string | null }[] = [];
  let branchesReads = 0;

  beforeEach(() => {
    setLang('th');
    employeesResponse = null;
    operationsResponse = null;
    branchesResponse = null;
    employeesReads = 0;
    operationsReads = [];
    branchesReads = 0;
    root = document.createElement('div');
    document.body.appendChild(root);
    toast('');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const raw = String(input);
        const url = new URL(raw, 'http://desk.test');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.pathname.includes('/api/v3/views/admin_employees') && method === 'GET') {
          employeesReads++;
          if (employeesResponse) {
            if (employeesResponse instanceof Error) {
              return Promise.reject(employeesResponse);
            }
            return Promise.resolve(
              new Response(JSON.stringify(employeesResponse.body), {
                status: employeesResponse.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { employees: SAMPLE_EMPLOYEES },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.pathname.includes('/api/v3/views/admin_operations') && method === 'GET') {
          operationsReads.push({
            branch: url.searchParams.get('branch'),
            since: url.searchParams.get('since'),
          });
          if (operationsResponse) {
            if (operationsResponse instanceof Error) {
              return Promise.reject(operationsResponse);
            }
            return Promise.resolve(
              new Response(JSON.stringify(operationsResponse.body), {
                status: operationsResponse.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: SAMPLE_OPERATIONS,
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.pathname.includes('/api/v3/views/admin_branches') && method === 'GET') {
          branchesReads++;
          if (branchesResponse) {
            if (branchesResponse instanceof Error) {
              return Promise.reject(branchesResponse);
            }
            return Promise.resolve(
              new Response(JSON.stringify(branchesResponse.body), {
                status: branchesResponse.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { branches: SAMPLE_BRANCHES },
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

  function mount(): { root: HTMLElement; setBranch: (b: string) => void } {
    const [branch, setBranch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <Employees branch={branch} period={period} shown={shown} />
            <Dialog />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
    return { root, setBranch };
  }

  it('row 1: Branches fail, then Refresh with branches OK -> Alert gone, staff table shown with branch names', async () => {
    branchesResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-BRANCHES-500-FAILURE',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });
    expect(root.querySelector('table')).toBeNull();

    // Now branches succeed on refresh
    branchesResponse = null;
    requestRefresh();

    await vi.waitFor(() => {
      expect(root.querySelector('[role="alert"]')).toBeNull();
      const table = root.querySelector('table');
      expect(table).not.toBeNull();
      expect(table!.textContent).toContain('Somchai Prasert');
      expect(table!.textContent).toContain('Main Branch');
    });
    expect(branchesReads).toBe(2);
  });

  it('row 2: Branches fail -> Alert with the wordedError text', async () => {
    branchesResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-BRANCHES-500-FAILURE',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });
    expect(root.textContent).not.toContain('RAW-BRANCHES-500-FAILURE');
    expect(root.querySelector('table')).toBeNull();
  });

  it('row 3: Accounts or operations fail on refresh, then recover -> Unchanged; this already recovers', async () => {
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('table')).not.toBeNull();
      expect(root.textContent).toContain('Somchai Prasert');
    });

    // Accounts fail on refresh
    employeesResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-EMPLOYEES-500',
        },
      },
    };
    requestRefresh();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });
    expect(root.querySelector('table')).toBeNull();

    // Accounts recover on next refresh
    employeesResponse = null;
    requestRefresh();

    await vi.waitFor(() => {
      expect(root.querySelector('[role="alert"]')).toBeNull();
      expect(root.querySelector('table')).not.toBeNull();
    });

    // Operations fail on refresh
    operationsResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-OPERATIONS-500',
        },
      },
    };
    requestRefresh();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });
    expect(root.querySelector('table')).not.toBeNull();

    // Operations recover on next refresh
    operationsResponse = null;
    requestRefresh();

    await vi.waitFor(() => {
      expect(root.querySelector('[role="alert"]')).toBeNull();
      expect(root.querySelector('table')).not.toBeNull();
    });
  });

  it('row 4: A normal refresh with no earlier branch failure -> Re-reads employees and operations', async () => {
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('table')).not.toBeNull();
      expect(root.textContent).toContain('Main Branch');
    });
    expect(employeesReads).toBe(1);
    expect(operationsReads).toHaveLength(1);
    expect(branchesReads).toBe(1);

    requestRefresh();

    await vi.waitFor(() => {
      expect(employeesReads).toBe(2);
      expect(operationsReads).toHaveLength(2);
    });
    expect(operationsReads[1]).toEqual({ branch: 'MAIN', since: '1' });
    expect(branchesReads).toBe(1);
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.querySelector('table')).not.toBeNull();
  });

  it('edge case (§9): The branch read fails again on refresh -> the alert stays, with the new error text', async () => {
    branchesResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-BRANCHES-500-FIRST',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });
    expect(root.querySelector('table')).toBeNull();
    expect(branchesReads).toBe(1);

    // Branch read fails again on refresh with a different error
    branchesResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-BRANCHES-403-SECOND',
        },
      },
    };
    requestRefresh();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.roleNotAllowedHere']);
    });
    expect(root.textContent).not.toContain(sharedTh['error.serverError']);
    expect(root.textContent).not.toContain('RAW-BRANCHES-403-SECOND');
    expect(root.querySelector('table')).toBeNull();
    expect(branchesReads).toBe(2);
  });
});
