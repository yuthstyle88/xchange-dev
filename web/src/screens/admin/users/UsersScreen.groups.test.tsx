import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import {
  makeAdminPermissionMatrixData,
  makeMatrixGroup,
} from '../../../shared/testing/fixtures/admin_permission_matrix';
import {
  makeAdminUserRole,
  makeAdminUserRow,
} from '../../../shared/testing/fixtures/admin_users';
import { AVAILABLE_ROLES, UsersScreen } from './UsersScreen';

const disposers: Array<() => void> = [];

const B01 = 'XC-B01';

const BRANCHES_DATA = {
  branches: [
    { id: B01, name: 'Silom Branch' },
  ],
};

const SEED_USERS = [
  makeAdminUserRow({
    userId: 'u-1',
    name: 'Somchai Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      makeAdminUserRole({
        role: 'TELLER',
        branch: B01,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      }),
    ],
  }),
];

const MATRIX_DATA = makeAdminPermissionMatrixData({
  version: 1,
  modules: ['DASHBOARD', 'FINANCE', 'USER_MANAGEMENT'],
  permissions: [],
  groups: [
    makeMatrixGroup({
      code: 'TELLER',
      name: 'TELLER',
      base: 'TELLER',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'BRANCH_MANAGER',
      name: 'BRANCH_MANAGER',
      base: 'BRANCH_MANAGER',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'RATE_MANAGER',
      name: 'RATE_MANAGER',
      base: 'RATE_MANAGER',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'FINANCE',
      name: 'FINANCE',
      base: 'FINANCE',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'VERIFICATION_REVIEWER',
      name: 'VERIFICATION_REVIEWER',
      base: 'VERIFICATION_REVIEWER',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'ADMIN',
      name: 'ADMIN',
      base: 'ADMIN',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'EXECUTIVE',
      name: 'EXECUTIVE',
      base: 'EXECUTIVE',
      builtIn: true,
      editable: false,
      permissions: [],
      levels: {},
    }),
    makeMatrixGroup({
      code: 'NIGHT_TELLER',
      name: 'Night teller',
      base: 'TELLER',
      builtIn: false,
      editable: true,
      permissions: [],
      levels: {},
    }),
  ],
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let calls: Array<{ url: string; method: string; body: any; headers: Headers }>;
let matrixResponse: Response | null = null;
let delayMatrix = false;
let matrixResolve: (() => void) | null = null;

async function serve(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const method = (init?.method ?? 'GET').toUpperCase();
  const headers = new Headers(init?.headers);
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  calls.push({ url, method, body, headers });

  const u = new URL(url, 'http://localhost');
  if (u.pathname === '/api/v3/views/admin_branches') {
    return json({ version: 1, changed: true, data: BRANCHES_DATA });
  }
  if (u.pathname === '/api/v3/views/admin_users') {
    return json({ version: 1, changed: true, data: SEED_USERS });
  }
  if (u.pathname === '/api/v3/views/admin_permission_matrix') {
    if (delayMatrix) {
      await new Promise<void>((resolve) => {
        matrixResolve = resolve;
      });
    }
    return matrixResponse ?? json({ version: 1, changed: true, data: MATRIX_DATA });
  }
  if (u.pathname === '/api/users/grant' && method === 'POST') {
    return json({
      userId: body.userId,
      status: 'ACTIVE',
      version: 2,
    });
  }
  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function mount(branch = B01) {
  const [b] = createSignal(branch);
  const [shown] = createSignal(true);
  const period = () => ({ from: '2026-10-01', to: '2026-10-31' });

  const container = document.createElement('div');
  document.body.appendChild(container);

  const dispose = render(
    () => <UsersScreen branch={b} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-11T05:00:00Z') });
  calls = [];
  matrixResponse = null;
  delayMatrix = false;
  matrixResolve = null;
  document.body.innerHTML = '';
  vi.stubGlobal('fetch', vi.fn(serve));
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('RBAC §11 W3: UsersScreen grant dialog group list', () => {
  // §9 Row 1
  it('matrix answers built-ins + a custom group NIGHT_TELLER named "Night teller": lists every group in order, custom last, value="NIGHT_TELLER" and shows "Night teller"', async () => {
    const { container } = mount();
    await flush();

    const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    const options = Array.from(roleSelect.options);

    // Lists every group in the view's order (8 groups)
    expect(options.map((o) => o.value)).toEqual([
      'TELLER',
      'BRANCH_MANAGER',
      'RATE_MANAGER',
      'FINANCE',
      'VERIFICATION_REVIEWER',
      'ADMIN',
      'EXECUTIVE',
      'NIGHT_TELLER',
    ]);

    // Custom group is last
    const customOpt = options[options.length - 1];
    expect(customOpt?.value).toBe('NIGHT_TELLER');
    expect(customOpt?.textContent).toContain('Night teller');
    expect(customOpt?.textContent).toContain('NIGHT_TELLER');
  });

  // §9 Row 2
  it('a group whose name equals its code: the option shows the code once', async () => {
    const { container } = mount();
    await flush();

    const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    const tellerOpt = Array.from(roleSelect.options).find((o) => o.value === 'TELLER');
    expect(tellerOpt).toBeDefined();
    expect(tellerOpt?.textContent).toBe('TELLER');
  });

  // §9 Row 3
  it('grant with the custom group selected: POST /api/users/grant body has role: "NIGHT_TELLER"', async () => {
    const { container } = mount();
    await flush();

    const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    roleSelect.value = 'NIGHT_TELLER';
    roleSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const grantCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users/grant'),
    );
    expect(grantCalls.length).toBe(1);
    expect(grantCalls[0]?.body).toEqual({
      userId: 'u-1',
      role: 'NIGHT_TELLER',
      branch: B01,
    });
  });

  // §9 Row 4
  it('dialog opens: TELLER is still the default selection', async () => {
    const { container } = mount();
    await flush();

    const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    expect(roleSelect.value).toBe('TELLER');
  });

  // §9 Row 5
  it('the matrix view fails or is still loading: lists the seven AVAILABLE_ROLES so a grant is still possible', async () => {
    // 1. While still loading
    delayMatrix = true;
    const { container } = mount();
    await flush();

    const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    // Flush without resolving matrix yet
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    let options = Array.from(roleSelect.options);
    expect(options.map((o) => o.value)).toEqual([...AVAILABLE_ROLES]);

    // 2. When matrix view fails
    matrixResponse = json({ error: { code: 'serverError', message: 'fail' } }, 500);
    if (matrixResolve) matrixResolve();
    await flush();

    options = Array.from(roleSelect.options);
    expect(options.map((o) => o.value)).toEqual([...AVAILABLE_ROLES]);

    // Grant is still possible with one of the available roles
    roleSelect.value = 'ADMIN';
    roleSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const grantCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users/grant'),
    );
    expect(grantCalls.length).toBe(1);
    expect(grantCalls[0]?.body.role).toBe('ADMIN');
  });

  // §9 Row 6
  it('create-user dialog: still lists AVAILABLE_ROLES only', async () => {
    const { container } = mount();
    await flush();

    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn.users-btn-primary')!;
    createBtn.click();
    await flush();

    const createRoleSelect = container.querySelector<HTMLSelectElement>('select[name="role"]')!;
    expect(createRoleSelect).not.toBeNull();
    const options = Array.from(createRoleSelect.options);
    expect(options.map((o) => o.value)).toEqual([...AVAILABLE_ROLES]);
  });
});
