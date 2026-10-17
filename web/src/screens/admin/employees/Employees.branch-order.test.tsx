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

const MAIN_EMPLOYEES: AdminEmployeeRow[] = [
  {
    id: 'emp-main',
    name: 'Somchai Main',
    username: 'somchai_main',
    role: 'TELLER',
    branchId: 'MAIN',
    active: true,
    mustChange: false,
  },
];

const B2_EMPLOYEES: AdminEmployeeRow[] = [
  {
    id: 'emp-b2',
    name: 'Nok Second',
    username: 'nok_b2',
    role: 'BRANCH_MANAGER',
    branchId: 'B2',
    active: true,
    mustChange: false,
  },
];

const SAMPLE_BRANCHES: AdminBranchItem[] = [
  { id: 'MAIN', name: 'Main Branch' },
  { id: 'B2', name: 'Second Branch' },
];

const MAIN_OPERATIONS: AdminOperationsData = {
  enabled: true,
  entity: {
    company: 'Main Co., Ltd.',
    site: 'Main HQ',
  },
};

const B2_OPERATIONS: AdminOperationsData = {
  enabled: true,
  entity: {
    company: 'Second Branch Co., Ltd.',
    site: 'Airport Desk',
  },
};

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flushAsync(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('Admin Employees branch switch read ordering (#2289)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let employeesReads: { branch: string | null }[] = [];
  let operationsReads: { branch: string | null; since: string | null }[] = [];
  let branchesReads = 0;

  let employeesHandler: ((branch: string | null, callIndex: number) => Promise<Response>) | null =
    null;
  let operationsHandler:
    | ((branch: string | null, since: string | null, callIndex: number) => Promise<Response>)
    | null = null;
  let branchesHandler: ((callIndex: number) => Promise<Response>) | null = null;

  beforeEach(() => {
    setLang('th');
    employeesReads = [];
    operationsReads = [];
    branchesReads = 0;
    employeesHandler = null;
    operationsHandler = null;
    branchesHandler = null;
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
          const branch = url.searchParams.get('branch');
          const idx = employeesReads.length;
          employeesReads.push({ branch });
          if (employeesHandler) {
            return employeesHandler(branch, idx);
          }
          const rows = branch === 'B2' ? B2_EMPLOYEES : MAIN_EMPLOYEES;
          return Promise.resolve(
            jsonResponse({
              version: 1,
              changed: true,
              data: { employees: rows },
            }),
          );
        }

        if (url.pathname.includes('/api/v3/views/admin_operations') && method === 'GET') {
          const branch = url.searchParams.get('branch');
          const since = url.searchParams.get('since');
          const idx = operationsReads.length;
          operationsReads.push({ branch, since });
          if (operationsHandler) {
            return operationsHandler(branch, since, idx);
          }
          const isB2 = branch === 'B2';
          return Promise.resolve(
            jsonResponse({
              version: isB2 ? 9 : 3,
              changed: true,
              data: isB2 ? B2_OPERATIONS : MAIN_OPERATIONS,
            }),
          );
        }

        if (url.pathname.includes('/api/v3/views/admin_branches') && method === 'GET') {
          const idx = branchesReads;
          branchesReads++;
          if (branchesHandler) {
            return branchesHandler(idx);
          }
          return Promise.resolve(
            jsonResponse({
              version: 1,
              changed: true,
              data: { branches: SAMPLE_BRANCHES },
            }),
          );
        }

        return Promise.resolve(jsonResponse({ version: 1, changed: false }));
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

  it("row 1: Branch A to B, A's employees answer last -> B's staff are shown", async () => {
    const slowMainEmployees = deferred<Response>();
    employeesHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowMainEmployees.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 2,
          changed: true,
          data: { employees: B2_EMPLOYEES },
        }),
      );
    };

    const { setBranch } = mount();
    expect(employeesReads).toEqual([{ branch: 'MAIN' }]);

    // Switch to B2 while MAIN's employee read is still in flight
    setBranch('B2');

    await vi.waitFor(() => {
      expect(root.textContent).toContain('Nok Second');
    });

    // MAIN's late employee response arrives after B2's
    slowMainEmployees.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: { employees: MAIN_EMPLOYEES },
      }),
    );
    await flushAsync();

    expect(root.textContent).toContain('Nok Second');
    expect(root.textContent).not.toContain('Somchai Main');
  });

  it("row 2: Branch A to B, A's operations answer last -> B's operations; B's version is used for the next refresh", async () => {
    const slowMainOps = deferred<Response>();
    operationsHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowMainOps.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 9,
          changed: true,
          data: B2_OPERATIONS,
        }),
      );
    };

    const { setBranch } = mount();
    expect(operationsReads).toEqual([{ branch: 'MAIN', since: null }]);

    // Switch to B2 while MAIN's operations read is still in flight
    setBranch('B2');

    await vi.waitFor(() => {
      expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
    });

    // MAIN's late operations answer arrives with version 3
    slowMainOps.resolve(
      jsonResponse({
        version: 3,
        changed: true,
        data: MAIN_OPERATIONS,
      }),
    );
    await flushAsync();

    expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
    expect(root.textContent).not.toContain('Main Co., Ltd. · Main HQ');

    // Next refresh on B2 uses B2's version (9), not MAIN's stale version (3),
    // and the branch switch + refresh made only 3 operations reads total
    const markBeforeRefresh = operationsReads.length;
    requestRefresh();
    await vi.waitFor(() => {
      expect(operationsReads.length).toBeGreaterThan(markBeforeRefresh);
    });
    expect(operationsReads.at(-1)).toEqual({ branch: 'B2', since: '9' });
    expect(operationsReads).toEqual([
      { branch: 'MAIN', since: null },
      { branch: 'B2', since: null },
      { branch: 'B2', since: '9' },
    ]);
  });

  it("row 3: An older read fails after a newer success -> No error; B's table stays", async () => {
    const slowMainEmployees = deferred<Response>();
    const slowMainOps = deferred<Response>();

    employeesHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowMainEmployees.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 2,
          changed: true,
          data: { employees: B2_EMPLOYEES },
        }),
      );
    };
    operationsHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowMainOps.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 9,
          changed: true,
          data: B2_OPERATIONS,
        }),
      );
    };

    const { setBranch } = mount();
    setBranch('B2');

    await vi.waitFor(() => {
      expect(root.textContent).toContain('Nok Second');
      expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
    });

    // Older MAIN reads fail after B2 has already succeeded
    slowMainEmployees.resolve(
      jsonResponse(
        { error: { code: 'serverError', message: 'STALE-EMP-500' } },
        500,
      ),
    );
    slowMainOps.resolve(
      jsonResponse(
        { error: { code: 'serverError', message: 'STALE-OPS-500' } },
        500,
      ),
    );
    await flushAsync();

    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.querySelector('table')).not.toBeNull();
    expect(root.textContent).toContain('Nok Second');
    expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
  });

  it('row 4: Refresh with no branch change -> Re-reads employees and operations with the held since (kept)', async () => {
    const { setBranch } = mount();

    await vi.waitFor(() => {
      expect(root.textContent).toContain('Somchai Main');
      expect(root.textContent).toContain('Main Co., Ltd. · Main HQ');
    });
    expect(employeesReads).toEqual([{ branch: 'MAIN' }]);
    expect(operationsReads).toEqual([{ branch: 'MAIN', since: null }]);

    // Switch to B2: triggers one employees read and one operations read
    setBranch('B2');
    await vi.waitFor(() => {
      expect(root.textContent).toContain('Nok Second');
      expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
    });
    expect(employeesReads).toEqual([{ branch: 'MAIN' }, { branch: 'B2' }]);
    expect(operationsReads).toEqual([
      { branch: 'MAIN', since: null },
      { branch: 'B2', since: null },
    ]);

    // Refresh with no branch change: re-reads employees and operations with held since ('9')
    requestRefresh();
    await vi.waitFor(() => {
      expect(employeesReads).toHaveLength(3);
      expect(operationsReads).toHaveLength(3);
    });
    expect(employeesReads[2]).toEqual({ branch: 'B2' });
    expect(operationsReads[2]).toEqual({ branch: 'B2', since: '9' });
  });

  it('row 5: Branch list fails, then Refresh -> Branches re-read (kept, Employees.recover.test.tsx)', async () => {
    const slowMainEmployees = deferred<Response>();
    let failBranches = true;

    branchesHandler = () => {
      if (failBranches) {
        return Promise.resolve(
          jsonResponse(
            { error: { code: 'serverError', message: 'BRANCHES-500' } },
            500,
          ),
        );
      }
      return Promise.resolve(
        jsonResponse({
          version: 1,
          changed: true,
          data: { branches: SAMPLE_BRANCHES },
        }),
      );
    };

    employeesHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowMainEmployees.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 2,
          changed: true,
          data: { employees: B2_EMPLOYEES },
        }),
      );
    };

    const { setBranch } = mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });
    expect(branchesReads).toBe(1);

    // Switch to B2 while MAIN's employee read is still in flight
    setBranch('B2');
    await vi.waitFor(() => {
      expect(employeesReads).toHaveLength(2);
    });

    // Now branches succeed on Refresh
    failBranches = false;
    requestRefresh();

    await vi.waitFor(() => {
      expect(root.querySelector('[role="alert"]')).toBeNull();
      const table = root.querySelector('table');
      expect(table).not.toBeNull();
      expect(table!.textContent).toContain('Nok Second');
      expect(table!.textContent).toContain('Second Branch');
    });

    // MAIN's slow initial employee read finishes after Refresh has recovered the table
    slowMainEmployees.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: { employees: MAIN_EMPLOYEES },
      }),
    );
    await flushAsync();

    expect(root.textContent).toContain('Nok Second');
    expect(root.textContent).not.toContain('Somchai Main');
    expect(branchesReads).toBe(2);
  });

  it("edge case (§9): A branch change while a refresh is in flight -> the refresh's answers are dropped", async () => {
    const { setBranch } = mount();

    await vi.waitFor(() => {
      expect(root.textContent).toContain('Somchai Main');
      expect(root.textContent).toContain('Main Co., Ltd. · Main HQ');
    });

    const slowRefreshEmployees = deferred<Response>();
    const slowRefreshOps = deferred<Response>();

    employeesHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowRefreshEmployees.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 2,
          changed: true,
          data: { employees: B2_EMPLOYEES },
        }),
      );
    };
    operationsHandler = (branch) => {
      if (branch === 'MAIN') {
        return slowRefreshOps.promise;
      }
      return Promise.resolve(
        jsonResponse({
          version: 9,
          changed: true,
          data: B2_OPERATIONS,
        }),
      );
    };

    // Start a refresh on MAIN
    requestRefresh();
    expect(employeesReads).toHaveLength(2);
    expect(operationsReads).toHaveLength(2);
    expect(operationsReads[1]).toEqual({ branch: 'MAIN', since: '3' });

    // Switch to B2 while MAIN's refresh is still in flight
    setBranch('B2');

    await vi.waitFor(() => {
      expect(root.textContent).toContain('Nok Second');
      expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
    });

    // MAIN's in-flight refresh answers arrive late
    slowRefreshEmployees.resolve(
      jsonResponse({
        version: 4,
        changed: true,
        data: { employees: MAIN_EMPLOYEES },
      }),
    );
    slowRefreshOps.resolve(
      jsonResponse({
        version: 4,
        changed: true,
        data: MAIN_OPERATIONS,
      }),
    );
    await flushAsync();

    expect(root.textContent).toContain('Nok Second');
    expect(root.textContent).not.toContain('Somchai Main');
    expect(root.textContent).toContain('Second Branch Co., Ltd. · Airport Desk');
    expect(root.textContent).not.toContain('Main Co., Ltd. · Main HQ');

    // Subsequent refresh on B2 still uses B2's cursor ('9'), not MAIN's refresh cursor ('4')
    requestRefresh();
    await vi.waitFor(() => {
      expect(operationsReads).toHaveLength(4);
    });
    expect(operationsReads[3]).toEqual({ branch: 'B2', since: '9' });
  });
});
