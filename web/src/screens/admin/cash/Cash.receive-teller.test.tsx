// Receive gate on #cash (#564 issuecomment-6604004214): the server lets only the teller of a transfer's open
// `toShiftId` shift receive it (ruling Q1, api/crates/xc-cash/src/application/transfer/transfer_flow.rs:736-750);
// anyone else gets 400 "only the shift's teller may receive into drawer". #cash must not draw what the server refuses.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests (`[data-section="transfers"] button[data-action="receive"][data-id=…]`, DISPATCHED rows):
// - R1: a row with `toShiftId` whose open shift's teller (`admin_cash.openShifts[].tellerId`) is not the viewer
//   (`me.id`): receive is not drawn.
// - R2: the viewer is that shift's teller: receive is drawn, as today.
// - R3: `toShiftId` names no open shift: receive is not drawn, for anyone.
// - R4: a row without `toShiftId` (BRANCH here) keeps today's receive.
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

let meId = 'u-admin';
const ME = () => makeMe({
  id: meId,
  name: 'ผู้จัดการทดสอบ',
  roles: [],
  permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
  branch: 'MAIN',
  branchName: 'สำนักงานใหญ่',
  branches: [makeBranchRef({ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' })],
});

function transfer(id: string, kind: string, toShiftId: string | null, toBranchId: string | null, fromBranchId = 'MAIN') {
  return makeCashTransferRow({
    id,
    kind,
    status: 'DISPATCHED',
    approvalRequestId: `ap-${id}`,
    approvalStatus: 'APPROVED',
    fromBranchId,
    toBranchId,
    fromShiftId: null,
    toShiftId,
    createdBy: 'EMP-002',
    createdAt: '2026-11-03T02:00:00Z',
    note: `note ${id}`,
    lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 11, qtySent: '3', qtyReceived: null })],
  });
}

// S-0001 is open with teller u-1 (admin_cash.openShifts below); S-0099 is not an open shift.
const TRANSFERS = [
  transfer('tr-drawer', 'SAFE_TO_DRAWER', 'S-0001', null),
  transfer('tr-closed', 'SAFE_TO_DRAWER', 'S-0099', null),
  transfer('tr-branch', 'BRANCH', null, 'MAIN', 'OTHER'),
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
      return ok(ME());
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
  await vi.waitFor(() => expect(rowOf('tr-drawer')).not.toBeNull());
  return root;
}

/** The transfers section's row of a transfer, found by its note cell. */
function rowOf(id: string): HTMLTableRowElement | null {
  const section = root.querySelector<HTMLElement>('[data-section="transfers"]');
  const cell = [...(section?.querySelectorAll('td') ?? [])].find((td) => td.textContent?.trim() === `note ${id}`);
  return (cell?.closest('tr') as HTMLTableRowElement | null) ?? null;
}

const receiveOf = (id: string) => root.querySelector(`[data-section="transfers"] button[data-action="receive"][data-id="${id}"]`);

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
  meId = 'u-admin';
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

describe('#cash draws receive only for the teller of the transfer\'s open shift', () => {
  it('R1 a viewer who is not the shift\'s teller gets no receive on a SAFE_TO_DRAWER row', async () => {
    await mount();
    expect(receiveOf('tr-drawer')).toBeNull();
  });

  it('R2 the shift\'s own teller gets receive', async () => {
    meId = 'u-1';
    await mount();
    expect(receiveOf('tr-drawer')).not.toBeNull();
  });

  it('R3 no open shift for toShiftId: no receive for anyone', async () => {
    await mount();
    expect(receiveOf('tr-closed'), 'not the teller').toBeNull();
    dispose?.();
    root.remove();
    meId = 'u-1';
    await mount();
    expect(receiveOf('tr-closed'), 'the teller of another shift').toBeNull();
  });

  it('R4 a row without toShiftId keeps receive', async () => {
    await mount();
    expect(receiveOf('tr-branch')).not.toBeNull();
  });
});
