import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { AdminUserRow } from '../../../shared/api/generated/admin_users';
import { UsersScreen } from './UsersScreen';
import { menu } from './menu';

const disposers: Array<() => void> = [];

const B01 = 'XC-B01';

const BRANCHES_DATA = {
  branches: [
    { id: B01, name: 'Silom Branch' },
  ],
};

const SEED_USERS: AdminUserRow[] = [
  {
    userId: 'u-1',
    name: 'Somchai Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      {
        role: 'TELLER',
        branch: B01,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      },
    ],
  },
  {
    userId: 'u-2',
    name: 'Somsak Invited',
    status: 'INVITED',
    version: 1,
    roles: [
      {
        role: 'FINANCE',
        branch: B01,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      },
    ],
  },
  {
    userId: 'u-3',
    name: 'Somying Disabled',
    status: 'DISABLED',
    version: 2,
    roles: [
      {
        role: 'TELLER',
        branch: B01,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: '2026-10-05T00:00:00Z',
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

let calls: Array<{ url: string; method: string; body: any; headers: Headers }>;
let usersData: AdminUserRow[] = SEED_USERS;

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
    return json({ version: 1, changed: true, data: usersData });
  }
  if (u.pathname === '/api/users' && method === 'POST') {
    if (body?.phone === '+66899999999') {
      return json(
        { error: { code: 'phoneTaken', message: 'เบอร์โทรศัพท์นี้ถูกใช้งานแล้ว' } },
        409,
      );
    }
    return json({
      userId: body.userId ?? 'u-new',
      status: 'ACTIVE',
      version: 1,
    });
  }
  if (u.pathname === '/api/users/grant' && method === 'POST') {
    return json({
      userId: body.userId,
      status: 'ACTIVE',
      version: 2,
    });
  }
  if (u.pathname === '/api/users/revoke' && method === 'POST') {
    return json({
      userId: body.userId,
      status: 'ACTIVE',
      version: 3,
    });
  }
  if (u.pathname === '/api/users/disable' && method === 'POST') {
    return json({
      userId: body.userId,
      status: 'DISABLED',
      version: 4,
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
  usersData = [...SEED_USERS];
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

describe('A05 UsersScreen', () => {
  it("pins its menu record: id 'users', group 'data', order 15, roles ['ADMIN']", () => {
    expect(menu.id).toBe('users');
    expect(menu.group).toBe('data');
    expect(menu.order).toBe(15);
    expect(menu.roles).toEqual(['ADMIN']);
    expect(menu.roles).not.toBe('all');
    for (const nonAdmin of ['TELLER', 'BRANCH_MANAGER', 'FINANCE', 'EXECUTIVE', 'MANAGER']) {
      expect((menu.roles as readonly string[]).includes(nonAdmin)).toBe(false);
    }
  });

  it('an INVITED row offers activate; an ACTIVE row does not', async () => {
    const { container } = mount();
    await flush();

    const rows = container.querySelectorAll('tbody tr');
    expect(rows.length).toBe(3);

    // Row 1: Active user
    const row1Text = rows[0]?.textContent ?? '';
    expect(row1Text).toContain('Somchai Active');
    const row1ActivateBtn = rows[0]?.querySelector('.users-btn-activate');
    expect(row1ActivateBtn, 'ACTIVE row must not offer activate').toBeNull();
    const row1GrantBtn = rows[0]?.querySelector('.users-btn-grant');
    expect(row1GrantBtn, 'ACTIVE row offers grant').not.toBeNull();

    // Row 2: Invited user
    const row2Text = rows[1]?.textContent ?? '';
    expect(row2Text).toContain('Somsak Invited');
    const row2ActivateBtn = rows[1]?.querySelector('.users-btn-activate');
    expect(row2ActivateBtn, 'INVITED row must offer activate').not.toBeNull();
  });

  it('the create dialog sends userCreate with phone, password and roles, and clears the password field after the call', async () => {
    const { container } = mount();
    await flush();

    // Click "Create User"
    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn-primary')!;
    createBtn.click();
    await flush();

    // Fill form
    const nameInput = container.querySelector<HTMLInputElement>('input[name="name"]')!;
    nameInput.value = 'New Staff';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const phoneInput = container.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '+66812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const passwordInput = container.querySelector<HTMLInputElement>('input[name="password"]')!;
    passwordInput.value = 'Secret123!';
    passwordInput.dispatchEvent(new Event('input', { bubbles: true }));

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="role"]')!;
    roleSelect.value = 'FINANCE';
    roleSelect.dispatchEvent(new Event('change', { bubbles: true }));

    // Submit form
    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    // Password field should be cleared immediately
    expect(passwordInput.value, 'password field must be cleared after call').toBe('');

    await flush();

    // Verify userCreate call
    const createCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users'),
    );
    expect(createCalls.length).toBe(1);
    expect(createCalls[0]?.body).toEqual({
      name: 'New Staff',
      username: null,
      phone: '+66812345678',
      email: null,
      password: 'Secret123!',
      roles: [{ role: 'FINANCE', branch: B01 }],
    });
  });

  it('a refused command error is shown in the form', async () => {
    const { container } = mount();
    await flush();

    // Click "Create User"
    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn-primary')!;
    createBtn.click();
    await flush();

    // Fill form with taken phone
    const nameInput = container.querySelector<HTMLInputElement>('input[name="name"]')!;
    nameInput.value = 'Duplicate Phone';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const phoneInput = container.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '+66899999999';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const passwordInput = container.querySelector<HTMLInputElement>('input[name="password"]')!;
    passwordInput.value = 'Secret123!';
    passwordInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Submit form
    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    expect(passwordInput.value, 'password field cleared on submit').toBe('');

    await flush();

    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl?.textContent).toContain('เบอร์โทรศัพท์นี้ถูกใช้งานแล้ว');
  });

  it('activate dialog retries userCreate with userId and clears password', async () => {
    const { container } = mount();
    await flush();

    // Click activate on invited user (Row 2)
    const activateBtn = container.querySelector<HTMLButtonElement>('.users-btn-activate')!;
    activateBtn.click();
    await flush();

    const phoneInput = container.querySelector<HTMLInputElement>('input[name="activatePhone"]')!;
    phoneInput.value = '+66811112222';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const passwordInput = container.querySelector<HTMLInputElement>('input[name="activatePassword"]')!;
    passwordInput.value = 'ActivatePass123';
    passwordInput.dispatchEvent(new Event('input', { bubbles: true }));

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    expect(passwordInput.value, 'password cleared on submit').toBe('');

    await flush();

    const activateCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users') && c.body.userId === 'u-2',
    );
    expect(activateCalls.length).toBe(1);
    expect(activateCalls[0]?.body.phone).toBe('+66811112222');
    expect(activateCalls[0]?.body.password).toBe('ActivatePass123');
    expect(activateCalls[0]?.body.userId).toBe('u-2');
  });

  it('activating a row whose roles is [] posts a body with roles: [], and no role the row does not have appears in the body', async () => {
    usersData = [
      {
        userId: 'u-noroles',
        name: 'No Roles User',
        status: 'INVITED',
        version: 1,
        roles: [],
      },
    ];
    const { container } = mount();
    await flush();

    const activateBtn = container.querySelector<HTMLButtonElement>('.users-btn-activate')!;
    activateBtn.click();
    await flush();

    const phoneInput = container.querySelector<HTMLInputElement>('input[name="activatePhone"]')!;
    phoneInput.value = '+66833334444';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const passwordInput = container.querySelector<HTMLInputElement>('input[name="activatePassword"]')!;
    passwordInput.value = 'PassNoRoles123';
    passwordInput.dispatchEvent(new Event('input', { bubbles: true }));

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    await flush();

    const activateCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users') && c.body.userId === 'u-noroles',
    );
    expect(activateCalls.length).toBe(1);
    expect(activateCalls[0]?.body.roles).toEqual([]);
    expect(JSON.stringify(activateCalls[0]?.body)).not.toContain('TELLER');
  });

  it('grant role sends userGrant command with idempotency key', async () => {
    const { container } = mount();
    await flush();

    // Click grant on Active user (Row 1)
    const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    roleSelect.value = 'ADMIN';
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
      role: 'ADMIN',
      branch: B01,
    });
    expect(grantCalls[0]?.headers.get('idempotency-key')).toBeTruthy();
  });

  it('revoke role sends userRevoke command with idempotency key', async () => {
    const { container } = mount();
    await flush();

    // Click revoke (the '×' button on the role tag)
    const revokeBtn = container.querySelector<HTMLButtonElement>('.users-btn-revoke')!;
    revokeBtn.click();
    await flush();

    const revokeCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users/revoke'),
    );
    expect(revokeCalls.length).toBe(1);
    expect(revokeCalls[0]?.body).toEqual({
      userId: 'u-1',
      role: 'TELLER',
      branch: B01,
    });
    expect(revokeCalls[0]?.headers.get('idempotency-key')).toBeTruthy();
  });

  it('disable user sends userDisable command with idempotency key', async () => {
    const { container } = mount();
    await flush();

    // Click disable on Active user (Row 1)
    const disableBtn = container.querySelector<HTMLButtonElement>('.users-btn-disable')!;
    disableBtn.click();
    await flush();

    const disableCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users/disable'),
    );
    expect(disableCalls.length).toBe(1);
    expect(disableCalls[0]?.body).toEqual({
      userId: 'u-1',
    });
    expect(disableCalls[0]?.headers.get('idempotency-key')).toBeTruthy();
  });
});
