// W1 (RBAC design docs/design/blueprint/user-management-rbac.md §3.6, §3.7): the Users screen gains the status actions
// (suspend / resume / enable), reset password with a temporary password shown once, the four profile fields, and a tab
// for every function W2–W4 fill in.
// Up-front tests by the ticket's writer (claude/177aa7ef). The implementer makes them pass and must not edit, delete
// or weaken them.
//
// Contract these tests read (the ticket §7 marks each [required]):
// - Tabs: buttons `[data-users-tab="users|matrix|audit|logins|import"]` in that order; panels `[data-users-panel="<id>"]`.
//   A tab whose component file does not exist yet shows `[data-users-tab-pending]` (W2–W4 add the files later).
// - Rows: each user row is `tr[data-user-id="<userId>"]`; its actions are `[data-users-action="suspend|resume|enable|
//   profile"]`, offered by status (ACTIVE: suspend, profile; SUSPENDED: resume, profile; DISABLED:
//   enable only).
// - Writes go through keyedWrite to POST /api/users/suspend | resume | enable | reset-password | profile, body
//   `{ userId }` (profile: plus the four fields, an empty field sent as null).
// - Amended by W1b (claude/177aa7ef): no row offers "reset" and nothing posts /api/users/reset-password. Owner's decision
//   2026-10-18 "C ก่อน และ A" (xchange#564 issuecomment-6246125141): staff reset a forgotten password themselves through
//   Identity, and the admin reset returns only after Identity's operator set-password endpoint (A) merges. The
//   temporary-password block below stays in the code for that return, but nothing can open it.
// - (Before W1b) the temporary password from reset showed once in `[data-temp-password]` and was gone after
//   `[data-users-action="temp-password-done"]`.
// - The profile form is `[data-users-form="profile"]` with inputs named employeeCode, department, position, tellCode
//   and the button `[data-users-action="profile-save"]`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { UsersScreen } from './UsersScreen';

type Row = {
  userId: string;
  name: string;
  status: string;
  version: number;
  roles: Array<{ role: string; branch: string | null; validFrom: string; validTo: string | null }>;
  employeeCode?: string | null;
  department?: string | null;
  position?: string | null;
  tellCode?: string | null;
};

const B01 = 'XC-B01';
const role = { role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null };
const ACTIVE: Row = {
  userId: 'u-active',
  name: 'Somchai Active',
  status: 'ACTIVE',
  version: 3,
  roles: [role],
  employeeCode: 'E-0042',
  department: 'Front desk',
  position: 'Teller',
  tellCode: 'T7',
};
const SUSPENDED: Row = { userId: 'u-susp', name: 'Malee Suspended', status: 'SUSPENDED', version: 5, roles: [role] };
const DISABLED: Row = { userId: 'u-dis', name: 'Anan Disabled', status: 'DISABLED', version: 9, roles: [] };

interface Write {
  path: string;
  key: string | null;
  body: any;
}
let writes: Write[] = [];
let userReads = 0;
let replies: Record<string, { status: number; body: unknown }> = {};
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

beforeEach(() => {
  setLang('th');
  writes = [];
  userReads = 0;
  replies = {};
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const u = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
        return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom' }] } });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
        userReads++;
        return json(200, { version: 1, changed: true, data: structuredClone([ACTIVE, SUSPENDED, DISABLED]) });
      }
      if (method === 'POST' && u.pathname.startsWith('/api/users/')) {
        writes.push({
          path: u.pathname,
          key: new Headers(init?.headers).get('idempotency-key'),
          body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
        });
        const r = replies[u.pathname] ?? { status: 200, body: { userId: 'u', status: 'ACTIVE', version: 1 } };
        return json(r.status, r.body);
      }
      return json(200, { version: 1, changed: false });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle(): Promise<void> {
  for (let round = 0; round < 6; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<HTMLElement> {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const c = document.createElement('div');
  document.body.appendChild(c);
  disposers.push(render(() => <UsersScreen branch={branch} period={period} shown={shown} />, c));
  await settle();
  return c;
}

function row(c: HTMLElement, userId: string): HTMLElement {
  const r = c.querySelector<HTMLElement>(`tr[data-user-id="${userId}"]`);
  expect(r, `a row tr[data-user-id="${userId}"]`).not.toBeNull();
  return r!;
}

function actions(r: HTMLElement): string[] {
  return [...r.querySelectorAll<HTMLElement>('[data-users-action]')].map((b) => b.dataset.usersAction!).sort();
}

function click(el: ParentNode, selector: string): void {
  const b = el.querySelector<HTMLElement>(selector);
  expect(b, `a control ${selector}`).not.toBeNull();
  b!.click();
}

describe('W1 tabs (§3.6: W1 mounts every tab)', () => {
  it('row 1: five tabs in order; users is shown first; each other tab shows its panel, pending until its file exists', async () => {
    const c = await mount();
    const tabs = [...c.querySelectorAll<HTMLElement>('[data-users-tab]')].map((b) => b.dataset.usersTab);
    expect(tabs).toEqual(['users', 'matrix', 'audit', 'logins', 'import']);
    expect(c.querySelector('[data-users-panel="users"]'), 'the users panel is shown first').not.toBeNull();
    expect(row(c, ACTIVE.userId).textContent).toContain(ACTIVE.name);

    const files: Record<string, string> = {
      matrix: 'PermissionMatrix',
      audit: 'UserAudit',
      logins: 'LoginHistory',
      import: 'RegisterImport',
    };
    const present = import.meta.glob('./{PermissionMatrix,UserAudit,LoginHistory,RegisterImport}.tsx');
    for (const [id, file] of Object.entries(files)) {
      click(c, `[data-users-tab="${id}"]`);
      await settle();
      const panel = c.querySelector(`[data-users-panel="${id}"]`);
      expect(panel, `the ${id} panel`).not.toBeNull();
      expect(c.querySelector('[data-users-panel="users"]'), `users panel hidden while ${id} is selected`).toBeNull();
      if (!Object.keys(present).some((k) => k.endsWith(`/${file}.tsx`))) {
        expect(panel!.querySelector('[data-users-tab-pending]'), `${file}.tsx does not exist yet: pending`).not.toBeNull();
      }
    }
  });
});

describe('W1 status actions (§3.2: SUSPENDED added; who may move)', () => {
  it('row 2: actions are offered by status', async () => {
    const c = await mount();
    expect(actions(row(c, ACTIVE.userId))).toEqual(['profile', 'suspend']);
    expect(actions(row(c, SUSPENDED.userId))).toEqual(['profile', 'resume']);
    expect(actions(row(c, DISABLED.userId))).toEqual(['enable']);
  });

  it('row 3: SUSPENDED is worded', async () => {
    const c = await mount();
    expect(row(c, SUSPENDED.userId).textContent).toContain((th as Record<string, string>).statusSuspended ?? '«statusSuspended missing»');
    expect(row(c, SUSPENDED.userId).textContent).not.toContain('SUSPENDED');
  });

  for (const [act, user, path, ok] of [
    ['suspend', ACTIVE, '/api/users/suspend', 'successSuspended'],
    ['resume', SUSPENDED, '/api/users/resume', 'successResumed'],
    ['enable', DISABLED, '/api/users/enable', 'successEnabled'],
  ] as const) {
    it(`row 4 (${act}): sends ${path} { userId } with an Idempotency-Key, shows the success line and reads the list again`, async () => {
      const c = await mount();
      const readsBefore = userReads;
      click(row(c, user.userId), `[data-users-action="${act}"]`);
      await settle();
      expect(writes.filter((w) => w.path === path)).toHaveLength(1);
      const w = writes.find((x) => x.path === path)!;
      expect(w.body).toEqual({ userId: user.userId });
      expect(w.key, 'an Idempotency-Key header').toBeTruthy();
      expect(c.textContent).toContain((th as Record<string, string>)[ok] ?? `«${ok} missing»`);
      expect(userReads, 'the list is read again').toBeGreaterThan(readsBefore);
    });
  }

  it('row 5: a refused suspend shows the worded refusal and no success line', async () => {
    replies['/api/users/suspend'] = { status: 409, body: { error: { code: 'refused', message: 'ผู้ใช้ยังถือเงินสดอยู่' } } };
    const c = await mount();
    click(row(c, ACTIVE.userId), '[data-users-action="suspend"]');
    await settle();
    expect(c.textContent).toContain('ผู้ใช้ยังถือเงินสดอยู่');
    expect(c.textContent).not.toContain((th as Record<string, string>).successSuspended ?? '«successSuspended missing»');
  });
});

describe('W1b: no admin reset until Identity endpoint A (owner "C ก่อน และ A", xchange#564 issuecomment-6246125141)', () => {
  it('row 6: no row offers reset, nothing posts /api/users/reset-password, and no temporary password can show', async () => {
    replies['/api/users/reset-password'] = { status: 200, body: { userId: ACTIVE.userId, temporaryPassword: 'Kq7mPz3RtW9xVb2N' } };
    const c = await mount();
    expect(c.querySelector('[data-users-action="reset"]'), 'no reset button on any row').toBeNull();
    for (const user of [ACTIVE, SUSPENDED, DISABLED]) {
      for (const b of [...row(c, user.userId).querySelectorAll<HTMLElement>('[data-users-action]')]) {
        if (b.dataset.usersAction === 'profile') continue;
        b.click();
        await settle();
      }
    }
    expect(writes.filter((x) => x.path === '/api/users/reset-password'), 'no reset-password write').toHaveLength(0);
    expect(c.querySelector('[data-temp-password]'), 'no temporary-password block').toBeNull();
    expect(c.textContent).not.toContain('Kq7mPz3RtW9xVb2N');
  });
});

describe('W1 profile fields (§1.2 G11, §4: employee code, department, position, tell code)', () => {
  it('row 7: the profile form opens filled from the row and saves the four fields, an empty one as null', async () => {
    const c = await mount();
    click(row(c, ACTIVE.userId), '[data-users-action="profile"]');
    await settle();
    const form = c.querySelector<HTMLElement>('[data-users-form="profile"]');
    expect(form, 'the profile form').not.toBeNull();
    const input = (name: string) => {
      const el = form!.querySelector<HTMLInputElement>(`input[name="${name}"]`);
      expect(el, `an input named ${name}`).not.toBeNull();
      return el!;
    };
    expect(input('employeeCode').value).toBe('E-0042');
    expect(input('department').value).toBe('Front desk');
    expect(input('position').value).toBe('Teller');
    expect(input('tellCode').value).toBe('T7');

    const dept = input('department');
    dept.value = 'Back office';
    dept.dispatchEvent(new Event('input', { bubbles: true }));
    const tell = input('tellCode');
    tell.value = '';
    tell.dispatchEvent(new Event('input', { bubbles: true }));
    click(form!, '[data-users-action="profile-save"]');
    await settle();

    const w = writes.filter((x) => x.path === '/api/users/profile');
    expect(w).toHaveLength(1);
    expect(w[0]!.body).toEqual({
      userId: ACTIVE.userId,
      employeeCode: 'E-0042',
      department: 'Back office',
      position: 'Teller',
      tellCode: null,
    });
    expect(w[0]!.key).toBeTruthy();
    expect(c.textContent).toContain((th as Record<string, string>).successProfileSaved ?? '«successProfileSaved missing»');
  });

  it('row 8: a row without the fields (admin_users does not return them yet) opens an empty form', async () => {
    const c = await mount();
    click(row(c, SUSPENDED.userId), '[data-users-action="profile"]');
    await settle();
    const form = c.querySelector<HTMLElement>('[data-users-form="profile"]');
    expect(form).not.toBeNull();
    for (const name of ['employeeCode', 'department', 'position', 'tellCode']) {
      expect(form!.querySelector<HTMLInputElement>(`input[name="${name}"]`)?.value).toBe('');
    }
  });
});

describe('W1 i18n (§3.6: W1 adds every key W2–W4 read; #564 contract)', () => {
  it('row 9: every key of the #564 contract exists in th and en, worded (not empty)', async () => {
    const tables = await import('./i18n');
    const keys = [
      'tabUsers', 'tabMatrix', 'tabAudit', 'tabLogins', 'tabImport', 'tabPending',
      'statusSuspended', 'suspend', 'resume', 'enable', 'resetPassword',
      'successSuspended', 'successResumed', 'successEnabled',
      'tempPasswordTitle', 'tempPasswordOnce', 'tempPasswordDone',
      'profileButton', 'profileTitle', 'profileEmployeeCode', 'profileDepartment', 'profilePosition', 'profileTellCode',
      'profileSave', 'successProfileSaved',
      'matrixTitle', 'matrixReadOnly', 'matrixRole', 'matrixModule', 'matrixLevelNone', 'matrixLevelView',
      'matrixLevelCreate', 'matrixLevelFull', 'matrixExport', 'matrixEmpty',
      // The ten modules of C2 (#3088 §7: Module::as_str()).
      'matrixModule_DASHBOARD', 'matrixModule_CUSTOMERS', 'matrixModule_DOCUMENTS', 'matrixModule_RATES_STOCK',
      'matrixModule_FINANCE', 'matrixModule_RISK', 'matrixModule_REPORTS', 'matrixModule_USER_MANAGEMENT',
      'matrixModule_SYSTEM_SETTINGS', 'matrixModule_AUDIT_LOGS',
      'auditTitle', 'auditWhen', 'auditActor', 'auditSubject', 'auditAction', 'auditBefore', 'auditAfter', 'auditIp',
      'auditEmpty', 'auditExport',
      'loginsTitle', 'loginsWhen', 'loginsUser', 'loginsResult', 'loginsIp', 'loginsEmpty', 'loginsExport',
      'loginsResultOk', 'loginsResultBadPassword', 'loginsResultUnknownUser', 'loginsResultNotActive',
      'loginsResultTempExpired', 'loginsResultRateLimited',
      'importTitle', 'importChooseFile', 'importPreview', 'importApply', 'importRowsCreate', 'importRowsUpdate',
      'importRowsUnchanged', 'importRowError', 'importDroppedColumn', 'importStoppedAt', 'importTempPasswordsFile',
      'importNothingToApply',
    ];
    for (const lang of ['th', 'en'] as const) {
      const table = (tables as unknown as Record<string, Record<string, unknown>>)[lang];
      const missing = keys.filter((k) => typeof table[k] !== 'string' || (table[k] as string).trim() === '');
      expect(missing, `${lang} keys missing or empty`).toEqual([]);
    }
  });
});
