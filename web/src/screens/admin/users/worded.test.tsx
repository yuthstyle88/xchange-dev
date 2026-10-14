import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import type { AdminUserRow } from '../../../shared/api/generated/admin_users';
import { UsersScreen } from './UsersScreen';

const B01 = 'XC-B01';
const B09 = 'XC-B09';

const SAMPLE_BRANCHES = [
  { id: B01, name: 'Silom Branch' },
  { id: B09, name: 'Siam Branch' },
];

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
    name: 'Somsri Invited',
    status: 'INVITED',
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

async function settle(): Promise<void> {
  await vi.runAllTimersAsync();
  await Promise.resolve();
  await Promise.resolve();
}

describe('WE-27: screens/admin/users/ worded error tests (web-worded-error.md)', () => {
  let disposers: Array<() => void> = [];
  let branchesHandler: (() => Response) | 'network' | null = null;
  let usersHandler: (() => Response) | 'network' | null = null;
  let postUsersHandler: (() => Response) | 'network' | null = null;
  let grantHandler: (() => Response) | 'network' | null = null;
  let revokeHandler: (() => Response) | 'network' | null = null;
  let disableHandler: (() => Response) | 'network' | null = null;

  beforeEach(() => {
    vi.useFakeTimers();
    disposers = [];
    branchesHandler = () => json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    usersHandler = () => json({ version: 1, changed: true, data: SEED_USERS });
    postUsersHandler = () => json({ ok: true });
    grantHandler = () => json({ ok: true });
    revokeHandler = () => json({ ok: true });
    disableHandler = () => json({ ok: true });
    document.body.innerHTML = '';
    setLang('th');

    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const u = new URL(url, 'http://localhost');

      if (u.pathname === '/api/v3/views/admin_branches') {
        if (branchesHandler === 'network') throw new TypeError('Failed to fetch');
        if (branchesHandler) return branchesHandler();
        return json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
      }

      if (u.pathname === '/api/v3/views/admin_users') {
        if (usersHandler === 'network') throw new TypeError('Failed to fetch');
        if (usersHandler) return usersHandler();
        return json({ version: 1, changed: true, data: SEED_USERS });
      }

      if (u.pathname === '/api/users' && method === 'POST') {
        if (postUsersHandler === 'network') throw new TypeError('Failed to fetch');
        if (postUsersHandler) return postUsersHandler();
        return json({ ok: true });
      }

      if (u.pathname === '/api/users/grant' && method === 'POST') {
        if (grantHandler === 'network') throw new TypeError('Failed to fetch');
        if (grantHandler) return grantHandler();
        return json({ ok: true });
      }

      if (u.pathname === '/api/users/revoke' && method === 'POST') {
        if (revokeHandler === 'network') throw new TypeError('Failed to fetch');
        if (revokeHandler) return revokeHandler();
        return json({ ok: true });
      }

      if (u.pathname === '/api/users/disable' && method === 'POST') {
        if (disableHandler === 'network') throw new TypeError('Failed to fetch');
        if (disableHandler) return disableHandler();
        return json({ ok: true });
      }

      return json({ error: { code: 'notFound', message: 'not found' } }, 404);
    }));
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()?.();
    document.body.innerHTML = '';
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialBranch = B01) {
    while (disposers.length) disposers.pop()?.();
    document.body.innerHTML = '';

    const [branch, setBranch] = createSignal(initialBranch);
    const [shown, setShown] = createSignal(true);
    const period = () => ({ from: '2026-10-01', to: '2026-10-31' });

    const container = document.createElement('div');
    document.body.appendChild(container);

    const dispose = render(
      () => <UsersScreen branch={branch} period={period} shown={shown} />,
      container,
    );
    disposers.push(dispose);
    return { container, setBranch, setShown };
  }

  // ==========================================
  // Catch 1: loadBranches
  // ==========================================
  describe('Catch 1: loadBranches', () => {
    it('(1) network TypeError shows th error.network and en error.network', async () => {
      branchesHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.network']);
      expect(errorEl?.textContent).not.toContain('Failed to fetch');
      expect(errorEl?.textContent).not.toContain('TypeError');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      const errorElEn = containerEn.querySelector('[role="alert"]');
      expect(errorElEn?.textContent).toContain(en['error.network']);
      expect(errorElEn?.textContent).not.toContain('Failed to fetch');
    });

    it('(2) a 500 refusal shows th error.serverError even with a keyed code', async () => {
      branchesHandler = () => refusal(500, 'roleNotAllowedHere', 'Database branch failure');
      setLang('th');
      const { container } = mount();
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
      expect(errorEl?.textContent).not.toContain('Database branch failure');
      expect(errorEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal with a server message shows that message', async () => {
      const sentence = 'ไม่สามารถดึงรายชื่อสาขาเพื่อแสดงสิทธิ์ผู้ใช้ได้';
      branchesHandler = () => refusal(400, 'unmappedBranchLookupRefusal', sentence);
      setLang('th');
      const { container } = mount();
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(sentence);
      expect(errorEl?.textContent).not.toContain('unmappedBranchLookupRefusal');
    });
  });

  // ==========================================
  // Catch 2: loadUsers
  // ==========================================
  describe('Catch 2: loadUsers', () => {
    it('(1) network TypeError shows th error.network and en error.network', async () => {
      usersHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.network']);
      expect(errorEl?.textContent).not.toContain('Failed to fetch');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      const errorElEn = containerEn.querySelector('[role="alert"]');
      expect(errorElEn?.textContent).toContain(en['error.network']);
      expect(errorElEn?.textContent).not.toContain('Failed to fetch');
    });

    it('(2) a 500 refusal shows th error.serverError even with a keyed code', async () => {
      usersHandler = () => refusal(500, 'roleNotAllowedHere', 'Users store crash');
      setLang('th');
      const { container } = mount();
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
      expect(errorEl?.textContent).not.toContain('Users store crash');
      expect(errorEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal with a server message shows that message', async () => {
      const sentence = 'ระบบยังไม่พร้อมให้บริการรายชื่อผู้ใช้ในขณะนี้';
      usersHandler = () => refusal(400, 'unmappedUsersQueryRefusal', sentence);
      setLang('th');
      const { container } = mount();
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(sentence);
      expect(errorEl?.textContent).not.toContain('unmappedUsersQueryRefusal');
    });
  });

  // ==========================================
  // Catch 3: handleCreateSubmit
  // ==========================================
  describe('Catch 3: handleCreateSubmit', () => {
    async function openAndSubmitCreate(container: HTMLElement) {
      const createBtn = container.querySelector<HTMLButtonElement>('.users-btn-primary');
      createBtn!.click();
      await settle();

      const nameInput = container.querySelector<HTMLInputElement>('#create-name');
      const phoneInput = container.querySelector<HTMLInputElement>('#create-phone');
      const pwdInput = container.querySelector<HTMLInputElement>('input[name="password"]');
      nameInput!.value = 'New Staff';
      nameInput!.dispatchEvent(new Event('input', { bubbles: true }));
      phoneInput!.value = '0812345678';
      phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
      pwdInput!.value = 'secret123';
      pwdInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const form = container.querySelector<HTMLFormElement>('.users-dialog-form');
      form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await settle();
    }

    it('(1) network TypeError on create shows th error.network and en error.network', async () => {
      postUsersHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitCreate(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(th['error.network']);
      expect(dialogError?.textContent).not.toContain('Failed to fetch');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      await openAndSubmitCreate(containerEn);
      const dialogErrorEn = containerEn.querySelector('.users-dialog [role="alert"]');
      expect(dialogErrorEn?.textContent).toContain(en['error.network']);
      expect(dialogErrorEn?.textContent).not.toContain('Failed to fetch');
    });

    it('(2) a 500 refusal on create shows th error.serverError even with a keyed code', async () => {
      postUsersHandler = () => refusal(500, 'roleNotAllowedHere', 'Create 500 server crash');
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitCreate(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(th['error.serverError']);
      expect(dialogError?.textContent).not.toContain('Create 500 server crash');
      expect(dialogError?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal on create shows server message', async () => {
      const sentence = 'ไม่สามารถสร้างผู้ใช้ได้เนื่องจากชื่อผู้ใช้ซ้ำซ้อนในระบบ';
      postUsersHandler = () => refusal(400, 'unmappedCreateUserError', sentence);
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitCreate(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(sentence);
      expect(dialogError?.textContent).not.toContain('unmappedCreateUserError');
    });
  });

  // ==========================================
  // Catch 4: handleActivateSubmit
  // ==========================================
  describe('Catch 4: handleActivateSubmit', () => {
    async function openAndSubmitActivate(container: HTMLElement) {
      const activateBtn = container.querySelector<HTMLButtonElement>('.users-btn-activate');
      activateBtn!.click();
      await settle();

      const phoneInput = container.querySelector<HTMLInputElement>('#activate-phone');
      const pwdInput = container.querySelector<HTMLInputElement>('input[name="activatePassword"]');
      phoneInput!.value = '0899999999';
      phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
      pwdInput!.value = 'newpass123';
      pwdInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const form = container.querySelector<HTMLFormElement>('.users-dialog-form');
      form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await settle();
    }

    it('(1) network TypeError on activate shows th error.network and en error.network', async () => {
      postUsersHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitActivate(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(th['error.network']);
      expect(dialogError?.textContent).not.toContain('Failed to fetch');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      await openAndSubmitActivate(containerEn);
      const dialogErrorEn = containerEn.querySelector('.users-dialog [role="alert"]');
      expect(dialogErrorEn?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal on activate shows th error.serverError even with a keyed code', async () => {
      postUsersHandler = () => refusal(500, 'roleNotAllowedHere', 'Activation service crash');
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitActivate(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(th['error.serverError']);
      expect(dialogError?.textContent).not.toContain('Activation service crash');
      expect(dialogError?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal on activate shows server message', async () => {
      const sentence = 'ไม่สามารถเปิดใช้งานบัญชีได้เนื่องจากหมายเลขโทรศัพท์ไม่ถูกต้อง';
      postUsersHandler = () => refusal(400, 'unmappedActivateUserError', sentence);
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitActivate(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(sentence);
      expect(dialogError?.textContent).not.toContain('unmappedActivateUserError');
    });
  });

  // ==========================================
  // Catch 5: handleGrantSubmit
  // ==========================================
  describe('Catch 5: handleGrantSubmit', () => {
    async function openAndSubmitGrant(container: HTMLElement) {
      const grantBtn = container.querySelector<HTMLButtonElement>('.users-btn-grant');
      grantBtn!.click();
      await settle();

      const form = container.querySelector<HTMLFormElement>('.users-dialog-form');
      form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await settle();
    }

    it('(1) network TypeError on grant shows th error.network and en error.network', async () => {
      grantHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitGrant(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(th['error.network']);
      expect(dialogError?.textContent).not.toContain('Failed to fetch');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      await openAndSubmitGrant(containerEn);
      const dialogErrorEn = containerEn.querySelector('.users-dialog [role="alert"]');
      expect(dialogErrorEn?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal on grant shows th error.serverError even with a keyed code', async () => {
      grantHandler = () => refusal(500, 'roleNotAllowedHere', 'Grant service crash');
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitGrant(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(th['error.serverError']);
      expect(dialogError?.textContent).not.toContain('Grant service crash');
      expect(dialogError?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal on grant shows server message', async () => {
      const sentence = 'ไม่สามารถมอบหมายบทบาทได้เนื่องจากผู้ใช้มีสิทธิ์ระดับสูงอยู่แล้ว';
      grantHandler = () => refusal(400, 'unmappedGrantRoleError', sentence);
      setLang('th');
      const { container } = mount();
      await settle();
      await openAndSubmitGrant(container);

      const dialogError = container.querySelector('.users-dialog [role="alert"]');
      expect(dialogError?.textContent).toContain(sentence);
      expect(dialogError?.textContent).not.toContain('unmappedGrantRoleError');
    });
  });

  // ==========================================
  // Catch 6: handleRevoke
  // ==========================================
  describe('Catch 6: handleRevoke', () => {
    async function triggerRevoke(container: HTMLElement) {
      const revokeBtn = container.querySelector<HTMLButtonElement>('.users-btn-revoke');
      revokeBtn!.click();
      await settle();
    }

    it('(1) network TypeError on revoke shows th error.network and en error.network', async () => {
      revokeHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();
      await triggerRevoke(container);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.network']);
      expect(errorEl?.textContent).not.toContain('Failed to fetch');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      await triggerRevoke(containerEn);
      const errorElEn = containerEn.querySelector('[role="alert"]');
      expect(errorElEn?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal on revoke shows th error.serverError even with a keyed code', async () => {
      revokeHandler = () => refusal(500, 'roleNotAllowedHere', 'Revoke service crash');
      setLang('th');
      const { container } = mount();
      await settle();
      await triggerRevoke(container);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
      expect(errorEl?.textContent).not.toContain('Revoke service crash');
      expect(errorEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal on revoke shows server message', async () => {
      const sentence = 'ไม่สามารถเพิกถอนสิทธิ์ได้เนื่องจากมีรายการค้างดำเนินการอยู่';
      revokeHandler = () => refusal(400, 'unmappedRevokeRoleError', sentence);
      setLang('th');
      const { container } = mount();
      await settle();
      await triggerRevoke(container);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(sentence);
      expect(errorEl?.textContent).not.toContain('unmappedRevokeRoleError');
    });
  });

  // ==========================================
  // Catch 7: handleDisable
  // ==========================================
  describe('Catch 7: handleDisable', () => {
    async function triggerDisable(container: HTMLElement) {
      const disableBtn = container.querySelector<HTMLButtonElement>('.users-btn-disable');
      disableBtn!.click();
      await settle();
    }

    it('(1) network TypeError on disable shows th error.network and en error.network', async () => {
      disableHandler = 'network';
      setLang('th');
      const { container } = mount();
      await settle();
      await triggerDisable(container);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.network']);
      expect(errorEl?.textContent).not.toContain('Failed to fetch');

      // English
      setLang('en');
      const { container: containerEn } = mount();
      await settle();
      await triggerDisable(containerEn);
      const errorElEn = containerEn.querySelector('[role="alert"]');
      expect(errorElEn?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal on disable shows th error.serverError even with a keyed code', async () => {
      disableHandler = () => refusal(500, 'roleNotAllowedHere', 'Disable service crash');
      setLang('th');
      const { container } = mount();
      await settle();
      await triggerDisable(container);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
      expect(errorEl?.textContent).not.toContain('Disable service crash');
      expect(errorEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) an unkeyed refusal on disable shows server message', async () => {
      const sentence = 'ไม่สามารถระงับการใช้งานบัญชีผู้ใช้นี้ได้';
      disableHandler = () => refusal(400, 'unmappedDisableUserError', sentence);
      setLang('th');
      const { container } = mount();
      await settle();
      await triggerDisable(container);

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(sentence);
      expect(errorEl?.textContent).not.toContain('unmappedDisableUserError');
    });
  });

  // ==========================================
  // Criterion 3: Branch switch clears stale data
  // ==========================================
  describe('Criterion 3: Branch switch stale data isolation', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const { container, setBranch } = mount(B01);
      await settle();

      expect(container.textContent).toContain('Somchai Active');

      // Fail subsequent users read on branch switch
      usersHandler = () => refusal(500, 'roleNotAllowedHere', 'Branch switch failure');
      setBranch(B09);
      await settle();

      const errorEl = container.querySelector('[role="alert"]');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
      // Stale user from previous branch must NOT be shown
      expect(container.textContent).not.toContain('Somchai Active');
    });
  });
});
