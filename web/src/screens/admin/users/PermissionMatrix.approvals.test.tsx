// AP-F1 up-front tests (XC FE lane controller claude/e8020026, 2026-10-22). RBAC §11.9 "FE": the permission matrix
// gains an "Approvals" section with the nine approval-kind permissions, on/off per group. Rule 6: an ADMIN base shows
// them unavailable and never sends them. Rule 4: the built-in EXECUTIVE group is fixed (read-only). The save payload
// keeps its shape (permission names), now including the approval kinds the group holds; `ApprovalsDecide` is the
// server's (A8). The DOM contract is in ApprovalsSection.tsx. The implementer makes these pass and must not edit,
// delete or weaken them. Harness copied from PermissionMatrix.test.tsx.
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

// The nine, in Permission::ALL order (§11.9). Written out here so the tests do not lean on the screen's own list.
const KINDS = [
  'ApproveCashTransferWithinLimit',
  'ApproveCashTransferAboveLimit',
  'ApproveCountDifference',
  'ApproveNegativeStock',
  'ApproveCampaignVersion',
  'ApproveRatePublication',
  'ApproveExchangeTransaction',
  'ApproveTransactionReversal',
  'ApproveLoyaltyRuleVersion',
];

// The seed (§11.9 table): who decides each kind today.
const BM_KINDS = ['ApproveCashTransferWithinLimit', 'ApproveNegativeStock', 'ApproveExchangeTransaction'];
const FIN_KINDS = ['ApproveCountDifference', 'ApproveTransactionReversal'];
const EXEC_KINDS = [
  'ApproveCashTransferAboveLimit',
  'ApproveCampaignVersion',
  'ApproveRatePublication',
  'ApproveLoyaltyRuleVersion',
];

const label = (kind: string) => (th as Record<string, string>)[`matrixApproval_${kind}`]!;

function matrix(editableForCaller: boolean) {
  return makeAdminPermissionMatrixData({
    version: 11,
    modules: ['DASHBOARD', 'CUSTOMERS', 'FINANCE', 'USER_MANAGEMENT'],
    // The view's order is Permission::ALL: module and no-module permissions interleaved, the nine last. The nine are
    // no-module, so the view sends them as `module: null, editable: false`, exactly like ApprovalsDecide.
    permissions: [
      makeMatrixPermission({ name: 'OverviewRead', module: 'DASHBOARD', level: 'VIEW', editable: true }),
      makeMatrixPermission({ name: 'UsersManage', module: 'USER_MANAGEMENT', level: 'FULL', editable: true }),
      makeMatrixPermission({ name: 'ApprovalsDecide', module: null, level: null, editable: false }),
      makeMatrixPermission({ name: 'ShiftOperate', module: null, level: null, editable: false }),
      makeMatrixPermission({ name: 'ExchangeTransact', module: 'CUSTOMERS', level: 'CREATE', editable: true }),
      makeMatrixPermission({ name: 'TransactionRead', module: 'CUSTOMERS', level: 'VIEW', editable: true }),
      makeMatrixPermission({ name: 'CashLimitsManage', module: 'FINANCE', level: 'FULL', editable: true }),
      ...KINDS.map((name) => makeMatrixPermission({ name, module: null, level: null, editable: false })),
    ],
    groups: [
      makeMatrixGroup({
        code: 'EXECUTIVE', name: 'EXECUTIVE', base: 'EXECUTIVE', builtIn: true, editable: false,
        permissions: [
          'OverviewRead', 'UsersManage', 'ApprovalsDecide', 'ExchangeTransact', 'TransactionRead', 'CashLimitsManage',
          ...EXEC_KINDS,
        ],
        levels: { DASHBOARD: 'VIEW', CUSTOMERS: 'CREATE', FINANCE: 'FULL', USER_MANAGEMENT: 'FULL' },
      }),
      makeMatrixGroup({
        code: 'ADMIN', name: 'ADMIN', base: 'ADMIN', builtIn: true, editable: editableForCaller,
        permissions: ['OverviewRead', 'UsersManage'],
        levels: { DASHBOARD: 'VIEW', USER_MANAGEMENT: 'FULL' },
      }),
      makeMatrixGroup({
        code: 'BRANCH_MANAGER', name: 'BRANCH_MANAGER', base: 'BRANCH_MANAGER', builtIn: true,
        editable: editableForCaller,
        permissions: [
          'OverviewRead', 'ApprovalsDecide', 'ShiftOperate', 'ExchangeTransact', 'TransactionRead', ...BM_KINDS,
        ],
        levels: { DASHBOARD: 'VIEW', CUSTOMERS: 'CREATE' },
      }),
      makeMatrixGroup({
        code: 'FINANCE', name: 'FINANCE', base: 'FINANCE', builtIn: true, editable: editableForCaller,
        permissions: ['OverviewRead', 'ApprovalsDecide', 'CashLimitsManage', ...FIN_KINDS],
        levels: { DASHBOARD: 'VIEW', FINANCE: 'FULL' },
      }),
      makeMatrixGroup({
        code: 'TELLER', name: 'TELLER', base: 'TELLER', builtIn: true, editable: editableForCaller,
        permissions: ['ShiftOperate', 'ExchangeTransact'],
        levels: { CUSTOMERS: 'CREATE' },
      }),
      // A custom group on the ADMIN base whose view row lists a kind (data written before rule 6). The screen must
      // still show it unavailable and never send it back.
      makeMatrixGroup({
        code: 'IT_HELP', name: 'IT help', base: 'ADMIN', builtIn: false, editable: editableForCaller,
        permissions: ['UsersManage', 'ApproveNegativeStock'],
        levels: { USER_MANAGEMENT: 'FULL' },
      }),
    ],
  });
}

const GROUPS = ['EXECUTIVE', 'ADMIN', 'BRANCH_MANAGER', 'FINANCE', 'TELLER', 'IT_HELP'];

type Handler = (u: URL, init: RequestInit | undefined) => Response | Promise<Response>;
let calls: Array<{ path: string; method: string; body: unknown; key: string | null }> = [];
let handler: Handler | null = null;
let viewData = matrix(true);
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

beforeEach(() => {
  setLang('th');
  calls = [];
  handler = null;
  viewData = matrix(true);
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

function mount() {
  const [shown] = createSignal(true);
  const [branch] = createSignal(BRANCH);
  const root = document.createElement('div');
  document.body.appendChild(root);
  disposers.push(render(() => <PermissionMatrix shown={shown} branch={branch} />, root));
  return { root };
}

const flush = () => new Promise((r) => setTimeout(r, 0));
async function settle() {
  for (let i = 0; i < 6; i++) await flush();
}
const q = (sel: string) => document.querySelector(sel) as HTMLElement | null;
const qa = (sel: string) => Array.from(document.querySelectorAll(sel)) as HTMLElement[];
const writes = (p: string) => calls.filter((c) => c.path === p && c.method === 'POST');
const toggle = (kind: string) => q(`[data-matrix-approvals-edit] input[data-matrix-approval="${kind}"]`) as HTMLInputElement | null;
const cell = (code: string, kind: string) => q(`[data-matrix-approval-cell="${code}:${kind}"]`);

function click(el: HTMLElement | null) {
  expect(el, 'element to click').not.toBeNull();
  el!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
}

async function openEdit(code: string) {
  mount();
  await settle();
  click(q(`[data-matrix-edit="${code}"]`));
  await settle();
}

async function saveAndBody(): Promise<{ code: string; permissions: string[] }> {
  click(q('[data-matrix-save]'));
  await settle();
  const saved = writes('/api/admin/groups/save');
  expect(saved, 'one adminGroupSave').toHaveLength(1);
  return saved[0]!.body as { code: string; permissions: string[] };
}

describe('AP-F1 matrix "Approvals" section (RBAC §11.9)', () => {
  it('A1 shows the nine kinds as rows under an "Approvals" heading, with Thai labels, for every group; the edit panel has nine toggles', async () => {
    await openEdit('BRANCH_MANAGER');
    const section = q('[data-matrix-approvals]');
    expect(section, 'the Approvals section').not.toBeNull();
    expect(section!.textContent).toContain(th.matrixApprovalsTitle);
    const rows = qa('[data-matrix-approval-row]');
    expect(rows.map((r) => r.getAttribute('data-matrix-approval-row'))).toEqual(KINDS);
    for (const row of rows) {
      expect(row.textContent).toContain(label(row.getAttribute('data-matrix-approval-row')!));
    }
    for (const code of GROUPS) {
      for (const kind of KINDS) expect(cell(code, kind), `${code}:${kind}`).not.toBeNull();
    }

    const editor = q('[data-matrix-approvals-edit]');
    expect(editor, 'the Approvals part of the edit panel').not.toBeNull();
    expect(editor!.textContent).toContain(th.matrixApprovalsTitle);
    const toggles = qa('[data-matrix-approvals-edit] input[data-matrix-approval]') as HTMLInputElement[];
    expect(toggles.map((i) => i.getAttribute('data-matrix-approval'))).toEqual(KINDS);
    for (const input of toggles) {
      expect(input.type).toBe('checkbox');
      expect(input.closest('label')?.textContent).toContain(label(input.getAttribute('data-matrix-approval')!));
    }
  });

  it('A2 every cell and toggle shows whether the group holds the kind: the seed BM 3, FINANCE 2, EXECUTIVE 4', async () => {
    mount();
    await settle();
    const holds: Record<string, string[]> = {
      EXECUTIVE: EXEC_KINDS,
      BRANCH_MANAGER: BM_KINDS,
      FINANCE: FIN_KINDS,
      TELLER: [],
    };
    for (const [code, held] of Object.entries(holds)) {
      for (const kind of KINDS) {
        const c = cell(code, kind);
        const on = held.includes(kind);
        expect(c?.getAttribute('data-held'), `${code}:${kind}`).toBe(on ? 'true' : 'false');
        expect(c?.textContent, `${code}:${kind}`).toContain(on ? th.matrixApprovalHeld : th.matrixApprovalNotHeld);
      }
    }

    click(q('[data-matrix-edit="BRANCH_MANAGER"]'));
    await settle();
    expect(KINDS.filter((k) => toggle(k)?.checked)).toEqual(BM_KINDS);

    click(q('[data-matrix-edit="FINANCE"]'));
    await settle();
    expect(KINDS.filter((k) => toggle(k)?.checked)).toEqual(FIN_KINDS);
  });

  it('A3 switching one kind on and saving sends every kind as set, the module permissions unchanged, and no ApprovalsDecide', async () => {
    await openEdit('BRANCH_MANAGER');
    click(toggle('ApproveCountDifference'));
    await settle();
    expect(toggle('ApproveCountDifference')?.checked).toBe(true);
    const body = await saveAndBody();
    expect(body).toEqual({
      code: 'BRANCH_MANAGER',
      // the view's permission order; no-module ones other than the nine are never sent
      permissions: [
        'OverviewRead', 'ExchangeTransact', 'TransactionRead',
        'ApproveCashTransferWithinLimit', 'ApproveCountDifference', 'ApproveNegativeStock', 'ApproveExchangeTransaction',
      ],
    });

    // and switching one off drops exactly that one
    calls = [];
    click(q('[data-matrix-edit="FINANCE"]'));
    await settle();
    click(toggle('ApproveTransactionReversal'));
    await settle();
    const fin = await saveAndBody();
    expect(fin).toEqual({ code: 'FINANCE', permissions: ['OverviewRead', 'CashLimitsManage', 'ApproveCountDifference'] });
    expect(fin.permissions).not.toContain('ApprovalsDecide');
  });

  it('A4 an ADMIN-based group: the nine are unavailable with a worded reason, and a save never sends any of them (rule 6)', async () => {
    await openEdit('ADMIN');
    for (const kind of KINDS) {
      expect(toggle(kind), kind).not.toBeNull();
      expect(toggle(kind)!.disabled, `${kind} disabled`).toBe(true);
      expect(cell('ADMIN', kind)?.textContent, `ADMIN:${kind}`).toContain(th.matrixApprovalUnavailable);
      expect(cell('IT_HELP', kind)?.textContent, `IT_HELP:${kind}`).toContain(th.matrixApprovalUnavailable);
    }
    expect(q('[data-matrix-approvals-edit] [data-matrix-approvals-unavailable]')?.textContent).toContain(
      th.matrixApprovalsAdminReason,
    );
    click(toggle('ApproveNegativeStock'));
    await settle();
    const body = await saveAndBody();
    expect(body).toEqual({ code: 'ADMIN', permissions: ['OverviewRead', 'UsersManage'] });

    // a custom ADMIN-based group whose row lists a kind: still unavailable, and the save drops it
    calls = [];
    click(q('[data-matrix-edit="IT_HELP"]'));
    await settle();
    expect(toggle('ApproveNegativeStock')?.disabled).toBe(true);
    expect(toggle('ApproveNegativeStock')?.checked).toBe(false);
    const itHelp = await saveAndBody();
    expect(itHelp).toEqual({ code: 'IT_HELP', permissions: ['UsersManage'] });
  });

  it('A5 the built-in EXECUTIVE group is read-only: no edit, no toggle, its four shown held; saving another group does not touch it (rule 4)', async () => {
    mount();
    await settle();
    expect(q('[data-matrix-edit="EXECUTIVE"]')).toBeNull();
    for (const kind of EXEC_KINDS) expect(cell('EXECUTIVE', kind)?.getAttribute('data-held'), kind).toBe('true');
    // the overview holds no control at all
    expect(qa('[data-matrix-approvals] input')).toHaveLength(0);
    expect(qa('[data-matrix-approvals] button')).toHaveLength(0);

    click(q('[data-matrix-edit="BRANCH_MANAGER"]'));
    await settle();
    click(toggle('ApproveCashTransferAboveLimit'));
    await settle();
    const body = await saveAndBody();
    expect(body.code).toBe('BRANCH_MANAGER');
    expect(writes('/api/admin/groups/save').every((c) => (c.body as { code: string }).code !== 'EXECUTIVE')).toBe(true);
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
  });

  it('A6 a refusal (409 guardrail:6) is shown in words in role="alert", never the raw message, and the toggles keep the edit', async () => {
    handler = (u) => {
      if (u.pathname === '/api/admin/groups/save') {
        return json(409, { error: { code: 'guardrail:6', message: 'RAW guardrail 6 text' } });
      }
      return json(200, { version: viewData.version, changed: true, data: viewData });
    };
    await openEdit('BRANCH_MANAGER');
    click(toggle('ApproveCountDifference'));
    await settle();
    click(toggle('ApproveNegativeStock'));
    await settle();
    click(q('[data-matrix-save]'));
    await settle();
    const err = q('[role="alert"][data-matrix-error]');
    expect(err, 'a worded alert').not.toBeNull();
    expect(err!.textContent ?? '').not.toContain('RAW guardrail 6 text');
    expect(err!.textContent).toContain(th.matrixGuardrail_6);
    // the panel is still open with the user's choices
    expect(q('[data-matrix-approvals-edit]')).not.toBeNull();
    expect(toggle('ApproveCountDifference')?.checked).toBe(true);
    expect(toggle('ApproveNegativeStock')?.checked).toBe(false);
    expect(toggle('ApproveCashTransferWithinLimit')?.checked).toBe(true);
  });

  it('A7 a save that changes only a module permission keeps the group\'s approval kinds (today it strips them)', async () => {
    await openEdit('BRANCH_MANAGER');
    click(q('input[data-matrix-perm="OverviewRead"]'));
    await settle();
    const body = await saveAndBody();
    expect(body.code).toBe('BRANCH_MANAGER');
    expect(body.permissions).toEqual([
      'ExchangeTransact', 'TransactionRead', 'ApproveCashTransferWithinLimit', 'ApproveNegativeStock',
      'ApproveExchangeTransaction',
    ]);
    for (const kind of BM_KINDS) expect(body.permissions, kind).toContain(kind);
    expect(body.permissions).not.toContain('ApprovalsDecide');
  });

  it('A8 every new key is in TH and EN, and the TH labels are Thai', () => {
    const t = th as Record<string, string>;
    const e = en as Record<string, string>;
    const keys = [
      'matrixGuardrail_6', 'matrixApprovalsTitle', 'matrixApprovalHeld', 'matrixApprovalNotHeld',
      'matrixApprovalUnavailable', 'matrixApprovalsAdminReason', ...KINDS.map((k) => `matrixApproval_${k}`),
    ];
    for (const k of keys) {
      expect(t[k], `th ${k}`).toBeTruthy();
      expect(e[k], `en ${k}`).toBeTruthy();
      expect(t[k], `th ${k} is Thai`).toMatch(/[฀-๿]/);
    }
  });
});
