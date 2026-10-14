import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { BranchesScreen } from './BranchesScreen';

const B01 = 'XC-B01';
const B02 = 'XC-B02';

const SAMPLE_BRANCHES = [
  { id: B01, name: 'Silom Branch' },
  { id: B02, name: 'Siam Branch' },
];

const SAMPLE_PROFILE_B01 = {
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

const SAMPLE_PROFILE_B02 = {
  branch: B02,
  name: 'Siam Branch',
  code: 'SIAM',
  address: '2 Siam Square',
  timezone: 'Asia/Bangkok',
  openingHours: {
    mon: [{ open: '10:00', close: '20:00' }],
  },
  bookingEnabled: false,
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('WE-6: screens/admin/branches/worded.test.tsx', () => {
  let disposers: Array<() => void> = [];
  let branchesReply: (() => Response) | 'network' | null;
  let profileReply: ((branch: string) => Response) | 'network' | null;
  let saveReply: (() => Response) | 'network' | null;

  beforeEach(() => {
    setLang('th');
    branchesReply = () => json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    profileReply = (b: string) =>
      json({ version: 1, changed: true, data: b === B02 ? SAMPLE_PROFILE_B02 : SAMPLE_PROFILE_B01 });
    saveReply = () => json(SAMPLE_PROFILE_B01);

    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const u = new URL(url, 'http://localhost');

      if (u.pathname === '/api/v3/views/admin_branches') {
        if (branchesReply === 'network') throw new TypeError('Failed to fetch');
        if (branchesReply) return branchesReply();
        return json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
      }

      if (u.pathname === '/api/v3/views/admin_branches_profile') {
        if (profileReply === 'network') throw new TypeError('Failed to fetch');
        if (profileReply) return profileReply(u.searchParams.get('branch') ?? B01);
        return json({ version: 1, changed: true, data: SAMPLE_PROFILE_B01 });
      }

      if (u.pathname === '/api/branches/profile' && method === 'PUT') {
        if (saveReply === 'network') throw new TypeError('Failed to fetch');
        if (saveReply) return saveReply();
        return json(SAMPLE_PROFILE_B01);
      }

      return json({ error: { code: 'notFound', message: 'not found' } }, 404);
    }));
  });

  afterEach(() => {
    for (const d of disposers) d();
    disposers = [];
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialBranch = B01) {
    const [branch, setBranch] = createSignal(initialBranch);
    const [shown, setShown] = createSignal(true);
    const period = () => ({ from: '2026-10-01', to: '2026-10-31' });

    const container = document.createElement('div');
    document.body.appendChild(container);

    const dispose = render(
      () => <BranchesScreen branch={branch} period={period} shown={shown} />,
      container,
    );
    disposers.push(dispose);
    return { container, setBranch, setShown };
  }

  function errorText(container: HTMLElement): string {
    const el = container.querySelector('[role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  // Catch 1: loadBranches (admin_branches)
  describe('Catch 1: loadBranches', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      branchesReply = 'network';
      const m1 = mount();

      await vi.waitFor(() => {
        expect(errorText(m1.container)).toBe(sharedTh['error.network']);
      });

      setLang('en');
      const m2 = mount();
      await vi.waitFor(() => {
        expect(errorText(m2.container)).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      branchesReply = () => json({ error: { code: 'serverError', message: 'Internal Server Error' } }, 500);
      const { container } = mount();

      await vi.waitFor(() => {
        expect(errorText(container)).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      branchesReply = () =>
        json({ error: { code: 'unkeyedBranchCode', message: 'ข้อความเฉพาะสำหรับสาขาจากเซิร์ฟเวอร์' } }, 400);
      const { container } = mount();

      await vi.waitFor(() => {
        expect(errorText(container)).toBe('ข้อความเฉพาะสำหรับสาขาจากเซิร์ฟเวอร์');
      });
    });
  });

  // Catch 2: loadProfile (admin_branches_profile)
  describe('Catch 2: loadProfile', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      profileReply = 'network';
      const m1 = mount();

      await vi.waitFor(() => {
        expect(errorText(m1.container)).toBe(sharedTh['error.network']);
      });

      setLang('en');
      const m2 = mount();
      await vi.waitFor(() => {
        expect(errorText(m2.container)).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      profileReply = () => json({ error: { code: 'serverError', message: 'Internal Server Error' } }, 500);
      const { container } = mount();

      await vi.waitFor(() => {
        expect(errorText(container)).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      profileReply = () =>
        json({ error: { code: 'unkeyedProfileCode', message: 'ข้อความเฉพาะสำหรับโปรไฟล์จากเซิร์ฟเวอร์' } }, 400);
      const { container } = mount();

      await vi.waitFor(() => {
        expect(errorText(container)).toBe('ข้อความเฉพาะสำหรับโปรไฟล์จากเซิร์ฟเวอร์');
      });
    });
  });

  // Catch 3: handleSave (PUT /api/branches/profile)
  describe('Catch 3: handleSave', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const { container } = mount();

      await vi.waitFor(() => {
        expect(container.querySelector('form.branches-form')).not.toBeNull();
      });

      saveReply = 'network';
      const form = container.querySelector('form.branches-form') as HTMLFormElement;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

      await vi.waitFor(() => {
        expect(errorText(container)).toBe(sharedTh['error.network']);
      });

      setLang('en');
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

      await vi.waitFor(() => {
        expect(errorText(container)).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      const { container } = mount();

      await vi.waitFor(() => {
        expect(container.querySelector('form.branches-form')).not.toBeNull();
      });

      saveReply = () => json({ error: { code: 'serverError', message: 'Internal Server Error' } }, 500);
      const form = container.querySelector('form.branches-form') as HTMLFormElement;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

      await vi.waitFor(() => {
        expect(errorText(container)).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const { container } = mount();

      await vi.waitFor(() => {
        expect(container.querySelector('form.branches-form')).not.toBeNull();
      });

      saveReply = () =>
        json({ error: { code: 'unkeyedSaveCode', message: 'ข้อความเฉพาะสำหรับบันทึกจากเซิร์ฟเวอร์' } }, 400);
      const form = container.querySelector('form.branches-form') as HTMLFormElement;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

      await vi.waitFor(() => {
        expect(errorText(container)).toBe('ข้อความเฉพาะสำหรับบันทึกจากเซิร์ฟเวอร์');
      });
    });
  });

  // Done criterion 3: branch switch clears stale data from previous branch
  describe('Done criterion 3: branch switch and stale data', () => {
    it('does not leave the previous branch profile data showing when new branch read fails', async () => {
      const { container } = mount(B01);

      await vi.waitFor(() => {
        expect(container.textContent).toContain('Silom Branch');
      });

      // Fail profile read for new branch
      profileReply = 'network';

      const select = container.querySelector('#branch-select') as HTMLSelectElement;
      select.value = B02;
      select.dispatchEvent(new Event('change', { bubbles: true }));

      await vi.waitFor(() => {
        expect(errorText(container)).toBe(sharedTh['error.network']);
      });

      // Previous branch profile is not showing
      expect(container.querySelector('.branches-card')).toBeNull();
      expect(container.textContent).not.toContain('1 Silom Rd');
    });
  });
});
