// #2432: admin cash flows after a branch switch — clear fromShift, toShift, note, refusal and awaiting while keeping
// kind, and reset ownDialog when a receive / reconcile dialog closes via Cancel or submit.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeAdminShiftRow,
  makeCashCountRow,
  makeCashCountTotalRow,
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeTransferBranchRow,
  type AdminShiftRow,
  type TransferBranchRow,
} from '../../../shared/testing/fixtures';
import { Dialog, closeDialog, openDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

type Answer = { status: number; body: unknown } | 'network';

const VIEWS = '/api/v3/views/';

function count(branch: string) {
  return makeCashCountRow({
    id: `cc-${branch}`,
    branchId: branch,
    shiftId: `S-${branch}`,
    kind: 'CLOSE',
    countedBy: `EMP-${branch}`,
    countedAt: '2026-10-15T01:00:00Z',
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [makeCashCountTotalRow({ currency: 'THB', counted: '1000', expected: '1000', difference: '0' })],
  });
}

function transfersFor(branch: string) {
  return [
    makeCashTransferRow({
      id: `tr-disp-${branch}`,
      kind: 'EXTERNAL_IN',
      status: 'DISPATCHED',
      fromBranchId: branch,
      toBranchId: null,
      fromShiftId: null,
      toShiftId: null,
      createdBy: `EMP-${branch}`,
      createdAt: '2026-10-15T01:00:00Z',
      note: `note-${branch}`,
      lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null })],
    }),
    makeCashTransferRow({
      id: `tr-recv-${branch}`,
      kind: 'EXTERNAL_IN',
      status: 'RECEIVED',
      fromBranchId: branch,
      toBranchId: null,
      fromShiftId: null,
      toShiftId: null,
      createdBy: `EMP-${branch}`,
      createdAt: '2026-10-15T02:00:00Z',
      note: `recv-${branch}`,
      lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: '9' })],
    }),
  ];
}

function shiftFor(branch: string) {
  return makeAdminShiftRow({
    id: `S-${branch}`,
    staff: `Staff ${branch}`,
    status: 'OPEN',
    openedAt: '2026-10-15T00:00:00Z',
    note: '',
    difference: null,
    balances: [],
  });
}

const DENOMS = [makeDenominationRow({ id: 1, currency: 'THB', faceValue: '1000', groupCode: 'NOTE' })];

const DESTS: Record<string, TransferBranchRow[]> = {
  A: [makeTransferBranchRow({ branchId: 'X', name: 'Branch X' })],
  B: [makeTransferBranchRow({ branchId: 'Y', name: 'Branch Y' })],
};

class Server {
  answers = new Map<string, Answer>();
  postAnswers = new Map<string, Answer>();
  reads: { view: string; branch: string | null }[] = [];
  posts: { path: string; body: Record<string, unknown> }[] = [];

  private data(view: string, branch: string): unknown {
    switch (view) {
      case 'cash_counts':
        return [count(branch)];
      case 'cash_transfers':
        return transfersFor(branch);
      case 'denominations':
        return { rows: DENOMS };
      case 'transfer_branches':
        return { rows: DESTS[branch] ?? [] };
      default:
        return undefined;
    }
  }

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const json = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

    if (method === 'POST') {
      this.posts.push({ path: url.pathname, body: JSON.parse(String(init?.body ?? '{}')) });
      const postReply = this.postAnswers.get(url.pathname);
      if (postReply === 'network') throw new TypeError('Failed to fetch');
      if (postReply) return json(postReply.status, postReply.body);
      return json(200, { id: 'tr-new', status: 'DRAFT' });
    }

    const view = url.pathname.startsWith(VIEWS) ? url.pathname.slice(VIEWS.length) : url.pathname;
    const branch = url.searchParams.get('branch');

    if (view === 'me') {
      return json(200, {
        version: 1,
        changed: true,
        data: {
          id: 'u-admin',
          name: 'Admin',
          roles: [],
          permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
          branch: 'A',
          branchName: 'Branch A',
          branches: [
            { id: 'A', code: 'A', name: 'Branch A' },
            { id: 'B', code: 'B', name: 'Branch B' },
          ],
        },
      });
    }

    this.reads.push({ view, branch });
    const key = `${view}:${branch}`;
    const fixed = this.answers.get(key);
    if (fixed === 'network') throw new TypeError('Failed to fetch');
    if (fixed) return json(fixed.status, fixed.body);
    return json(200, { version: 1, changed: true, data: this.data(view, branch ?? '') });
  };
}

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('CashFlows: branch switch form, banners and dialog ownership (#2432)', () => {
  let server: Server;
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  function mount(branch = 'A') {
    const [b, setBranch] = createSignal(branch);
    const [shown, setShown] = createSignal(true);
    const [shifts, setShifts] = createSignal<AdminShiftRow[]>([shiftFor(branch)]);
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () => (
        <>
          <CashFlows branch={b} shown={shown} shifts={shifts} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    const switchBranch = (next: string) => {
      setShifts([shiftFor(next)]);
      setBranch(next);
    };
    return { setBranch: switchBranch, setShown, setShifts };
  }

  const verifyIds = () => [...root.querySelectorAll<HTMLElement>('[data-action="verify"]')].map((el) => el.dataset.id);

  function choose(selector: string, value: string) {
    const el = root.querySelector<HTMLSelectElement>(selector)!;
    el.value = value;
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function typeNote(selector: string, value: string) {
    const el = root.querySelector<HTMLInputElement>(selector)!;
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  beforeEach(() => {
    setLang('th');
    server = new Server();
    vi.stubGlobal('fetch', vi.fn(server.fetch));
  });

  afterEach(async () => {
    await pendingWrite.abandon();
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  // Row 1 (Controller ruling): Choose kind DRAWER_TO_SAFE + from-shift S-A in A, switch to B, Create ->
  // no POST carries fromShiftId: "S-A"; the kind stays DRAWER_TO_SAFE (and likewise for SAFE_TO_DRAWER / toShift and note).
  it('row 1: Choose kind DRAWER_TO_SAFE + from-shift S-A in A, switch to B, Create -> no POST carries fromShiftId: "S-A"; the kind stays DRAWER_TO_SAFE', async () => {
    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    choose('#transfer-kind', 'DRAWER_TO_SAFE');
    await vi.waitFor(() => expect(root.querySelector('#transfer-from-shift')).not.toBeNull());
    choose('#transfer-from-shift', 'S-A');
    typeNote('#transfer-note', 'note-from-A');

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    expect(root.querySelector<HTMLSelectElement>('#transfer-kind')!.value).toBe('DRAWER_TO_SAFE');
    expect(root.querySelector<HTMLSelectElement>('#transfer-from-shift')!.value).toBe('');
    expect(root.querySelector<HTMLInputElement>('#transfer-note')!.value).toBe('');

    choose('#transfer-from-shift', 'S-B');
    root.querySelector<HTMLButtonElement>('[data-action="create"]')!.click();
    await vi.waitFor(() => expect(server.posts.length).toBe(1));

    expect(server.posts[0]!.body).toMatchObject({
      branch: 'B',
      kind: 'DRAWER_TO_SAFE',
      fromShiftId: 'S-B',
      toShiftId: null,
      note: '',
    });

    // Also verify SAFE_TO_DRAWER + toShift is cleared when switching back to A
    choose('#transfer-kind', 'SAFE_TO_DRAWER');
    await vi.waitFor(() => expect(root.querySelector('#transfer-to-shift')).not.toBeNull());
    choose('#transfer-to-shift', 'S-B');
    typeNote('#transfer-note', 'note-from-B');

    setBranch('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    expect(root.querySelector<HTMLSelectElement>('#transfer-kind')!.value).toBe('SAFE_TO_DRAWER');
    expect(root.querySelector<HTMLSelectElement>('#transfer-to-shift')!.value).toBe('');
    expect(root.querySelector<HTMLInputElement>('#transfer-note')!.value).toBe('');

    choose('#transfer-to-shift', 'S-A');
    root.querySelector<HTMLButtonElement>('[data-action="create"]')!.click();
    await vi.waitFor(() => expect(server.posts.length).toBe(2));

    expect(server.posts[1]!.body).toMatchObject({
      branch: 'A',
      kind: 'SAFE_TO_DRAWER',
      fromShiftId: null,
      toShiftId: 'S-A',
      note: '',
    });
  });

  // Row 2: An approval-request banner is shown in A, switch to B -> No approval banner
  it('row 2: An approval-request banner is shown in A, switch to B -> No approval banner', async () => {
    server.postAnswers.set('/api/cash-counts/cc-A/verify', {
      status: 200,
      body: { id: 'cc-A', status: 'AWAITING_FINANCE', approvalRequestId: 'APR-A-99' },
    });

    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-A"]')!.click();
    await vi.waitFor(() => {
      expect(root.querySelector('.cash-flows .stock-banner')?.textContent).toContain('APR-A-99');
    });

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    expect(root.querySelector('.cash-flows .stock-banner')).toBeNull();
  });

  // Row 3: A refusal banner is shown in A, switch to B -> No refusal banner
  it('row 3: A refusal banner is shown in A, switch to B -> No refusal banner', async () => {
    server.postAnswers.set('/api/cash-counts/cc-A/verify', {
      status: 409,
      body: { error: { code: 'shiftNotClosed', message: 'shift still open' } },
    });

    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-A"]')!.click();
    await vi.waitFor(() => {
      expect(root.querySelector('.cash-flows [role="alert"]')).not.toBeNull();
    });

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    expect(root.querySelector('.cash-flows [role="alert"]')).toBeNull();
  });

  // Row 4: Open a receive dialog, Cancel it, switch branch -> closeDialog is not called by the switch
  it('row 4: Open a receive (or reconcile) dialog, Cancel (or submit) it, switch branch -> closeDialog is not called by the switch', async () => {
    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    // 1. Receive dialog -> Cancel -> external dialog opened -> switch to B -> external dialog stays open
    root.querySelector<HTMLButtonElement>('[data-action="receive"][data-id="tr-disp-A"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).not.toBeNull());
    document.querySelector<HTMLButtonElement>('.cash-transfer-dialog button.secondary')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).toBeNull());

    openDialog('External 1', () => <div data-testid="external-dialog">ext-1</div>);
    await vi.waitFor(() => expect(document.querySelector('[data-testid="external-dialog"]')).not.toBeNull());

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));
    expect(document.querySelector('[data-testid="external-dialog"]')?.textContent).toBe('ext-1');
    closeDialog();
    await settle();

    // 2. Receive dialog -> Submit success -> external dialog opened -> switch to A -> external dialog stays open
    root.querySelector<HTMLButtonElement>('[data-action="receive"][data-id="tr-disp-B"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).not.toBeNull());
    document.querySelector<HTMLButtonElement>('.cash-transfer-dialog button[data-action="confirm"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).toBeNull());

    openDialog('External 2', () => <div data-testid="external-dialog">ext-2</div>);
    await vi.waitFor(() => expect(document.querySelector('[data-testid="external-dialog"]')).not.toBeNull());

    setBranch('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));
    expect(document.querySelector('[data-testid="external-dialog"]')?.textContent).toBe('ext-2');
    closeDialog();
    await settle();

    // 3. Reconcile dialog -> Cancel -> external dialog opened -> switch to B -> external dialog stays open
    root.querySelector<HTMLButtonElement>('[data-action="reconcile"][data-id="tr-recv-A"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).not.toBeNull());
    document.querySelector<HTMLButtonElement>('.cash-transfer-dialog button.secondary')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).toBeNull());

    openDialog('External 3', () => <div data-testid="external-dialog">ext-3</div>);
    await vi.waitFor(() => expect(document.querySelector('[data-testid="external-dialog"]')).not.toBeNull());

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));
    expect(document.querySelector('[data-testid="external-dialog"]')?.textContent).toBe('ext-3');
    closeDialog();
    await settle();

    // 4. Reconcile dialog -> Submit success -> external dialog opened -> switch to A -> external dialog stays open
    root.querySelector<HTMLButtonElement>('[data-action="reconcile"][data-id="tr-recv-B"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).not.toBeNull());
    document.querySelector<HTMLButtonElement>('.cash-transfer-dialog button[data-action="confirm"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).toBeNull());

    openDialog('External 4', () => <div data-testid="external-dialog">ext-4</div>);
    await vi.waitFor(() => expect(document.querySelector('[data-testid="external-dialog"]')).not.toBeNull());

    setBranch('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));
    expect(document.querySelector('[data-testid="external-dialog"]')?.textContent).toBe('ext-4');
  });

  // Edge cases (§9): Same branch re-selected clears nothing; a dialog still open at the switch is closed alongside form/banner clear.
  it('edge cases (§9): same branch re-selected clears nothing, while switching branch closes an open dialog and clears shift, note and banners', async () => {
    server.postAnswers.set('/api/cash-counts/cc-A/verify', {
      status: 200,
      body: { id: 'cc-A', status: 'AWAITING_FINANCE', approvalRequestId: 'APR-EDGE' },
    });

    const { setBranch, setShown } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    choose('#transfer-kind', 'DRAWER_TO_SAFE');
    await vi.waitFor(() => expect(root.querySelector('#transfer-from-shift')).not.toBeNull());
    choose('#transfer-from-shift', 'S-A');
    typeNote('#transfer-note', 'keep-on-same-branch');

    root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-A"]')!.click();
    await vi.waitFor(() => {
      expect(root.querySelector('.cash-flows .stock-banner')?.textContent).toContain('APR-EDGE');
    });

    // Re-showing the same branch A does not clear shift, note or banner
    setShown(false);
    await settle();
    setShown(true);
    await settle();

    expect(root.querySelector<HTMLSelectElement>('#transfer-from-shift')!.value).toBe('S-A');
    expect(root.querySelector<HTMLInputElement>('#transfer-note')!.value).toBe('keep-on-same-branch');
    expect(root.querySelector('.cash-flows .stock-banner')?.textContent).toContain('APR-EDGE');

    // Open receive dialog on A, then switch to B: dialog closes AND shift, note and banner clear
    root.querySelector<HTMLButtonElement>('[data-action="receive"][data-id="tr-disp-A"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).not.toBeNull());

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    expect(document.querySelector('.cash-transfer-dialog')).toBeNull();
    expect(root.querySelector<HTMLSelectElement>('#transfer-from-shift')!.value).toBe('');
    expect(root.querySelector<HTMLInputElement>('#transfer-note')!.value).toBe('');
    expect(root.querySelector('.cash-flows .stock-banner')).toBeNull();
  });
});
