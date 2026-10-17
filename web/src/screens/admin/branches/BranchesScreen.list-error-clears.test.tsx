import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';

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
  },
  bookingEnabled: true,
};

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
let profileReply: (() => Response) | null;
let saveReply: (() => Response) | null;

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
  if (u.pathname === '/api/v3/views/admin_branches_profile') {
    if (profileReply) return profileReply();
    const branch = u.searchParams.get('branch') ?? B01;
    return json({
      version: 1,
      changed: true,
      data: { ...PROFILE_DATA, branch },
    });
  }
  if (u.pathname === '/api/branches/profile' && method === 'PUT') {
    if (saveReply) return saveReply();
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

function mount(initialBranch = B01) {
  const [b, setBranch] = createSignal(initialBranch);
  const [shown, setShown] = createSignal(true);
  const period = () => ({ from: '2026-10-01', to: '2026-10-31' });

  const container = document.createElement('div');
  document.body.appendChild(container);

  const dispose = render(
    () => <BranchesScreen branch={b} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container, setBranch, setShown };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-11T05:00:00Z') });
  calls = [];
  branchesReply = null;
  profileReply = null;
  saveReply = null;
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

describe('108-Plaza/xchange#2666: Branches list error clearing behaviour', () => {
  // Row 1: The list read fails on a re-show, then succeeds on the next re-show (same selected branch)
  it('Row 1: The list read fails on a re-show, then succeeds on the next re-show (same selected branch) -> Alert gone after the success; form still shown', async () => {
    const { container, setShown } = mount();
    await flush();

    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelector('.branches-card')).not.toBeNull();

    // Re-show with list read failure
    setShown(false);
    await flush();
    branchesReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    setShown(true);
    await flush();

    expect(container.querySelector('[role="alert"]')).not.toBeNull();

    // Next re-show with list read success (same selected branch)
    setShown(false);
    await flush();
    branchesReply = () => json({ version: 1, changed: true, data: BRANCHES_DATA });
    setShown(true);
    await flush();

    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelector('.branches-card')).not.toBeNull();
  });

  // Row 2: The list read fails after a shell branch change, then succeeds on the next change
  it('Row 2: The list read fails after a shell branch change, then succeeds on the next change -> Alert gone after the success', async () => {
    const { container, setBranch } = mount(B01);
    await flush();

    expect(container.querySelector('[role="alert"]')).toBeNull();

    // Shell branch changes and list read fails
    branchesReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    setBranch(B09);
    await flush();

    expect(container.querySelector('[role="alert"]')).not.toBeNull();

    // Next shell branch change and list read succeeds
    branchesReply = () => json({ version: 1, changed: true, data: BRANCHES_DATA });
    setBranch('XC-B03');
    await flush();

    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  // Row 3: The list succeeds but the profile read fails
  it('Row 3: The list succeeds but the profile read fails -> The profile error is still shown', async () => {
    profileReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const { container } = mount();
    await flush();

    const alert = container.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  // Row 4: A save is refused (e.g. codeConflict), then the list re-reads OK
  it('Row 4: A save is refused (e.g. codeConflict), then the list re-reads OK -> The save error is still shown', async () => {
    const { container, setShown } = mount();
    await flush();

    saveReply = () => refusal(409, 'codeConflict', 'รหัสสาขาซ้ำ');
    const form = container.querySelector('form.branches-form');
    form?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    const alertBefore = container.querySelector('[role="alert"]');
    expect(alertBefore).not.toBeNull();
    expect(alertBefore?.textContent).toContain('รหัสสาขาซ้ำ');

    // List re-reads OK on re-show
    setShown(false);
    await flush();
    setShown(true);
    await flush();

    const alertAfter = container.querySelector('[role="alert"]');
    expect(alertAfter).not.toBeNull();
    expect(alertAfter?.textContent).toContain('รหัสสาขาซ้ำ');
  });

  // Row 5: Refresh: the list fails, the profile succeeds
  it('Row 5: Refresh: the list fails, the profile succeeds -> The list error is shown', async () => {
    const { container } = mount();
    await flush();

    expect(container.querySelector('[role="alert"]')).toBeNull();

    branchesReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    profileReply = () => json({ version: 1, changed: true, data: { ...PROFILE_DATA, branch: B01 } });

    const refreshBtn = container.querySelector<HTMLButtonElement>('.branches-btn');
    refreshBtn!.click();
    await flush();

    const alert = container.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });
});
