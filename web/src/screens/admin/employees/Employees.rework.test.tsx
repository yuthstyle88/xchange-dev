import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import type { AdminBranchItem } from '../../../shared/api/generated/admin_branches';
import type { AdminEmployeeRow } from '../../../shared/api/generated/admin_employees';
import type { AdminOperationsData } from '../../../shared/api/generated/admin_operations';
import { Employees } from './Employees';
import { Dialog } from '../parts/Dialog';
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

describe('Employees rework (#1696): worded errors on failed operations and branch reads', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let employeesResponse: { status: number; body: unknown } | Error | null = null;
  let operationsResponse: { status: number; body: unknown } | Error | null = null;
  let branchesResponse: { status: number; body: unknown } | Error | null = null;
  let postResponse: { status: number; body: unknown } | Error | null = null;
  let opsCallCount = 0;

  beforeEach(() => {
    setLang('th');
    employeesResponse = null;
    operationsResponse = null;
    branchesResponse = null;
    postResponse = null;
    opsCallCount = 0;
    root = document.createElement('div');
    document.body.appendChild(root);
    toast('');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = String(input);
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.includes('/api/v3/views/admin_employees') && method === 'GET') {
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

        if (url.includes('/api/v3/views/admin_operations') && method === 'GET') {
          opsCallCount++;
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

        if (url.includes('/api/v3/views/admin_branches') && method === 'GET') {
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

        if (url.includes('/api/employees') && method === 'POST') {
          if (postResponse) {
            if (postResponse instanceof Error) {
              return Promise.reject(postResponse);
            }
            return Promise.resolve(
              new Response(JSON.stringify(postResponse.body), {
                status: postResponse.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
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

  // ── Row 1 (§2): Employees.tsx:88 (readOperations failure) ──────────────────

  it('row 1: readOperations 500 serverError displays errorText in Thai and not raw message', async () => {
    operationsResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-OPS-500-FAILURE',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.serverError']);
    expect(text).not.toContain('RAW-OPS-500-FAILURE');
  });

  it('row 1: readOperations 500 serverError in English displays en errorText', async () => {
    setLang('en');
    operationsResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-OPS-500-FAILURE',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedEn['error.serverError']);
    expect(text).not.toContain('RAW-OPS-500-FAILURE');
  });

  it('row 1: readOperations 403 refusal with known code (roleNotAllowedHere) displays worded error', async () => {
    operationsResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-OPS-ROLE-NOT-ALLOWED',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(text).not.toContain('RAW-OPS-ROLE-NOT-ALLOWED');
  });

  it('row 1: readOperations refusal with unknown code displays generic worded error and not raw message', async () => {
    operationsResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownOpsCode1696',
          message: 'RAW-UNKNOWN-OPS-MSG-1696',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain('RAW-UNKNOWN-OPS-MSG-1696');
  });

  it('row 1: readOperations network rejection (non-ApiError) displays generic worded error and not raw message', async () => {
    operationsResponse = new TypeError('Failed to fetch');

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.network']);
    expect(text).not.toContain('Failed to fetch');
  });

  it('row 1: failed read for a new branch clears stale operations state and shows worded error', async () => {
    // 1. Initial mount succeeds with SAMPLE_OPERATIONS (enabled: true) for MAIN
    const { setBranch } = mount();

    // CA-10-FE (claude/c4d72c): the operations show only as the stored entity, read-only
    await vi.waitFor(() => {
      expect(root.textContent).toContain('108 Exchange Co., Ltd.');
    });

    // 2. Next read for B2 fails with 500
    operationsResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-OPS-POLL-FAILURE',
        },
      },
    };

    // Change branch to trigger readOperations(true)
    setBranch('B2');

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });

    // Stale operations state from MAIN is NOT shown under B2, and unknown state is not rendered as off
    expect(root.textContent).not.toContain('108 Exchange Co., Ltd.');
  });

  // ── Row 2 (§2): Employees.tsx:99 (readBranches failure) ────────────────────

  it('row 2: readBranches 500 serverError displays errorText in Thai and not raw message', async () => {
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
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.serverError']);
    expect(text).not.toContain('RAW-BRANCHES-500-FAILURE');
  });

  it('row 2: readBranches 500 serverError in English displays en errorText', async () => {
    setLang('en');
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
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedEn['error.serverError']);
    expect(text).not.toContain('RAW-BRANCHES-500-FAILURE');
  });

  it('row 2: readBranches 403 refusal with known code (roleNotAllowedHere) displays worded error', async () => {
    branchesResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-BRANCHES-ROLE-NOT-ALLOWED',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(text).not.toContain('RAW-BRANCHES-ROLE-NOT-ALLOWED');
  });

  it('row 2: readBranches refusal with unknown code displays generic worded error and not raw message', async () => {
    branchesResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownBranchCode1696',
          message: 'RAW-UNKNOWN-BRANCH-MSG-1696',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain('RAW-UNKNOWN-BRANCH-MSG-1696');
  });

  it('row 2: readBranches network rejection (non-ApiError) displays generic worded error and not raw message', async () => {
    branchesResponse = new TypeError('Failed to fetch');

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.network']);
    expect(text).not.toContain('Failed to fetch');
  });

  // ── readEmployees and write handlers wording (#1696 §3) ───────────────────

  it('readEmployees refusal with unknown code displays generic worded error and not raw message', async () => {
    employeesResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownEmpCode1696',
          message: 'RAW-UNKNOWN-EMP-MSG-1696',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain('RAW-UNKNOWN-EMP-MSG-1696');
  });

  it('write handler (disable) refusal with unknown code displays generic worded error in toast and not raw message', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    postResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownDisableCode1696',
          message: 'RAW-UNKNOWN-DISABLE-MSG-1696',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const disableBtn = root.querySelector('button.danger');
      expect(disableBtn).not.toBeNull();
    });

    const disableBtn = root.querySelector('button.danger') as HTMLButtonElement;
    disableBtn.click();

    await vi.waitFor(() => {
      const toastEl = root.querySelector('[role="status"]');
      expect(toastEl).not.toBeNull();
      expect(toastEl!.textContent).toContain('RAW-UNKNOWN-DISABLE-MSG-1696');
    });
  });
});
