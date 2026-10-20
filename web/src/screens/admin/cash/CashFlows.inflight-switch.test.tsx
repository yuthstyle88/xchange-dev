// #2515: admin cash flows — a write still in flight at a branch switch must not land its
// approval banner, refusal banner / toast, or create-form clear on the new branch.
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
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast, toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

type Answer = { status: number; body: unknown } | 'network';

interface DeferredPost {
  path: string;
  body: Record<string, unknown>;
  resolve: (answer: Answer) => void;
}

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
  heldPosts = new Set<string>();
  pendingPosts: DeferredPost[] = [];
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
      const body = JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>;
      this.posts.push({ path: url.pathname, body });
      if (this.heldPosts.has(url.pathname)) {
        const heldReply = await new Promise<Answer>((resolve) => {
          this.pendingPosts.push({ path: url.pathname, body, resolve });
        });
        if (heldReply === 'network') throw new TypeError('Failed to fetch');
        return json(heldReply.status, heldReply.body);
      }
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

describe('CashFlows: write in flight at a branch switch (#2515)', () => {
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

  const verifyIds = () =>
    [...root.querySelectorAll<HTMLElement>('[data-action="verify"]')].map((el) => el.dataset.id);

  function choose(selector: string, value: string) {
    const el = root.querySelector<HTMLSelectElement>(selector)!;
    el.value = value;
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function typeInput(selector: string, value: string) {
    const el = root.querySelector<HTMLInputElement>(selector)!;
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  beforeEach(() => {
    setLang('th');
    toast('');
    server = new Server();
    vi.stubGlobal('fetch', vi.fn(server.fetch));
  });

  afterEach(async () => {
    await pendingWrite.abandon();
    closeDialog();
    toast('');
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  // Row 1: Verify in A held, switch to B, answer AWAITING_FINANCE -> No approval banner on B
  it('row 1: Verify in A held, switch to B, answer AWAITING_FINANCE -> no approval banner on B and busy ends', async () => {
    server.heldPosts.add('/api/cash-counts/cc-A/verify');

    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-A"]')!.click();
    await vi.waitFor(() => expect(server.pendingPosts.length).toBe(1));

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    server.pendingPosts[0]!.resolve({
      status: 200,
      body: { id: 'cc-A', status: 'AWAITING_FINANCE', approvalRequestId: 'ar-A' },
    });
    await settle();

    expect(root.querySelector('.cash-flows .stock-banner')).toBeNull();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(
      root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-B"]')!.disabled,
    ).toBe(false);
  });

  // Row 2: Create in A held, switch to B, answer is a refusal (4xx) -> No refusal banner and no toast on B
  it('row 2: Create in A held, switch to B, answer is a refusal (4xx) -> no refusal banner and no toast on B', async () => {
    server.heldPosts.add('/api/cash-transfers');

    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    choose('#transfer-kind', 'EXTERNAL_IN');
    typeInput('#transfer-note', 'note-on-A');
    root.querySelector<HTMLButtonElement>('[data-action="create"]')!.click();
    await vi.waitFor(() => expect(server.pendingPosts.length).toBe(1));

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    server.pendingPosts[0]!.resolve({
      status: 409,
      body: { error: { code: 'shiftNotClosed', message: 'A drawer shift is closed' } },
    });
    await settle();

    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
    expect(root.querySelector<HTMLButtonElement>('[data-action="create"]')!.disabled).toBe(false);
  });

  // Row 3: Create in A held, switch to B, answer OK -> B's create-form note and quantities are not cleared
  it("row 3: Create in A held, switch to B, answer OK -> B's create-form note and quantities are not cleared", async () => {
    server.heldPosts.add('/api/cash-transfers');

    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    choose('#transfer-kind', 'EXTERNAL_IN');
    typeInput('input[data-denomination="1"]', '5');
    typeInput('#transfer-note', 'note-on-A');
    root.querySelector<HTMLButtonElement>('[data-action="create"]')!.click();
    await vi.waitFor(() => expect(server.pendingPosts.length).toBe(1));

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    typeInput('input[data-denomination="1"]', '12');
    typeInput('#transfer-note', 'draft-note-on-B');

    server.pendingPosts[0]!.resolve({
      status: 200,
      body: { id: 'tr-A-created', status: 'DRAFT' },
    });
    await settle();

    expect(root.querySelector<HTMLInputElement>('input[data-denomination="1"]')!.value).toBe('12');
    expect(root.querySelector<HTMLInputElement>('#transfer-note')!.value).toBe('draft-note-on-B');
    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
    expect(root.querySelector<HTMLButtonElement>('[data-action="create"]')!.disabled).toBe(false);
  });

  // Row 4 + Edge cases: network error in A after switch, and A->B->A while write is in flight
  it('row 4 & edge cases: network error in A after switch shows no refusal on B; A->B->A while write is in flight applies on A', async () => {
    server.heldPosts.add('/api/cash-counts/cc-A/verify');

    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    // 1. Network error in A after switching to B -> no refusal on B
    root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-A"]')!.click();
    await vi.waitFor(() => expect(server.pendingPosts.length).toBe(1));

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    server.pendingPosts[0]!.resolve('network');
    await settle();

    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');

    // 2. Switch back to A, start Verify in A, switch A->B->A while in flight -> answer applies on A
    setBranch('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    root.querySelector<HTMLButtonElement>('[data-action="verify"][data-id="cc-A"]')!.click();
    await vi.waitFor(() => expect(server.pendingPosts.length).toBe(2));

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));
    setBranch('A');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-A']));

    server.pendingPosts[1]!.resolve({
      status: 200,
      body: { id: 'cc-A', status: 'AWAITING_FINANCE', approvalRequestId: 'ar-A-roundtrip' },
    });
    await settle();

    expect(root.querySelector('.cash-flows .stock-banner')?.textContent).toContain(
      'ar-A-roundtrip',
    );
  });
});
