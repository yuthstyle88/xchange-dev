import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import { UsersScreen } from './UsersScreen';

const disposers: Array<() => void> = [];

const B01 = 'XC-B01';
const B09 = 'XC-B09';

const BRANCHES_DATA = {
  branches: [
    { id: B01, name: 'Silom Branch' },
    { id: B09, name: 'Siam Branch' },
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

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

let calls: Array<{ url: string; method: string; body: any; headers: Headers }>;
let branchesReply: (() => Response) | null;

async function serve(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const method = (init?.method ?? 'GET').toUpperCase();
  const headers = new Headers(init?.headers);
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  calls.push({ url, method, body, headers });

  const u = new URL(url, 'http://localhost');
  if (u.pathname === '/api/v3/views/admin_branches') {
    if (branchesReply) return branchesReply();
    return json({ version: 1, changed: true, data: BRANCHES_DATA });
  }
  if (u.pathname === '/api/v3/views/admin_users') {
    return json({ version: 1, changed: true, data: SEED_USERS });
  }
  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function mount(branch = B01) {
  const [b, setBranch] = createSignal(branch);
  const [shown] = createSignal(true);
  const period = () => ({ from: '2026-10-01', to: '2026-10-31' });

  const container = document.createElement('div');
  document.body.appendChild(container);

  const dispose = render(
    () => <UsersScreen branch={b} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container, setBranch };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-11T05:00:00Z') });
  calls = [];
  branchesReply = null;
  document.body.innerHTML = '';
  vi.stubGlobal('fetch', vi.fn(serve));
  setLang('th');
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('108-Plaza/xchange#1716: UsersScreen branch list failure shows worded error', () => {
  it('shows worded error when admin_branches fails with 500', async () => {
    branchesReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const { container } = mount();
    await flush();

    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl, 'Error alert exists when branch list fails').not.toBeNull();
    const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    expect(errorEl?.textContent).toContain(expected);
  });

  it('shows worded error when admin_branches fails with 403 refusal (roleNotAllowedHere)', async () => {
    branchesReply = () => refusal(403, 'roleNotAllowedHere', 'raw wire message');
    const { container } = mount();
    await flush();

    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl, 'Error alert exists when branch list fails').not.toBeNull();
    const expected = errorText('roleNotAllowedHere', 'เซิร์ฟเวอร์ขัดข้อง');
    expect(errorEl?.textContent).toContain(expected);
    expect(errorEl?.textContent).not.toContain('raw wire message');
  });

  it('shows generic worded error for 400 with unknown code without leaking raw wire message', async () => {
    branchesReply = () => refusal(400, 'somethingNew', 'raw wire message');
    const { container } = mount();
    await flush();

    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl).not.toBeNull();
    const expected = 'raw wire message';
    expect(errorEl?.textContent).toContain(expected);
    expect(errorEl?.textContent).not.toContain('somethingNew');
  });

  it('shows generic worded error when fetch rejects with TypeError without leaking Failed to fetch', async () => {
    branchesReply = () => {
      throw new TypeError('Failed to fetch');
    };
    const { container } = mount();
    await flush();

    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl).not.toBeNull();
    const expected = errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่');
    expect(errorEl?.textContent).toContain(expected);
    expect(errorEl?.textContent).not.toContain('Failed to fetch');
  });

  it('retains stale branch list while showing error if subsequent branch list read fails', async () => {
    let callCount = 0;
    branchesReply = () => {
      callCount++;
      if (callCount === 1) {
        return json({ version: 1, changed: true, data: BRANCHES_DATA });
      }
      return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    };

    const { container, setBranch } = mount();
    await flush();

    // Open create dialog to verify branch options were populated
    const createBtn = container.querySelector<HTMLButtonElement>('.users-btn-primary');
    expect(createBtn).not.toBeNull();
    createBtn!.click();
    await flush();

    const branchSelect = container.querySelector<HTMLSelectElement>('#create-branch');
    expect(branchSelect?.options.length, 'Options populated initially').toBe(3);

    // Switch branch to trigger reload of branches
    setBranch('B09');
    await flush();

    // Verify error is shown and branches are retained
    const errorEl = container.querySelector('[role="alert"]');
    expect(errorEl).not.toBeNull();
    expect(errorEl?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(branchSelect?.options.length, 'Stale branches remain in picker').toBe(3);
  });
});
