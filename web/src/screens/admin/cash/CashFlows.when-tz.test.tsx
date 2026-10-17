// #2521: Admin cash flows — count (countedAt) and movement (createdAt) times are formatted
// in Asia/Bangkok regardless of the device timezone.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { TransferBranchRow } from '../../../shared/api/generated/transfer_branches';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

const VIEWS = '/api/v3/views/';
const ISO_A = '2026-10-15T18:30:00Z';
const ISO_B = '2026-10-16T19:45:00Z';

const DENOMS: DenominationRow[] = [{ id: 1, currency: 'THB', faceValue: '1000', groupCode: 'NOTE' }];

const DESTS: Record<string, TransferBranchRow[]> = {
  A: [{ branchId: 'X', name: 'Branch X' }],
  B: [{ branchId: 'Y', name: 'Branch Y' }],
};

function countFor(branch: string): CashCountRow {
  return {
    id: `cc-${branch}`,
    branchId: branch,
    shiftId: `S-${branch}`,
    kind: 'CLOSE',
    countedBy: `EMP-${branch}`,
    countedAt: branch === 'B' ? ISO_B : ISO_A,
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [{ currency: 'THB', counted: '1000', expected: '1000', difference: '0' }],
  };
}

function transferFor(branch: string): CashTransferRow {
  return {
    id: `tr-${branch}`,
    kind: 'EXTERNAL_IN',
    status: 'DISPATCHED',
    fromBranchId: branch,
    toBranchId: null,
    fromShiftId: null,
    toShiftId: null,
    createdBy: `EMP-${branch}`,
    createdAt: branch === 'B' ? ISO_B : ISO_A,
    note: `note-${branch}`,
    lines: [{ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null }],
  };
}

describe('#2521: CashFlows formats countedAt and createdAt in Asia/Bangkok', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    setLang('th');

    vi.stubGlobal('fetch', async (input: RequestInfo | URL): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, 'http://localhost');
      const view = url.pathname.startsWith(VIEWS) ? url.pathname.slice(VIEWS.length) : url.pathname;
      const branch = url.searchParams.get('branch') ?? 'A';
      const json = (body: unknown) =>
        new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });

      if (view === 'me') {
        return json({
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
      if (view === 'cash_counts') {
        return json({ version: 1, changed: true, data: [countFor(branch)] });
      }
      if (view === 'cash_transfers') {
        return json({ version: 1, changed: true, data: [transferFor(branch)] });
      }
      if (view === 'denominations') {
        return json({ version: 1, changed: true, data: { rows: DENOMS } });
      }
      if (view === 'transfer_branches') {
        return json({ version: 1, changed: true, data: { rows: DESTS[branch] ?? [] } });
      }
      return json({ version: 1, changed: true, data: undefined });
    });
  });

  afterEach(async () => {
    await pendingWrite.abandon();
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function mount(initialBranch = 'A') {
    dispose?.();
    root?.remove();
    const [b, setBranch] = createSignal(initialBranch);
    const [shown] = createSignal(true);
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
    await vi.waitFor(() => {
      expect(root?.querySelector('.counts-table tbody tr')).not.toBeNull();
      expect(root?.querySelector('.transfers-table tbody tr')).not.toBeNull();
    });
    return { setBranch };
  }

  function countTimeText(): string {
    const cells = root?.querySelectorAll<HTMLTableCellElement>('.counts-table tbody tr td');
    expect(cells && cells.length >= 3, 'counts table row cells exist').toBe(true);
    return (cells![2]!.textContent ?? '').replace(/\s+/g, ' ').trim();
  }

  function transferTimeText(): string {
    const cell = root?.querySelector<HTMLTableCellElement>('.transfers-table tbody tr td');
    expect(cell, 'transfers table first cell exists').not.toBeNull();
    return (cell!.textContent ?? '').replace(/\s+/g, ' ').trim();
  }

  it('row 1: countedAt 2026-10-15T18:30:00Z with device TZ UTC shows 16/10/2569 01:30:00 in the count row', async () => {
    vi.stubEnv('TZ', 'UTC');
    expect(new Date(ISO_A).getHours()).toBe(18);

    await mount('A');

    const text = countTimeText();
    expect(text).toBe('16/10/2569 01:30:00');
    expect(text).not.toContain('15/10');
    expect(text).not.toContain('18:30');
  });

  it('row 2: createdAt 2026-10-15T18:30:00Z with device TZ UTC shows 16/10/2569 01:30:00 · EMP-A in the flow line', async () => {
    vi.stubEnv('TZ', 'UTC');
    expect(new Date(ISO_A).getHours()).toBe(18);

    await mount('A');

    const text = transferTimeText();
    expect(text).toBe('16/10/2569 01:30:00 · EMP-A');
    expect(text).not.toContain('15/10');
    expect(text).not.toContain('18:30');
  });

  it('row 3: same rows with device TZ Asia/Bangkok show the exact same text as with device TZ UTC', async () => {
    vi.stubEnv('TZ', 'UTC');
    expect(new Date(ISO_A).getHours()).toBe(18);
    await mount('A');
    const utcCount = countTimeText();
    const utcTransfer = transferTimeText();

    vi.stubEnv('TZ', 'Asia/Bangkok');
    expect(new Date(ISO_A).getHours()).toBe(1);
    await mount('A');
    const bkkCount = countTimeText();
    const bkkTransfer = transferTimeText();

    expect(bkkCount).toBe('16/10/2569 01:30:00');
    expect(bkkTransfer).toBe('16/10/2569 01:30:00 · EMP-A');
    expect(utcCount).toBe(bkkCount);
    expect(utcTransfer).toBe(bkkTransfer);
  });

  it('row 4: branch switch updates counts and transfers and formats the new branch times in Bangkok time under TZ=UTC', async () => {
    vi.stubEnv('TZ', 'UTC');
    const { setBranch } = await mount('A');
    expect(countTimeText()).toBe('16/10/2569 01:30:00');
    expect(transferTimeText()).toBe('16/10/2569 01:30:00 · EMP-A');

    setBranch('B');
    await vi.waitFor(() => {
      expect(root?.querySelector('[data-action="verify"]')?.getAttribute('data-id')).toBe('cc-B');
      expect(root?.querySelector('[data-section="transfers"]')?.textContent).toContain('note-B');
    });

    expect(countTimeText()).toBe('17/10/2569 02:45:00');
    expect(transferTimeText()).toBe('17/10/2569 02:45:00 · EMP-B');
  });
});
