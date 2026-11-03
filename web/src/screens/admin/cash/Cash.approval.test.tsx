// F5 FE (#564 finding F5; server side PR 6043 / #6037, design docs/design/blueprint/cash-shift-transfer.md §4.10):
// on the admin cash screen a submitted transfer says it waits for approval, and a rejected one says it was not approved.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests (`[data-section="transfers"]`, the status pill of each row):
// - A1: `status: DRAFT` + `approvalStatus: PENDING` ⇒ "รออนุมัติ" (owner-approved, #564 issuecomment-6596705394).
// - A2: `DRAFT` + `REJECTED` ⇒ "ไม่อนุมัติ" (owner-approved).
// - A3: `DRAFT` + `null` ⇒ "ร่าง", as today; any other status as today (`APPROVED` + `APPROVED` ⇒ "อนุมัติแล้ว").
// - A4: the submit action (`[data-action="submit"]`) is not drawn for a draft that is waiting for approval; a rejected
//   draft keeps it (re-submitting raises a new request, design §4.10 T-5), and so does a draft never submitted.
// - A5: in English, "Awaiting approval" and "Not approved".
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import {
  makeBranchRef,
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeMe,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

const ME = makeMe({
  id: 'u-admin',
  name: 'ผู้จัดการทดสอบ',
  roles: [],
  permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
  branch: 'MAIN',
  branchName: 'สำนักงานใหญ่',
  branches: [makeBranchRef({ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' })],
});

function transfer(id: string, status: string, approvalStatus: string | null) {
  return makeCashTransferRow({
    id,
    kind: 'SAFE_TO_DRAWER',
    status,
    approvalRequestId: approvalStatus ? `ap-${id}` : null,
    approvalStatus,
    fromBranchId: 'MAIN',
    toBranchId: null,
    fromShiftId: null,
    toShiftId: 'S-0001',
    createdBy: 'EMP-002',
    createdAt: '2026-11-03T02:00:00Z',
    note: `note ${id}`,
    lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 11, qtySent: '3', qtyReceived: null })],
  });
}

const TRANSFERS = [
  transfer('tr-pending', 'DRAFT', 'PENDING'),
  transfer('tr-rejected', 'DRAFT', 'REJECTED'),
  transfer('tr-draft', 'DRAFT', null),
  transfer('tr-approved', 'APPROVED', 'APPROVED'),
];

const ADMIN_CASH = {
  openShifts: [{ shiftId: 'S-0001', tellerId: 'u-1', tellerName: 'EMP-007', openedAt: '2026-11-03T01:30:00.000Z' }],
  stock: [{ code: 'THB', opening: '100.00', net: '0.00', expected: '100.00' }],
  stats: { opening: '100.00', incoming: '0.00', outgoing: '0.00', remaining: '100.00' },
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB'],
  lastCounted: null,
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fakeFetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const u = new URL(raw, 'http://xc.test');
  const method = (init?.method ?? 'GET').toUpperCase();
  const ok = (data: unknown) => json(200, { version: 1, changed: true, data });
  if (method !== 'GET') return json(200, { id: 'x', status: 'OK' });
  switch (u.pathname) {
    case '/api/v3/views/me':
      return ok(ME);
    case QUEUE_CARD_PATH:
      return json(200, queueCardUnchanged());
    case '/api/v3/views/admin_branches':
      return ok({ branches: [{ id: 'MAIN', name: 'สำนักงานใหญ่' }] });
    case '/api/v3/views/admin_cash':
      return ok(ADMIN_CASH);
    case '/api/v3/views/admin_cash_movements':
      return ok([]);
    case '/api/v3/views/admin_shifts':
      return ok([]);
    case '/api/v3/views/cash_counts':
      return ok([]);
    case '/api/v3/views/cash_transfers':
      return ok(TRANSFERS);
    case '/api/v3/views/denominations':
      return ok({ rows: [] });
    case '/api/v3/views/transfer_branches':
      return ok({ rows: [] });
    default:
      return json(200, { version: 1, changed: true, data: null });
  }
};

let dispose: (() => void) | undefined;
let root: HTMLElement;

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<HTMLElement> {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  await vi.waitFor(() => expect(rowOf('tr-pending')).not.toBeNull());
  return root;
}

/** The transfers section's row of a transfer, found by its note cell. */
function rowOf(id: string): HTMLTableRowElement | null {
  const section = root.querySelector<HTMLElement>('[data-section="transfers"]');
  const cell = [...(section?.querySelectorAll('td') ?? [])].find((td) => td.textContent?.trim() === `note ${id}`);
  return (cell?.closest('tr') as HTMLTableRowElement | null) ?? null;
}

const pillOf = (id: string) => (rowOf(id)?.querySelector('.pill')?.textContent ?? '').trim();
const submitOf = (id: string) => root.querySelector(`[data-section="transfers"] button[data-action="submit"][data-id="${id}"]`);

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  vi.stubGlobal('matchMedia', (query: string) => ({
    media: query,
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
  }));
  location.hash = '#cash';
  setLang('th');
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
  location.hash = '';
  setLang('th');
});

describe('F5 the admin cash screen names the approval state of a draft transfer', () => {
  it('A1 a draft waiting for approval reads รออนุมัติ', async () => {
    await mount();
    expect(pillOf('tr-pending')).toBe('รออนุมัติ');
  });

  it('A2 a draft whose approval was rejected reads ไม่อนุมัติ', async () => {
    await mount();
    expect(pillOf('tr-rejected')).toBe('ไม่อนุมัติ');
  });

  it('A3 a draft never submitted reads ร่าง and an approved transfer reads as today', async () => {
    await mount();
    expect(pillOf('tr-draft')).toBe('ร่าง');
    expect(pillOf('tr-approved')).toBe('อนุมัติแล้ว');
  });

  it('A4 no submit for a draft waiting for approval; a rejected draft and a fresh draft keep it', async () => {
    await mount();
    expect(submitOf('tr-pending'), 'waiting: no submit').toBeNull();
    expect(submitOf('tr-rejected'), 'rejected: may re-submit').not.toBeNull();
    expect(submitOf('tr-draft'), 'never submitted: submit').not.toBeNull();
  });

  it('A5 in English: Awaiting approval, Not approved', async () => {
    setLang('en');
    await mount();
    expect(pillOf('tr-pending')).toBe('Awaiting approval');
    expect(pillOf('tr-rejected')).toBe('Not approved');
    expect(pillOf('tr-draft')).toBe('Draft');
  });
});
