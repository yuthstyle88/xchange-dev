// #2417: admin cash flows after a branch switch. The previous branch's counts, transfers, denominations, destination
// branches and form choices must not stay on screen or reach a write sent under the new branch, and a late answer
// for the previous branch must not land over the current one. Setup copied from `CashFlows.rework.test.tsx`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import {
  makeCashCountRow,
  makeCashCountTotalRow,
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeTransferBranchRow,
  type TransferBranchRow,
} from '../../../shared/testing/fixtures';
import { Dialog, closeDialog } from '../parts/Dialog';
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

function transfer(branch: string) {
  return makeCashTransferRow({
    id: `tr-${branch}`,
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
  });
}

/** Both branches count the same THB 1000 note, so a quantity typed in A would still match a row in B. */
const DENOMS = [makeDenominationRow({ id: 1, currency: 'THB', faceValue: '1000', groupCode: 'NOTE' })];

const DESTS: Record<string, TransferBranchRow[]> = {
  A: [makeTransferBranchRow({ branchId: 'X', name: 'Branch X' })],
  B: [makeTransferBranchRow({ branchId: 'Y', name: 'Branch Y' })],
};

class Server {
  /** Fixed answers by `view:branch`; anything absent answers the branch's normal rows. */
  answers = new Map<string, Answer>();
  /** Held answers by `view:branch` (or `me`): the read waits until the hold is released. */
  holds = new Map<string, Promise<void>>();
  private releases: (() => void)[] = [];
  reads: { view: string; branch: string | null }[] = [];
  posts: { path: string; body: Record<string, unknown> }[] = [];

  hold(key: string): () => void {
    let release!: () => void;
    this.holds.set(key, new Promise<void>((r) => (release = r)));
    this.releases.push(release);
    return release;
  }

  releaseAll() {
    for (const r of this.releases) r();
  }

  private data(view: string, branch: string): unknown {
    switch (view) {
      case 'cash_counts':
        return [count(branch)];
      case 'cash_transfers':
        return [transfer(branch)];
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
      return json(200, { id: 'tr-new', status: 'DRAFT' });
    }

    const view = url.pathname.startsWith(VIEWS) ? url.pathname.slice(VIEWS.length) : url.pathname;
    const branch = url.searchParams.get('branch');

    if (view === 'me') {
      await this.holds.get('me');
      return json(200, {
        version: 1,
        changed: true,
        data: {
          id: 'u-admin',
          name: 'Admin',
          roles: ['ADMIN'],
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
    await this.holds.get(key);
    const fixed = this.answers.get(key);
    if (fixed === 'network') throw new TypeError('Failed to fetch');
    if (fixed) return json(fixed.status, fixed.body);
    return json(200, { version: 1, changed: true, data: this.data(view, branch ?? '') });
  };
}

async function settle(rounds = 5) {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('CashFlows: branch switch (#2417)', () => {
  let server: Server;
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  function mount(branch = 'A') {
    const [b, setBranch] = createSignal(branch);
    const [shown, setShown] = createSignal(true);
    const [shifts] = createSignal([]);
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
    return { setBranch, setShown };
  }

  const verifyIds = () => [...root.querySelectorAll<HTMLElement>('[data-action="verify"]')].map((el) => el.dataset.id);
  const transfersText = () => root.querySelector('[data-section="transfers"]')?.textContent ?? '';
  const destinationValues = () =>
    [...root.querySelectorAll<HTMLOptionElement>('#transfer-to-branch option')].map((o) => o.value).filter((v) => v);
  const alertText = () => (root.querySelector('.cash-flows [role="alert"]')?.textContent ?? '').trim();

  function choose(selector: string, value: string) {
    const el = root.querySelector<HTMLSelectElement>(selector)!;
    el.value = value;
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  /** Mount on A, wait for A's rows, and pick a branch transfer so the destination list is on screen. */
  async function mountOnA() {
    const handles = mount('A');
    await vi.waitFor(() => {
      expect(verifyIds()).toEqual(['cc-A']);
      expect(transfersText()).toContain('note-A');
    });
    choose('#transfer-kind', 'BRANCH');
    await vi.waitFor(() => expect(destinationValues()).toEqual(['X']));
    return handles;
  }

  beforeEach(() => {
    setLang('th');
    server = new Server();
    vi.stubGlobal('fetch', vi.fn(server.fetch));
  });

  afterEach(async () => {
    server.releaseAll();
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

  // Row 1: A → B, before B's reads answer: no A count / transfer rows and no A destination choices.
  it("shows none of A's counts, transfers or destinations while B's reads are still out", async () => {
    const { setBranch } = await mountOnA();
    for (const view of ['cash_counts', 'cash_transfers', 'denominations', 'transfer_branches']) server.hold(`${view}:B`);

    setBranch('B');
    await settle();

    expect(verifyIds()).toEqual([]);
    expect(transfersText()).not.toContain('note-A');
    expect(destinationValues()).toEqual([]);
  });

  // Row 2: A → B, A's counts read answers after B's: B's rows are shown.
  it("keeps B's counts when A's counts answer arrives after B's", async () => {
    const releaseA = server.hold('cash_counts:A');
    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(transfersText()).toContain('note-A'));

    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    releaseA();
    await settle();

    expect(verifyIds()).toEqual(['cc-B']);
  });

  // Row 3: A → B, B's reads fail: A's rows are not shown; the worded error shows.
  it("shows the worded error and none of A's rows when B's reads fail", async () => {
    const { setBranch } = await mountOnA();
    server.answers.set('cash_counts:B', 'network');
    server.answers.set('cash_transfers:B', 'network');
    // B's form reads are still out when the failures land.
    server.hold('denominations:B');
    server.hold('transfer_branches:B');

    setBranch('B');
    await vi.waitFor(() => expect(alertText()).toBe(sharedTh['error.network']));

    expect(verifyIds()).toEqual([]);
    expect(transfersText()).not.toContain('note-A');
    expect(destinationValues()).toEqual([]);
  });

  // Row 4: choose destination X in A, switch to B, Create: the form is empty; no POST carries A's destination.
  it("empties the create form on a switch and Create does not send A's destination or quantities", async () => {
    const { setBranch } = await mountOnA();
    choose('#transfer-to-branch', 'X');
    const qty = root.querySelector<HTMLInputElement>('#transfer-line-1')!;
    qty.value = '5';
    qty.dispatchEvent(new Event('input', { bubbles: true }));

    setBranch('B');
    await vi.waitFor(() => {
      expect(destinationValues()).toEqual(['Y']);
      expect(verifyIds()).toEqual(['cc-B']);
    });

    expect(root.querySelector<HTMLSelectElement>('#transfer-to-branch')!.value).toBe('');
    expect(root.querySelector<HTMLInputElement>('#transfer-line-1')!.value).toBe('');

    root.querySelector<HTMLButtonElement>('[data-action="create"]')!.click();
    await vi.waitFor(() => expect(server.posts.length).toBe(1));

    const body = server.posts[0]!.body;
    expect(body.branch).toBe('B');
    expect(body.toBranchId).not.toBe('X');
    expect(body.toBranchId).toBeNull();
    expect(body.lines).toEqual([]);
  });

  // Edge: a receive dialog open during the switch is closed.
  it('closes an open receive dialog when the branch changes', async () => {
    const { setBranch } = mount('A');
    await vi.waitFor(() => expect(root.querySelector('[data-action="receive"][data-id="tr-A"]')).not.toBeNull());
    root.querySelector<HTMLButtonElement>('[data-action="receive"][data-id="tr-A"]')!.click();
    await vi.waitFor(() => expect(document.querySelector('.cash-transfer-dialog')).not.toBeNull());

    setBranch('B');
    await settle();

    expect(document.querySelector('.cash-transfer-dialog')).toBeNull();
  });

  // Edge: the same branch shown again does not clear or re-read (the `branch === lastBranch` return stays).
  it('neither clears nor re-reads when the same branch is shown again', async () => {
    const { setShown } = await mountOnA();
    choose('#transfer-to-branch', 'X');
    const readsBefore = server.reads.length;

    setShown(false);
    await settle();
    setShown(true);
    await settle();

    expect(server.reads.length).toBe(readsBefore);
    expect(verifyIds()).toEqual(['cc-A']);
    expect(transfersText()).toContain('note-A');
    expect(root.querySelector<HTMLSelectElement>('#transfer-to-branch')!.value).toBe('X');
  });

  // Edge: while permissions are loading (`held === null`) no read is sent; reads then go to the current branch only.
  it('waits for me before reading, then reads only the current branch', async () => {
    const releaseMe = server.hold('me');
    const { setBranch } = mount('A');
    await settle();
    setBranch('B');
    await settle();
    expect(server.reads).toEqual([]);

    releaseMe();
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));
    expect(server.reads.every((r) => r.branch === 'B')).toBe(true);
  });
});
