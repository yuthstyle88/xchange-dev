// R3-4b (docs/design/web-silent-failures.md §7.2): a failed `me` read in CashFlows sets only
// `meError` (never `refusal`), shown with the shared `PermissionsUnavailable` line whose retry
// re-reads `me`; a branch switch keeps it. Up-front tests: every row is red at 36abb596, where the
// failure goes into `refusal` (wiped by `clearBranchState`) and `me` is never read again.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import {
  makeCashCountRow,
  makeCashCountTotalRow,
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeMe,
} from '../../../shared/testing/fixtures';
import { permissionsUnavailableText } from '../../../shared/ui/PermissionsUnavailable';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

const VERIFY = 'ShiftCloseCertify';
const MANAGE = 'CashManage';

/** The `me` failure's own server sentence, so its text is told apart from any other refusal. */
const ME_MSG = 'อ่านสิทธิ์ผู้ใช้ไม่ได้ชั่วคราว';
const ME_REPLY = { status: 400, code: 'unkeyedMeCode', message: ME_MSG };

const COUNT = makeCashCountRow({
  id: 'cc-1',
  branchId: 'MAIN',
  shiftId: 'S-01',
  kind: 'CLOSE',
  countedBy: 'EMP-1',
  countedAt: '2026-10-15T01:00:00Z',
  verifiedBy: null,
  verifiedAt: null,
  note: null,
  totals: [makeCashCountTotalRow({ currency: 'THB', counted: '1000', expected: '1000', difference: '0' })],
});

const DRAFT = makeCashTransferRow({
  id: 'tr-draft',
  kind: 'EXTERNAL_OUT',
  status: 'DRAFT',
  fromBranchId: 'MAIN',
  toBranchId: null,
  createdBy: 'EMP-1',
  createdAt: '2026-10-15T01:00:00Z',
  lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null })],
});

const RECEIVED = makeCashTransferRow({
  id: 'tr-received',
  kind: 'BRANCH',
  status: 'RECEIVED',
  fromBranchId: 'MAIN',
  toBranchId: 'B02',
  createdBy: 'EMP-1',
  createdAt: '2026-10-15T02:00:00Z',
  lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: '10' })],
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

interface Sent {
  path: string;
  branch: string | null;
}

class Server {
  meFails = true;
  /** What a successful `me` grants: count verify and transfers, but not reconcile. */
  permissions = [VERIFY, MANAGE];
  sent: Sent[] = [];

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    this.sent.push({ path: u.pathname, branch: u.searchParams.get('branch') });
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method !== 'GET') return json(404, { error: { code: 'notFound', message: `unexpected ${method}` } });
    switch (u.pathname) {
      case '/api/v3/views/me':
        if (this.meFails) return json(ME_REPLY.status, { error: { code: ME_REPLY.code, message: ME_REPLY.message } });
        return view(makeMe({ id: 'u-admin', permissions: this.permissions, branch: 'MAIN' }));
      case '/api/v3/views/cash_counts':
        return view([COUNT]);
      case '/api/v3/views/cash_transfers':
        return view([DRAFT, RECEIVED]);
      default:
        return view({ rows: [] });
    }
  };

  reads(view: string): Sent[] {
    return this.sent.filter((s) => s.path === `/api/v3/views/${view}`);
  }
}

let server: Server;
let root: HTMLElement;
let dispose: (() => void) | undefined;
let setBranchSignal: Setter<string>;

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<HTMLElement> {
  const [branch, setBranch] = createSignal('MAIN');
  setBranchSignal = setBranch;
  const [shown] = createSignal(true);
  const [shifts] = createSignal([]);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () => (
      <>
        <CashFlows branch={branch} shown={shown} shifts={shifts} />
        <Dialog />
        <Toast />
      </>
    ),
    root,
  );
  await settle();
  return root.querySelector<HTMLElement>('.cash-flows')!;
}

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  await settle();
  await pendingWrite.abandon();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

const expectedLine = () => permissionsUnavailableText(new ApiError(ME_REPLY));
const permLine = (s: ParentNode) => s.querySelector<HTMLElement>('[data-permissions-unavailable]');
const retryButton = (s: ParentNode) => s.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');
const button = (s: ParentNode, action: string, id?: string) =>
  s.querySelector<HTMLButtonElement>(`button[data-action="${action}"]${id ? `[data-id="${id}"]` : ''}`);
/** Every role="alert" in CashFlows whose text carries the `me` failure. */
const meAlerts = (s: ParentNode) =>
  Array.from(s.querySelectorAll<HTMLElement>('[role="alert"]')).filter((a) => (a.textContent ?? '').includes(ME_MSG));

describe('CashFlows me failure: the shared line with a retry (R3-4b)', () => {
  it('(a) a failed me read shows the shared PermissionsUnavailable line with a retry', async () => {
    const s = await mount();

    const line = permLine(s);
    expect(line, 'the PermissionsUnavailable line').not.toBeNull();
    expect(line!.getAttribute('role')).toBe('alert');
    expect(line!.textContent).toContain(expectedLine());
    expect(retryButton(s), 'its retry').not.toBeNull();
  });

  it('(b) after a branch switch the line is still shown', async () => {
    const s = await mount();

    setBranchSignal('B02');
    await settle();

    expect(server.reads('cash_counts').map((r) => r.branch)).toContain('B02');
    const line = permLine(s);
    expect(line, 'the PermissionsUnavailable line after the switch').not.toBeNull();
    expect(line!.textContent).toContain(expectedLine());
  });

  it('(c) a retry with me succeeding removes the line, enables the gated buttons per permissions, and the per-branch reads run', async () => {
    const s = await mount();
    const retry = retryButton(s);
    expect(retry, 'its retry').not.toBeNull();

    server.meFails = false;
    const meReadsBefore = server.reads('me').length;
    retry!.click();
    await settle();

    expect(server.reads('me').length).toBe(meReadsBefore + 1);
    expect(permLine(s)).toBeNull();
    expect(meAlerts(s)).toHaveLength(0);

    const verify = button(s, 'verify', 'cc-1');
    expect(verify, 'verify (ShiftCloseCertify held)').not.toBeNull();
    expect(verify!.disabled).toBe(false);
    expect(verify!.title).toBe('');
    const submit = button(s, 'submit', 'tr-draft');
    expect(submit, 'submit (CashManage held)').not.toBeNull();
    expect(submit!.disabled).toBe(false);
    expect(submit!.title).toBe('');
    expect(button(s, 'reconcile', 'tr-received'), 'reconcile hidden (FinanceReconcile not held)').toBeNull();

    expect(server.reads('cash_counts').filter((r) => r.branch === 'MAIN').length).toBeGreaterThan(0);
    expect(server.reads('cash_transfers').filter((r) => r.branch === 'MAIN').length).toBeGreaterThan(0);
  });

  it('(d) refusal is not used for the me failure: before and after a branch switch, the only alert carrying it is the shared line', async () => {
    const s = await mount();

    const before = meAlerts(s);
    expect(before, 'one alert carries the me failure').toHaveLength(1);
    expect(before[0]!.hasAttribute('data-permissions-unavailable'), 'and it is the shared line, not refusal').toBe(true);

    setBranchSignal('B02');
    await settle();

    const after = meAlerts(s);
    expect(after, 'after the switch, one alert carries the me failure').toHaveLength(1);
    expect(after[0]!.hasAttribute('data-permissions-unavailable')).toBe(true);
    expect(
      Array.from(s.querySelectorAll('.error-message')).some((e) => (e.textContent ?? '').includes(ME_MSG)),
      'no me text in the refusal area',
    ).toBe(false);
  });
});
