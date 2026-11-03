// #6065: Admin cash flows — a transfer row names its creator by `createdByName`
// (user-set-settings.md:48: a missing or blank name is null, shown as "—"; never the raw id).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeTransferBranchRow,
} from '../../../shared/testing/fixtures';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

const VIEWS = '/api/v3/views/';
const ISO = '2026-10-15T18:30:00Z';
const SHOWN_AT = '16/10/2569 01:30:00';
const NAMED_ID = 'e7a9e6f0-1111-4aaa-8bbb-000000000001';
const UNNAMED_ID = 'e7a9e6f0-2222-4aaa-8bbb-000000000002';

const ROWS = [
  makeCashTransferRow({
    id: 'tr-named',
    kind: 'EXTERNAL_IN',
    status: 'DISPATCHED',
    fromBranchId: 'A',
    createdBy: NAMED_ID,
    createdByName: 'Malee Manager',
    createdAt: ISO,
    note: 'note-named',
    lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null })],
  }),
  makeCashTransferRow({
    id: 'tr-unnamed',
    kind: 'EXTERNAL_IN',
    status: 'DISPATCHED',
    fromBranchId: 'A',
    createdBy: UNNAMED_ID,
    createdByName: null,
    createdAt: ISO,
    note: 'note-unnamed',
    lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null })],
  }),
];

describe('#6065: CashFlows transfer row shows the creator name, never the id', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    setLang('th');

    vi.stubGlobal('fetch', async (input: RequestInfo | URL): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, 'http://localhost');
      const view = url.pathname.startsWith(VIEWS) ? url.pathname.slice(VIEWS.length) : url.pathname;
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
            branches: [{ id: 'A', code: 'A', name: 'Branch A' }],
          },
        });
      }
      if (view === 'cash_counts') return json({ version: 1, changed: true, data: [] });
      if (view === 'cash_transfers') return json({ version: 1, changed: true, data: ROWS });
      if (view === 'denominations') {
        return json({
          version: 1,
          changed: true,
          data: { rows: [makeDenominationRow({ id: 1, currency: 'THB', faceValue: '1000', groupCode: 'NOTE' })] },
        });
      }
      if (view === 'transfer_branches') {
        return json({ version: 1, changed: true, data: { rows: [makeTransferBranchRow({ branchId: 'X', name: 'Branch X' })] } });
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

  async function mount() {
    const [b] = createSignal('A');
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
      expect(root?.querySelectorAll('.transfers-table tbody tr').length).toBeGreaterThanOrEqual(2);
    });
  }

  function createdCell(note: string): string {
    const row = Array.from(root?.querySelectorAll<HTMLTableRowElement>('.transfers-table tbody tr') ?? []).find((tr) =>
      (tr.textContent ?? '').includes(note),
    );
    expect(row, `transfer row with ${note} exists`).toBeDefined();
    const cell = row!.querySelector<HTMLTableCellElement>('td');
    return (cell!.textContent ?? '').replace(/\s+/g, ' ').trim();
  }

  function transfersText(): string {
    return root?.querySelector('[data-section="transfers"]')?.textContent ?? '';
  }

  it('C1: createdByName set shows "<when> · <name>"', async () => {
    await mount();
    expect(createdCell('note-named')).toBe(`${SHOWN_AT} · Malee Manager`);
  });

  it('C2: createdByName null shows "<when> · —"', async () => {
    await mount();
    expect(createdCell('note-unnamed')).toBe(`${SHOWN_AT} · —`);
  });

  it('C3: the raw createdBy id appears nowhere in the transfers section', async () => {
    await mount();
    const text = transfersText();
    expect(text).toContain('Malee Manager');
    expect(text).not.toContain(NAMED_ID);
    expect(text).not.toContain(UNNAMED_ID);
    expect(text).not.toContain('e7a9e6f0');
  });

  it('C4: English shows the same form (the cell has no translated words)', async () => {
    setLang('en');
    await mount();
    expect(createdCell('note-named')).toBe(`${SHOWN_AT} · Malee Manager`);
    expect(createdCell('note-unnamed')).toBe(`${SHOWN_AT} · —`);
    expect(transfersText()).not.toContain('e7a9e6f0');
  });
});
