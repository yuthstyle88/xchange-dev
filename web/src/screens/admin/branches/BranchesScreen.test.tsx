import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { BranchesScreen } from './BranchesScreen';
import { menu } from './menu';

const disposers: Array<() => void> = [];

const B01 = 'XC-B01';
const B09 = 'XC-B09';

const BRANCHES_DATA = {
  branches: [
    { id: B01, name: 'Silom Branch' },
    { id: B09, name: 'Siam Branch' },
  ],
};

const PROFILE_DATA = {
  branch: B01,
  name: 'Silom Branch',
  code: 'SILOM',
  address: '1 Silom Rd',
  timezone: 'Asia/Bangkok',
  openingHours: {
    mon: [{ open: '09:00', close: '17:00' }],
    tue: [{ open: '09:00', close: '17:00' }],
    wed: [{ open: '09:00', close: '17:00' }],
    thu: [{ open: '09:00', close: '17:00' }],
    fri: [{ open: '09:00', close: '17:00' }],
    sat: [],
    sun: [],
  },
  bookingEnabled: true,
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let calls: Array<{ url: string; method: string; body: any; headers: Headers }>;

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
  if (u.pathname === '/api/v3/views/admin_branches_profile') {
    const branch = u.searchParams.get('branch') ?? B01;
    return json({
      version: 1,
      changed: true,
      data: { ...PROFILE_DATA, branch },
    });
  }
  if (u.pathname === '/api/branches/profile' && method === 'PUT') {
    if (body?.code === 'CONFLICT') {
      return json(
        { error: { code: 'conflict', message: 'รหัสสาขาซ้ำกับสาขาอื่น' } },
        409,
      );
    }
    return json({
      branch: body.branch,
      name: 'Silom Branch',
      code: body.code,
      address: body.address,
      timezone: body.timezone,
      openingHours: body.openingHours,
      bookingEnabled: body.bookingEnabled,
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
    () => <BranchesScreen branch={b} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-11T05:00:00Z') });
  calls = [];
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

describe('A04 BranchesScreen', () => {
  it("pins its menu record: id 'branches', group 'data', order 14, roles ['ADMIN']", () => {
    expect(menu.id).toBe('branches');
    expect(menu.group).toBe('data');
    expect(menu.order).toBe(14);
    expect(menu.roles).toEqual(['ADMIN']);
    expect(menu.roles).not.toBe('all');
    for (const nonAdmin of ['TELLER', 'BRANCH_MANAGER', 'FINANCE', 'EXECUTIVE', 'MANAGER']) {
      expect((menu.roles as readonly string[]).includes(nonAdmin)).toBe(false);
    }
  });

  it('loads and displays branch profile in the form', async () => {
    const { container } = mount();
    await flush();

    const codeInput = container.querySelector<HTMLInputElement>('input[name="code"]');
    expect(codeInput?.value).toBe('SILOM');

    const addressInput = container.querySelector<HTMLInputElement>('input[name="address"]');
    expect(addressInput?.value).toBe('1 Silom Rd');

    const timezoneInput = container.querySelector<HTMLInputElement>('input[name="timezone"]');
    expect(timezoneInput?.value).toBe('Asia/Bangkok');

    const bookingInput = container.querySelector<HTMLInputElement>('input[name="bookingEnabled"]');
    expect(bookingInput?.checked).toBe(true);
  });

  it('submitting form sends PUT /api/branches/profile with branchSave payload and idempotency-key', async () => {
    const { container } = mount();
    await flush();

    const codeInput = container.querySelector<HTMLInputElement>('input[name="code"]')!;
    codeInput.value = 'SILOM-NEW';
    codeInput.dispatchEvent(new Event('input', { bubbles: true }));

    const form = container.querySelector('form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const saveCalls = calls.filter(
      (c) => c.method === 'PUT' && c.url.includes('/api/branches/profile'),
    );
    expect(saveCalls.length).toBe(1);
    expect(saveCalls[0]?.body.code).toBe('SILOM-NEW');
    expect(saveCalls[0]?.body.branch).toBe(B01);
    expect(saveCalls[0]?.headers.get('idempotency-key')).toBeTruthy();

    const successEl = container.querySelector('[role="status"]');
    expect(successEl?.textContent).toBeTruthy();
  });

  it('refused branchSave shows error in the form', async () => {
    const { container } = mount();
    await flush();

    const codeInput = container.querySelector<HTMLInputElement>('input[name="code"]')!;
    codeInput.value = 'CONFLICT';
    codeInput.dispatchEvent(new Event('input', { bubbles: true }));

    const form = container.querySelector('form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl?.textContent).toContain('รหัสสาขาซ้ำกับสาขาอื่น');
  });
});
