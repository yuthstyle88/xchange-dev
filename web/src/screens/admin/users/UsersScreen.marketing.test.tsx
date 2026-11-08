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
import { UsersScreen } from './UsersScreen';

// OL-7 row 86 (restated by claude/94389a6b on the sub-controller claude/1cc51615's decision, /tmp/ol7/ORDER-claude-19-b.md
// item 2): reset, grant, suspend and disable now sit in each row's "more" menu, rendered only while it is open. A lookup
// that does not find its button opens the first closed menu under `root` that holds it; nothing else in the test changes.
function withMore<T extends ParentNode>(root: T, sel: string): T {
  if (!root.querySelector(sel)) {
    for (const t of Array.from(root.querySelectorAll<HTMLButtonElement>('[data-users-menu="toggle"]'))) {
      if (t.getAttribute('aria-expanded') === 'true') continue;
      t.click();
      if (root.querySelector(sel)) break;
    }
  }
  return root;
}

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
    return matrixResponse ?? json({ version: 1, changed: true, data: MATRIX_DATA });
  }
  if (u.pathname === '/api/users' && method === 'POST') {
    return json({ temporaryPassword: 'temp-password-123' }, 201);
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

describe('UsersScreen: MARKETING role support (#5947)', () => {
  // M1: Open the create dialog -> select[name="role"] options are in order:
  // TELLER, BRANCH_MANAGER, RATE_MANAGER, FINANCE, VERIFICATION_REVIEWER, ADMIN, EXECUTIVE, MARKETING
  it('M1: open the create dialog: role options include MARKETING last, TELLER is default', async () => {
    const { container } = mount();
    await flush();

    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn.users-btn-primary')!;
    createBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="role"]')!;
    expect(roleSelect).not.toBeNull();
    expect(roleSelect.value).toBe('TELLER');

    const options = Array.from(roleSelect.options).map((o) => o.value);
    expect(options).toEqual([
      'TELLER',
      'BRANCH_MANAGER',
      'RATE_MANAGER',
      'FINANCE',
      'VERIFICATION_REVIEWER',
      'ADMIN',
      'EXECUTIVE',
      'MARKETING',
    ]);
  });

  // M2: Pick MARKETING, branch left on "organisation-wide", fill name and phone, submit ->
  // one POST /api/users whose body has roles: [{ role: 'MARKETING', branch: null }]
  it('M2: pick MARKETING, branch left on organisation-wide, submit creates user with null branch', async () => {
    const { container } = mount('');
    await flush();

    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn.users-btn-primary')!;
    createBtn.click();
    await flush();

    const nameInput = container.querySelector<HTMLInputElement>('input[name="name"]')!;
    nameInput.value = 'Alice Marketer';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const phoneInput = container.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '+66812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="role"]')!;
    roleSelect.value = 'MARKETING';
    roleSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const branchSelect = container.querySelector<HTMLSelectElement>('select[name="branch"]')!;
    expect(branchSelect.value).toBe('');

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const createCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users'),
    );
    expect(createCalls.length).toBe(1);
    expect(createCalls[0]?.body.roles).toEqual([
      { role: 'MARKETING', branch: null },
    ]);
  });

  // M3: Pick MARKETING and a branch -> body has roles: [{ role: 'MARKETING', branch: '<that branch id>' }]
  it('M3: pick MARKETING and a branch, submit creates user with that branch', async () => {
    const { container } = mount();
    await flush();

    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn.users-btn-primary')!;
    createBtn.click();
    await flush();

    const nameInput = container.querySelector<HTMLInputElement>('input[name="name"]')!;
    nameInput.value = 'Bob Marketer';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const phoneInput = container.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '+66812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="role"]')!;
    roleSelect.value = 'MARKETING';
    roleSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const branchSelect = container.querySelector<HTMLSelectElement>('select[name="branch"]')!;
    branchSelect.value = B01;
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const createCalls = calls.filter(
      (c) => c.method === 'POST' && c.url.endsWith('/api/users'),
    );
    expect(createCalls.length).toBe(1);
    expect(createCalls[0]?.body.roles).toEqual([
      { role: 'MARKETING', branch: B01 },
    ]);
  });

  // M4: Group matrix fails to load, open the grant dialog -> fallback role list includes MARKETING (last)
  it('M4: group matrix fails to load, open grant dialog: fallback includes MARKETING last', async () => {
    matrixResponse = json({ error: { code: 'serverError', message: 'matrix load failure' } }, 500);
    const { container } = mount();
    await flush();

    const grantBtn = withMore(container, '.users-btn-grant').querySelector<HTMLButtonElement>('.users-btn-grant')!;
    grantBtn.click();
    await flush();

    const roleSelect = container.querySelector<HTMLSelectElement>('select[name="grantRole"]')!;
    const options = Array.from(roleSelect.options).map((o) => o.value);
    expect(options).toEqual([
      'TELLER',
      'BRANCH_MANAGER',
      'RATE_MANAGER',
      'FINANCE',
      'VERIFICATION_REVIEWER',
      'ADMIN',
      'EXECUTIVE',
      'MARKETING',
    ]);
    expect(options[options.length - 1]).toBe('MARKETING');
  });
});
