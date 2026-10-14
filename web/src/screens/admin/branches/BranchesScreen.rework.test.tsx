import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';
import { th, en } from './i18n';

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

describe('108-Plaza/xchange#1692: BranchesScreen rework requirements', () => {
  // Row 1: BranchesScreen.tsx:77 - branch list fails: empty picker, no message -> worded error
  describe('Row 1: branch list fails -> worded error', () => {
    it('shows worded error when branch list fails with 500', async () => {
      branchesReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const { container } = mount();
      await flush();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl, 'Error alert exists when branch list fails').not.toBeNull();
      const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      expect(errorEl?.textContent).toContain(expected);
    });

    it('shows worded error when branch list fails with 403 refusal (roleNotAllowedHere)', async () => {
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
      expect(errorEl, 'Error alert exists when branch list fails').not.toBeNull();
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
      expect(errorEl, 'Error alert exists when branch list fails').not.toBeNull();
      const expected = errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่');
      expect(errorEl?.textContent).toContain(expected);
      expect(errorEl?.textContent).not.toContain('Failed to fetch');
    });

    it('retains stale branch list while showing error if refresh fails', async () => {
      let callCount = 0;
      branchesReply = () => {
        callCount++;
        if (callCount === 1) {
          return json({ version: 1, changed: true, data: BRANCHES_DATA });
        }
        return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      };

      const { container } = mount();
      await flush();

      // Initial read succeeded: options exist in select
      const select = container.querySelector<HTMLSelectElement>('#branch-select');
      expect(select?.options.length).toBe(2);

      // Click refresh button
      const refreshBtn = container.querySelector<HTMLButtonElement>('.branches-btn');
      expect(refreshBtn).not.toBeNull();
      refreshBtn!.click();
      await flush();

      // Stale branches remain
      expect(select?.options.length).toBe(2);

      // Worded error is shown
      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
  });

  // Row 2: BranchesScreen.tsx:95 - timezone ?? 'Asia/Bangkok' -> show the field empty
  describe('Row 2: show timezone field empty when server has none', () => {
    it('leaves timezone input empty when server returns null timezone', async () => {
      profileReply = () =>
        json({
          version: 1,
          changed: true,
          data: { ...PROFILE_DATA, timezone: null },
        });

      const { container } = mount();
      await flush();

      const timezoneInput = container.querySelector<HTMLInputElement>('input[name="timezone"]');
      expect(timezoneInput).not.toBeNull();
      expect(timezoneInput?.value).toBe('');
    });
  });

  // Row 3: BranchesScreen.tsx:181 - blank timezone saved as Bangkok -> refuse to save with worded validation message
  describe('Row 3: refuse to save blank timezone with worded validation message', () => {
    it('refuses to save when timezone is blank and shows worded error in Thai', async () => {
      const { container } = mount();
      await flush();

      const timezoneInput = container.querySelector<HTMLInputElement>('input[name="timezone"]')!;
      timezoneInput.value = '';
      timezoneInput.dispatchEvent(new Event('input', { bubbles: true }));

      const form = container.querySelector('form')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      // Verify no PUT call was sent
      const saveCalls = calls.filter(
        (c) => c.method === 'PUT' && c.url.includes('/api/branches/profile'),
      );
      expect(saveCalls.length, 'Must refuse to send PUT when timezone is blank').toBe(0);

      // Worded error message is shown
      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl, 'Validation error alert is shown').not.toBeNull();
      expect(errorEl?.textContent).toBe((th as any).timezoneRequired ?? 'กรุณากรอกเขตเวลา');
    });

    it('refuses to save when timezone is whitespace and shows worded error in English', async () => {
      setLang('en');
      const { container } = mount();
      await flush();

      const timezoneInput = container.querySelector<HTMLInputElement>('input[name="timezone"]')!;
      timezoneInput.value = '   ';
      timezoneInput.dispatchEvent(new Event('input', { bubbles: true }));

      const form = container.querySelector('form')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const saveCalls = calls.filter(
        (c) => c.method === 'PUT' && c.url.includes('/api/branches/profile'),
      );
      expect(saveCalls.length).toBe(0);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toBe((en as any).timezoneRequired ?? 'Timezone is required');
    });
  });

  describe('conflict refusal shows localized worded error and never wire text', () => {
    it('shows codeConflict worded error in Thai on conflict refusal without leaking raw wire message', async () => {
      saveReply = () => refusal(409, 'conflict', 'raw conflict text from server');
      setLang('th');
      const { container } = mount();
      await flush();

      const form = container.querySelector('form')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toBe((th as any).codeConflict);
      expect(errorEl?.textContent).toBe('รหัสสาขาซ้ำกับสาขาอื่น');
      expect(errorEl?.textContent).not.toContain('raw conflict text from server');
    });

    it('shows codeConflict worded error in English on conflict refusal without leaking raw wire message', async () => {
      saveReply = () => refusal(409, 'conflict', 'raw conflict text from server');
      setLang('en');
      const { container } = mount();
      await flush();

      const form = container.querySelector('form')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toBe((en as any).codeConflict);
      expect(errorEl?.textContent).toBe('Branch code is already used by another branch');
      expect(errorEl?.textContent).not.toContain('raw conflict text from server');
    });
  });
});

