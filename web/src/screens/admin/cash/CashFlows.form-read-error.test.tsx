// #2551: Admin cash flows: error handling for denominations and transfer_branches reads.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import type { AdminShiftRow } from '../../../shared/api/generated/admin_shifts';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { TransferBranchRow } from '../../../shared/api/generated/transfer_branches';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

type Answer = { status: number; body: unknown } | 'network' | (() => Promise<Response>);

const VIEWS = '/api/v3/views/';

function count(branch: string): CashCountRow {
  return {
    id: `cc-${branch}`,
    branchId: branch,
    shiftId: `S-${branch}`,
    kind: 'CLOSE',
    countedBy: `EMP-${branch}`,
    countedAt: '2026-10-15T01:00:00Z',
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [{ currency: 'THB', counted: '1000', expected: '1000', difference: '0' }],
  };
}

function transfersFor(branch: string): CashTransferRow[] {
  return [
    {
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
      lines: [{ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null }],
    },
  ];
}

function shiftFor(branch: string): AdminShiftRow {
  return {
    id: `S-${branch}`,
    staff: `Staff ${branch}`,
    status: 'OPEN',
    openedAt: '2026-10-15T00:00:00Z',
    note: '',
    difference: null,
    balances: [],
  };
}

const DENOMS: DenominationRow[] = [{ id: 1, currency: 'THB', faceValue: '1000', groupCode: 'NOTE' }];

const DESTS: Record<string, TransferBranchRow[]> = {
  A: [{ branchId: 'X', name: 'Branch X' }],
  B: [{ branchId: 'Y', name: 'Branch Y' }],
};

class Server {
  answers = new Map<string, Answer>();
  reads: { view: string; branch: string | null }[] = [];

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
    if (typeof fixed === 'function') {
      return await fixed();
    }
    if (fixed === 'network') throw new TypeError('Failed to fetch');
    if (fixed) return json(fixed.status, fixed.body);
    return json(200, { version: 1, changed: true, data: this.data(view, branch ?? '') });
  };
}

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('CashFlows: error handling for denominations and transfer_branches reads (#2551)', () => {
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

  // Row 1: denominations read answers 500 -> Refusal sentence shown
  it('row 1: denominations read answers 500 -> refusal sentence shown', async () => {
    server.answers.set('denominations:A', {
      status: 500,
      body: { error: { message: 'Internal Server Error' } },
    });
    mount('A');
    await vi.waitFor(() => {
      const alert = root.querySelector('.cash-flows [role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert?.textContent?.trim()).toBe(th['error.serverError']);
    });
    expect(root.querySelectorAll('.cash-new-transfer .transfer-lines > div').length).toBe(0);
  });

  // Row 2: transfer_branches read fails (network) -> Refusal sentence shown
  it('row 2: transfer_branches read fails (network) -> refusal sentence shown', async () => {
    server.answers.set('transfer_branches:A', 'network');
    mount('A');
    await vi.waitFor(() => {
      const alert = root.querySelector('.cash-flows [role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert?.textContent?.trim()).toBe(th['error.network']);
    });
    choose('#transfer-kind', 'BRANCH');
    await vi.waitFor(() => expect(root.querySelector('#transfer-to-branch')).not.toBeNull());
    const options = root.querySelectorAll('#transfer-to-branch option');
    expect(options.length).toBe(1); // only "choose branch"
  });

  // Row 3: Both succeed -> No refusal; form filled (unchanged)
  it('row 3: both succeed -> no refusal; form filled (unchanged)', async () => {
    mount('A');
    await vi.waitFor(() => {
      expect(verifyIds()).toEqual(['cc-A']);
    });
    expect(root.querySelector('.cash-flows [role="alert"]')).toBeNull();
    expect(root.querySelectorAll('.cash-new-transfer .transfer-lines > div').length).toBe(1);
    choose('#transfer-kind', 'BRANCH');
    await vi.waitFor(() => expect(root.querySelector('#transfer-to-branch')).not.toBeNull());
    const branchOptions = [...root.querySelectorAll('#transfer-to-branch option')].map(
      (o) => (o as HTMLOptionElement).value,
    );
    expect(branchOptions).toContain('X');
  });

  // Edge case (§9): A stale answer (seq changed) still sets nothing.
  it('edge case (§9): a stale answer (seq changed) still sets nothing', async () => {
    let rejectA: ((err: unknown) => void) | undefined;
    server.answers.set(
      'denominations:A',
      () =>
        new Promise((_, reject) => {
          rejectA = reject;
        }),
    );

    const { setBranch } = mount('A');
    // Wait until branch A's denominations read has actually been issued
    await vi.waitFor(() => expect(rejectA).toBeDefined());

    // Switch to branch B
    setBranch('B');
    await vi.waitFor(() => expect(verifyIds()).toEqual(['cc-B']));

    // Now reject A's stale read
    rejectA!(new TypeError('Failed to fetch'));
    await settle();

    // Stale rejection for A must not set refusal on B
    expect(root.querySelector('.cash-flows [role="alert"]')).toBeNull();
  });
});
