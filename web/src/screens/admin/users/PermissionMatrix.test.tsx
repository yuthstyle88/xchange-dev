// W2 up-front tests (claude/5edf7bb5, ticket author). RBAC §11 matrix tab: read for everyone, add/edit/delete for
// EXECUTIVE. Contract: generated `admin_permission_matrix.ts` / `admin_groups.ts` on `feat/rbac-groups` (skeleton
// 7de41348). The implementer makes these pass and must not edit, delete or weaken them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeAdminPermissionMatrixData,
  makeMatrixGroup,
  makeMatrixPermission,
} from '../../../shared/testing/fixtures/admin_permission_matrix';
import { en, th } from './i18n';
import { PermissionMatrix } from './PermissionMatrix';

const BRANCH = 'XC-B01';

function matrix(editableForCaller: boolean) {
  return makeAdminPermissionMatrixData({
    version: 7,
    modules: ['DASHBOARD', 'FINANCE', 'USER_MANAGEMENT'],
    permissions: [
      makeMatrixPermission({ name: 'OverviewRead', module: 'DASHBOARD', level: 'VIEW', editable: true }),
      makeMatrixPermission({ name: 'CashManage', module: 'FINANCE', level: 'FULL', editable: true }),
      makeMatrixPermission({ name: 'CashRead', module: 'FINANCE', level: 'VIEW', editable: true }),
      makeMatrixPermission({ name: 'UsersManage', module: 'USER_MANAGEMENT', level: 'FULL', editable: true }),
      makeMatrixPermission({ name: 'ShiftOperate', module: null, level: null, editable: false }),
    ],
    groups: [
      makeMatrixGroup({
        code: 'EXECUTIVE', name: 'EXECUTIVE', base: 'EXECUTIVE', builtIn: true, editable: false,
        permissions: ['OverviewRead', 'CashManage', 'CashRead', 'UsersManage', 'ShiftOperate'],
        levels: { DASHBOARD: 'VIEW', FINANCE: 'FULL', USER_MANAGEMENT: 'FULL' },
      }),
      makeMatrixGroup({
        code: 'TELLER', name: 'TELLER', base: 'TELLER', builtIn: true, editable: editableForCaller,
        permissions: ['CashRead', 'ShiftOperate'],
        levels: { FINANCE: 'VIEW' },
      }),
      makeMatrixGroup({
        code: 'SENIOR_TELLER', name: 'Senior teller', base: 'TELLER', builtIn: false, editable: editableForCaller,
        permissions: ['OverviewRead', 'CashRead', 'ShiftOperate'],
        levels: { DASHBOARD: 'VIEW', FINANCE: 'VIEW' },
      }),
    ],
  });
}

type Handler = (u: URL, init: RequestInit | undefined) => Response | Promise<Response>;
let calls: Array<{ path: string; method: string; body: unknown; key: string | null }> = [];
let handler: Handler | null = null;
let viewData = matrix(false);
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

beforeEach(() => {
  setLang('th');
  calls = [];
  handler = null;
  viewData = matrix(false);
  document.body.innerHTML = '';
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const u = new URL(String(input), 'http://localhost');
      const headers = new Headers(init?.headers);
      calls.push({
        path: u.pathname + u.search,
        method: (init?.method ?? 'GET').toUpperCase(),
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
        key: headers.get('Idempotency-Key'),
      });
      if (handler) return handler(u, init);
      if (u.pathname === '/api/v3/views/admin_permission_matrix') {
        return json(200, { version: viewData.version, changed: true, data: viewData });
      }
      if (u.pathname.startsWith('/api/admin/groups/')) {
        return json(200, { code: 'X', version: viewData.version + 1 });
      }
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );
  vi.stubGlobal('confirm', vi.fn(() => true));
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

function mount(shownAtStart = true) {
  const [shown, setShown] = createSignal(shownAtStart);
  const [branch] = createSignal(BRANCH);
  const root = document.createElement('div');
  document.body.appendChild(root);
  disposers.push(render(() => <PermissionMatrix shown={shown} branch={branch} />, root));
  return { root, setShown };
}

const flush = () => new Promise((r) => setTimeout(r, 0));
async function settle() {
  for (let i = 0; i < 6; i++) await flush();
}
const q = (sel: string) => document.querySelector(sel) as HTMLElement | null;
const qa = (sel: string) => Array.from(document.querySelectorAll(sel)) as HTMLElement[];
const views = () => calls.filter((c) => c.path.startsWith('/api/v3/views/admin_permission_matrix'));
const writes = (p: string) => calls.filter((c) => c.path === p && c.method === 'POST');

function click(el: HTMLElement | null) {
  expect(el, 'element to click').not.toBeNull();
  el!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
}
function setValue(el: HTMLElement | null, v: string) {
  expect(el, 'input to fill').not.toBeNull();
  const input = el as HTMLInputElement | HTMLSelectElement;
  input.value = v;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('W2 matrix tab (RBAC §11)', () => {
  it('M1 reads admin_permission_matrix for the tab branch, and only once the tab is shown', async () => {
    const { setShown } = mount(false);
    await settle();
    expect(views()).toHaveLength(0);
    setShown(true);
    await settle();
    expect(views()).toHaveLength(1);
    expect(views()[0]!.path).toBe(`/api/v3/views/admin_permission_matrix?branch=${BRANCH}`);
  });

  it('M2 shows one row per group and each module level in words, in the server module order', async () => {
    mount();
    await settle();
    expect(qa('[data-matrix-group]').map((r) => r.getAttribute('data-matrix-group'))).toEqual([
      'EXECUTIVE', 'TELLER', 'SENIOR_TELLER',
    ]);
    expect(q('[data-matrix-level="EXECUTIVE:FINANCE"]')?.textContent).toContain(th.matrixLevelFull);
    expect(q('[data-matrix-level="TELLER:FINANCE"]')?.textContent).toContain(th.matrixLevelView);
    // a module the group has no level in reads "none"
    expect(q('[data-matrix-level="TELLER:DASHBOARD"]')?.textContent).toContain(th.matrixLevelNone);
    const cols = qa('[data-matrix-group="TELLER"] [data-matrix-level]').map((c) =>
      c.getAttribute('data-matrix-level'),
    );
    expect(cols).toEqual(['TELLER:DASHBOARD', 'TELLER:FINANCE', 'TELLER:USER_MANAGEMENT']);
    // the group's display name is shown, not only its code
    expect(q('[data-matrix-group="SENIOR_TELLER"]')?.textContent).toContain('Senior teller');
  });

  it('M3 a caller with no editable group gets a read-only tab: no add, edit or delete control', async () => {
    mount();
    await settle();
    expect(q('[data-matrix-readonly]')?.textContent).toContain(th.matrixReadOnly);
    expect(qa('[data-matrix-edit]')).toHaveLength(0);
    expect(qa('[data-matrix-delete]')).toHaveLength(0);
    expect(q('[data-matrix-add-open]')).toBeNull();
  });

  it('M4 an EXECUTIVE caller can edit every editable group but never the built-in EXECUTIVE group', async () => {
    viewData = matrix(true);
    mount();
    await settle();
    expect(q('[data-matrix-readonly]')).toBeNull();
    expect(q('[data-matrix-edit="EXECUTIVE"]')).toBeNull();
    expect(q('[data-matrix-edit="TELLER"]')).not.toBeNull();
    expect(q('[data-matrix-edit="SENIOR_TELLER"]')).not.toBeNull();
    expect(q('[data-matrix-add-open]')).not.toBeNull();
    // delete only for a custom group
    expect(q('[data-matrix-delete="TELLER"]')).toBeNull();
    expect(q('[data-matrix-delete="SENIOR_TELLER"]')).not.toBeNull();
  });

  it('M5 editing offers only editable permissions, and save sends adminGroupSave with an Idempotency-Key, then re-reads', async () => {
    viewData = matrix(true);
    mount();
    await settle();
    click(q('[data-matrix-edit="TELLER"]'));
    await settle();
    // no-module permissions are never offered (§11.3 rule 5)
    expect(q('input[data-matrix-perm="ShiftOperate"]')).toBeNull();
    const cash = q('input[data-matrix-perm="CashRead"]') as HTMLInputElement;
    const overview = q('input[data-matrix-perm="OverviewRead"]') as HTMLInputElement;
    expect(cash?.checked).toBe(true);
    expect(overview?.checked).toBe(false);
    click(overview);
    await settle();
    click(q('[data-matrix-save]'));
    await settle();
    const saved = writes('/api/admin/groups/save');
    expect(saved).toHaveLength(1);
    expect(saved[0]!.key).toBeTruthy();
    // module permissions only, in the server's permission order
    expect(saved[0]!.body).toEqual({ code: 'TELLER', permissions: ['OverviewRead', 'CashRead'] });
    expect(views().length).toBeGreaterThanOrEqual(2);
  });

  it('M6 adding a group offers only built-in bases other than EXECUTIVE, and sends adminGroupAdd', async () => {
    viewData = matrix(true);
    mount();
    await settle();
    click(q('[data-matrix-add-open]'));
    await settle();
    const base = q('select[data-matrix-add-base]') as HTMLSelectElement;
    const options = Array.from(base?.options ?? []).map((o) => o.value).filter((v) => v !== '');
    expect(options).toEqual(['TELLER']);
    setValue(q('[data-matrix-add-code]'), 'FX_LEAD');
    setValue(q('[data-matrix-add-name]'), 'FX lead');
    setValue(base, 'TELLER');
    await settle();
    click(q('[data-matrix-add-form] input[data-matrix-perm="CashRead"]'));
    await settle();
    click(q('[data-matrix-add-submit]'));
    await settle();
    const added = writes('/api/admin/groups/add');
    expect(added).toHaveLength(1);
    expect(added[0]!.key).toBeTruthy();
    expect(added[0]!.body).toEqual({ code: 'FX_LEAD', name: 'FX lead', base: 'TELLER', permissions: ['CashRead'] });
  });

  it('M7 delete asks first: declining sends nothing, accepting sends adminGroupDelete', async () => {
    viewData = matrix(true);
    mount();
    await settle();
    (globalThis.confirm as unknown as ReturnType<typeof vi.fn>).mockReturnValueOnce(false);
    click(q('[data-matrix-delete="SENIOR_TELLER"]'));
    await settle();
    expect(writes('/api/admin/groups/delete')).toHaveLength(0);
    click(q('[data-matrix-delete="SENIOR_TELLER"]'));
    await settle();
    const deleted = writes('/api/admin/groups/delete');
    expect(deleted).toHaveLength(1);
    expect(deleted[0]!.body).toEqual({ code: 'SENIOR_TELLER' });
  });

  it('M8 a guardrail refusal is shown in words, never the raw server message, and keeps the edit', async () => {
    viewData = matrix(true);
    handler = (u) => {
      if (u.pathname === '/api/admin/groups/save') {
        return json(409, { error: { code: 'guardrail:2', message: 'RAW guardrail 2 text' } });
      }
      return json(200, { version: viewData.version, changed: true, data: viewData });
    };
    mount();
    await settle();
    click(q('[data-matrix-edit="TELLER"]'));
    await settle();
    click(q('input[data-matrix-perm="UsersManage"]'));
    await settle();
    click(q('[data-matrix-save]'));
    await settle();
    const err = q('[data-matrix-error]');
    expect(err?.textContent ?? '').not.toContain('RAW guardrail 2 text');
    expect(err?.textContent).toContain((th as Record<string, string>)['matrixGuardrail_2']);
    // the edit stays open with the user's choice
    expect((q('input[data-matrix-perm="UsersManage"]') as HTMLInputElement)?.checked).toBe(true);
  });

  it('M9 every new matrix key exists in both TH and EN', () => {
    const keys = (o: Record<string, string>) => Object.keys(o).filter((k) => k.startsWith('matrix')).sort();
    expect(keys(en as Record<string, string>)).toEqual(keys(th as Record<string, string>));
    for (const n of [1, 2, 3, 4, 5]) {
      expect((th as Record<string, string>)[`matrixGuardrail_${n}`], `th matrixGuardrail_${n}`).toBeTruthy();
    }
  });
});
