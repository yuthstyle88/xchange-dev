import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { AdminUserRow } from '../../../shared/api/generated/admin_users';
import * as table from './i18n';
import { UsersScreen } from './UsersScreen';

const disposers: Array<() => void> = [];

const BRANCH_A = 'XC-B01';
const BRANCH_B = 'XC-B09';
const BRANCH_C = 'XC-B12';

const BRANCHES_DATA = {
  branches: [
    { id: BRANCH_A, name: 'Silom Branch' },
    { id: BRANCH_B, name: 'Siam Branch' },
  ],
};

const USERS_A: AdminUserRow[] = [
  {
    userId: 'u-branch-a',
    name: 'Arun BranchA',
    status: 'ACTIVE',
    version: 1,
    roles: [
      {
        role: 'TELLER',
        branch: BRANCH_A,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      },
    ],
  },
];

const USERS_B: AdminUserRow[] = [
  {
    userId: 'u-branch-b',
    name: 'Boonmee BranchB',
    status: 'ACTIVE',
    version: 1,
    roles: [
      {
        role: 'BRANCH_MANAGER',
        branch: BRANCH_B,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      },
    ],
  },
];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T = Response>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

let branchesHandler: ((branch: string | null) => Promise<Response> | Response) | null = null;
let usersHandler: ((branch: string | null) => Promise<Response> | Response) | null = null;
let postHandler: ((pathname: string, body: unknown) => Promise<Response> | Response) | null = null;

async function serve(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  const u = new URL(url, 'http://localhost');

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
    const branch = u.searchParams.get('branch');
    if (branchesHandler) return branchesHandler(branch);
    return json({ version: 1, changed: true, data: BRANCHES_DATA });
  }

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
    const branch = u.searchParams.get('branch');
    if (usersHandler) return usersHandler(branch);
    return json({ version: 1, changed: true, data: branch === BRANCH_B ? USERS_B : USERS_A });
  }

  if (method === 'POST') {
    if (postHandler) return postHandler(u.pathname, body);
    return json({ ok: true });
  }

  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
  }
}

function mount(initialBranch = BRANCH_A) {
  const [branch, setBranch] = createSignal(initialBranch);
  const [shown] = createSignal(true);
  const period = () => ({ from: '2026-10-01', to: '2026-10-31' });

  const container = document.createElement('div');
  document.body.appendChild(container);

  const dispose = render(
    () => <UsersScreen branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container, setBranch };
}

describe('Admin Users branch switch ordering and loading guard (#2277)', () => {
  beforeEach(() => {
    branchesHandler = null;
    usersHandler = null;
    postHandler = null;
    document.body.innerHTML = '';
    vi.stubGlobal('fetch', vi.fn(serve));
    setLang('th');
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()?.();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  it("row 1: A then B, A answers last -> B's users are shown", async () => {
    const reqA = deferred<Response>();
    const reqB = deferred<Response>();

    usersHandler = (branch) => {
      if (branch === BRANCH_A) return reqA.promise;
      if (branch === BRANCH_B) return reqB.promise;
      return json({ version: 1, changed: true, data: [] });
    };

    const { container, setBranch } = mount(BRANCH_A);
    await flush();

    // Switch to B while A is still in flight
    setBranch(BRANCH_B);
    await flush();

    // B answers first with B's users
    reqB.resolve(json({ version: 1, changed: true, data: USERS_B }));
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Boonmee BranchB');
    });

    // A answers last with A's users -> must be ignored
    reqA.resolve(json({ version: 1, changed: true, data: USERS_A }));
    await flush();

    expect(container.textContent).toContain('Boonmee BranchB');
    expect(container.textContent).not.toContain('Arun BranchA');
  });

  it("row 2: B still in flight -> The loading text, not 'no users'", async () => {
    const reqA = deferred<Response>();
    const reqB = deferred<Response>();

    usersHandler = (branch) => {
      if (branch === BRANCH_A) return reqA.promise;
      if (branch === BRANCH_B) return reqB.promise;
      return json({ version: 1, changed: true, data: [] });
    };

    const { container, setBranch } = mount(BRANCH_A);
    await flush();

    // Switch to B while A is still in flight
    setBranch(BRANCH_B);
    await flush();

    // A's read finishes (with empty or any data) while B's read is still in flight
    reqA.resolve(json({ version: 1, changed: true, data: [] }));
    await flush();

    // Because B is still in flight, the screen must show the loading text, not 'no users'
    const emptyEl = container.querySelector('.users-empty');
    expect(emptyEl).not.toBeNull();
    expect(emptyEl?.textContent).toContain(table.th.loading);
    expect(emptyEl?.textContent).not.toContain(table.th.noUsers);

    // Once B finishes with empty list, 'no users' is shown
    reqB.resolve(json({ version: 1, changed: true, data: [] }));
    await vi.waitFor(() => {
      expect(container.querySelector('.users-empty')?.textContent).toContain(table.th.noUsers);
    });
  });

  it('row 3: An older read fails after a newer success -> No pageError', async () => {
    const usersReqA = deferred<Response>();
    const usersReqB = deferred<Response>();
    const branchesReqA = deferred<Response>();
    const branchesReqB = deferred<Response>();

    usersHandler = (branch) => (branch === BRANCH_A ? usersReqA.promise : usersReqB.promise);
    branchesHandler = (branch) => (branch === BRANCH_A ? branchesReqA.promise : branchesReqB.promise);

    const { container, setBranch } = mount(BRANCH_A);
    await flush();

    // Switch to B
    setBranch(BRANCH_B);
    await flush();

    // Newer reads for B succeed
    branchesReqB.resolve(json({ version: 1, changed: true, data: BRANCHES_DATA }));
    usersReqB.resolve(json({ version: 1, changed: true, data: USERS_B }));
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Boonmee BranchB');
    });
    expect(container.querySelector('[role="alert"]')).toBeNull();

    // Older reads for A fail after B succeeded -> must not set pageError or clear B's users
    branchesReqA.resolve(refusal(500, 'serverError', 'Stale branch failure'));
    usersReqA.resolve(refusal(500, 'serverError', 'Stale users failure'));
    await flush();

    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.textContent).toContain('Boonmee BranchB');
  });

  it('row 4: A failed read after a branch switch -> Does not leave the old data', async () => {
    let callA = 0;
    const staleRefreshA = deferred<Response>();

    usersHandler = (branch) => {
      if (branch === BRANCH_A) {
        callA += 1;
        if (callA === 1) {
          return json({ version: 1, changed: true, data: USERS_A });
        }
        return staleRefreshA.promise;
      }
      return refusal(500, 'serverError', 'Branch B failed');
    };

    const { container, setBranch } = mount(BRANCH_A);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Arun BranchA');
    });

    // Start a refresh on A (stays in flight), then switch to B where the read fails
    const refreshBtn = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (b) => b.textContent?.trim() === table.th.refresh,
    );
    expect(refreshBtn).toBeDefined();
    refreshBtn!.click();
    await flush();

    setBranch(BRANCH_B);
    await vi.waitFor(() => {
      const alert = container.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });

    // Old data from A is not left on screen, even if A's earlier refresh finishes after B failed
    expect(container.textContent).not.toContain('Arun BranchA');
    staleRefreshA.resolve(json({ version: 1, changed: true, data: USERS_A }));
    await flush();
    expect(container.textContent).not.toContain('Arun BranchA');
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
  });

  it('row 5: A failed branch list -> Keeps the old list and shows the error', async () => {
    const slowBranchA2 = deferred<Response>();

    branchesHandler = (branch) => {
      if (branch === BRANCH_A) {
        return json({ version: 1, changed: true, data: BRANCHES_DATA });
      }
      if (branch === BRANCH_C) {
        return slowBranchA2.promise;
      }
      return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    };

    const { container, setBranch } = mount(BRANCH_A);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Arun BranchA');
    });

    // Open create dialog to check branch options
    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn-primary');
    expect(createBtn).not.toBeNull();
    createBtn!.click();
    await flush();

    const branchSelect = container.querySelector<HTMLSelectElement>('#create-branch');
    expect(branchSelect?.options.length).toBe(3);

    // Switch through C (in-flight admin_branches) to B (fails admin_branches)
    setBranch(BRANCH_C);
    await flush();
    setBranch(BRANCH_B);
    await vi.waitFor(() => {
      const alert = container.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });

    // Even when C's stale branch list answers later with an empty list, the old list is kept
    slowBranchA2.resolve(json({ version: 2, changed: true, data: { branches: [] } }));
    await flush();

    expect(branchSelect?.options.length, 'Old branch list is kept').toBe(3);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
  });

  it('edge case (§9): The re-read after create, grant or disable goes through the same guard', async () => {
    let readCount = 0;
    const staleRefresh = deferred<Response>();

    usersHandler = () => {
      readCount += 1;
      if (readCount === 1) {
        return json({ version: 1, changed: true, data: USERS_A });
      }
      if (readCount === 2) {
        // Stale refresh started before disable completes
        return staleRefresh.promise;
      }
      // Re-read triggered after disable shows user as DISABLED
      return json({
        version: 2,
        changed: true,
        data: [{ ...USERS_A[0]!, status: 'DISABLED' }],
      });
    };

    const { container } = mount(BRANCH_A);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Arun BranchA');
      expect(container.textContent).toContain(table.th.statusActive);
    });

    // Start a slow refresh (read #2)
    const refreshBtn = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (b) => b.textContent?.trim() === table.th.refresh,
    );
    refreshBtn!.click();
    await flush();

    // Now click disable on the user -> triggers POST /api/users/disable then loadUsers() (read #3)
    const disableBtn = container.querySelector<HTMLButtonElement>('.users-btn-disable');
    expect(disableBtn).not.toBeNull();
    disableBtn!.click();

    await vi.waitFor(() => {
      expect(container.textContent).toContain(table.th.statusDisabled);
    });

    // Now the older read #2 resolves with stale ACTIVE status -> must be ignored
    staleRefresh.resolve(json({ version: 1, changed: true, data: USERS_A }));
    await flush();

    expect(container.textContent).toContain(table.th.statusDisabled);
    expect(container.querySelector('.users-status-active')).toBeNull();
  });
});
