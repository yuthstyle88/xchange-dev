import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeCashTransferRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
} from '../../../shared/testing/fixtures';
import { TransfersCard } from './TransfersCard';
import type { StaffCtx } from '../cards';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000b1';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mockMe(permissions: string[] = ['CashManage', 'FinanceReconcile']) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function mockFloor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: {
      id: 'shift-open-001',
      openedAt: '2026-10-11T01:00:00Z',
    },
    cutoff: null,
    target: null,
    safe: null,
  });
}

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
];

const BRANCHES = [
  makeTransferBranchRow({ branchId: 'B99', name: 'Branch 99' }),
];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stub(rows: ReturnType<typeof makeCashTransferRow>[] = []) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && path.startsWith('/api/v3/views/')) {
        const viewName = path.slice('/api/v3/views/'.length);
        if (viewName === 'cash_transfers') {
          return json(200, { version: 1, changed: true, data: rows });
        }
        if (viewName === 'denominations') {
          return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        }
        if (viewName === 'transfer_branches') {
          return json(200, { version: 1, changed: true, data: { rows: BRANCHES } });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      return json(200, { id: 'tx-new' });
    }),
  );
}

async function mount(rows: ReturnType<typeof makeCashTransferRow>[] = []) {
  stub(rows);
  const ctx: StaffCtx = {
    me: mockMe(),
    floor: mockFloor(),
    refresh: vi.fn(async () => {}),
  };
  dispose = render(() => <TransfersCard ctx={ctx} />, host);
  for (let i = 0; i < 5; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('TransfersCard worded statuses, short ids, and table header (#5205)', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.restoreAllMocks();
  });

  it('T1: a row with status RECONCILED reads "กระทบยอดแล้ว" and not "RECONCILED"', async () => {
    const row = makeCashTransferRow({
      id: 'tx-001',
      status: 'RECONCILED',
    });
    await mount([row]);

    const tr = host.querySelector('tr[data-transfer-id="tx-001"]')!;
    expect(tr).not.toBeNull();
    const cells = tr.querySelectorAll('td');
    expect(cells[2].textContent).toBe('กระทบยอดแล้ว');
    expect(cells[2].textContent).not.toContain('RECONCILED');
  });

  it('T2: each of the 5 statuses shows its label in TH and EN', async () => {
    const statuses = [
      { code: 'DRAFT', th: 'ร่าง', en: 'Draft' },
      { code: 'APPROVED', th: 'อนุมัติแล้ว', en: 'Approved' },
      { code: 'DISPATCHED', th: 'ส่งออกแล้ว', en: 'Dispatched' },
      { code: 'RECEIVED', th: 'รับแล้ว', en: 'Received' },
      { code: 'RECONCILED', th: 'กระทบยอดแล้ว', en: 'Reconciled' },
    ];

    // Thai
    setLang('th');
    const rowsTh = statuses.map((s, idx) =>
      makeCashTransferRow({ id: `tx-th-${idx}`, status: s.code }),
    );
    await mount(rowsTh);

    for (let idx = 0; idx < statuses.length; idx++) {
      const tr = host.querySelector(`tr[data-transfer-id="tx-th-${idx}"]`)!;
      const cells = tr.querySelectorAll('td');
      expect(cells[2].textContent).toBe(statuses[idx].th);
    }

    // English
    dispose?.();
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('en');

    const rowsEn = statuses.map((s, idx) =>
      makeCashTransferRow({ id: `tx-en-${idx}`, status: s.code }),
    );
    await mount(rowsEn);

    for (let idx = 0; idx < statuses.length; idx++) {
      const tr = host.querySelector(`tr[data-transfer-id="tx-en-${idx}"]`)!;
      const cells = tr.querySelectorAll('td');
      expect(cells[2].textContent).toBe(statuses[idx].en);
    }
  });

  it('T3: unknown status WHATEVER shows WHATEVER', async () => {
    const row = makeCashTransferRow({
      id: 'tx-unk',
      status: 'WHATEVER',
    });
    await mount([row]);

    const tr = host.querySelector('tr[data-transfer-id="tx-unk"]')!;
    const cells = tr.querySelectorAll('td');
    expect(cells[2].textContent).toBe('WHATEVER');
  });

  it('T4: id d420fa35-8a1a-4b2c-9d3e-7f6a5b4c3d2e reads #d420fa35 with title full id', async () => {
    const fullId = 'd420fa35-8a1a-4b2c-9d3e-7f6a5b4c3d2e';
    const row = makeCashTransferRow({
      id: fullId,
      status: 'DRAFT',
    });
    await mount([row]);

    const tr = host.querySelector(`tr[data-transfer-id="${fullId}"]`)!;
    expect(tr).not.toBeNull();
    const cells = tr.querySelectorAll('td');
    expect(cells[0].textContent).toBe('#d420fa35');
    expect(cells[0].getAttribute('title')).toBe(fullId);
  });

  it('T5: open receive form heading shows #d420fa35 not full id, with title full id', async () => {
    const fullId = 'd420fa35-8a1a-4b2c-9d3e-7f6a5b4c3d2e';
    const row = makeCashTransferRow({
      id: fullId,
      status: 'DISPATCHED',
      fromBranchId: 'B99',
      toBranchId: BRANCH,
    });
    await mount([row]);

    const receiveBtn = host.querySelector<HTMLButtonElement>(
      `tr[data-transfer-id="${fullId}"] [data-testid="transfer-receive"]`,
    )!;
    expect(receiveBtn).not.toBeNull();
    receiveBtn.click();

    for (let i = 0; i < 5; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }

    const heading = host.querySelector('form[data-testid="transfer-receive-form"] h3')!;
    expect(heading).not.toBeNull();
    expect(heading.textContent).toContain('#d420fa35');
    expect(heading.textContent).not.toContain(fullId);
    expect(heading.getAttribute('title')).toBe(fullId);
  });

  it('T6: table header shows four column labels in TH', async () => {
    const row = makeCashTransferRow({
      id: 'tx-001',
      status: 'DRAFT',
    });
    await mount([row]);

    const ths = host.querySelectorAll('thead th');
    expect(ths.length).toBe(5);
    expect(ths[0].textContent).toBe('เลขที่');
    expect(ths[1].textContent).toBe('ประเภท');
    expect(ths[2].textContent).toBe('สถานะ');
    expect(ths[3].textContent).toBe('หมายเหตุ');
    expect(ths[4].textContent).toBe('');
  });
});
