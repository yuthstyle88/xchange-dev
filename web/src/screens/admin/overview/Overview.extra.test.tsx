// Tests for #461 Decisions:
// (2) Decision 2 (issuecomment-6067199516): the opener's name is displayed when the shift is open.
//     Since CA-10p (#2916 / #2914) the names come from status.openShifts; openedBy is gone (design §4.7).
// (5) Decision 5 (issuecomment-6067199516): CSV export walks admin_transactions for the period.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { meOf } from '../../../test-support/me';

// Amended for EX-9 (#1147) by the up-front test writer claude/709ec749: the export buttons show only to a `me`
// whose `permissions` hold `ReportsExport` (E-2: EXECUTIVE and FINANCE). `test-support/me.ts`'s table predates
// EX-1 #1070, so the permission is added here to the OWNER fixture (EXECUTIVE + ADMIN, #1030's shape).
const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as { permissions: string[] };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

const VIEW = '/api/v3/views/admin_overview';

function row(id: string) {
  return makeTransactionRow({
    id,
    date: '2026-10-06',
    created: '2026-10-06T03:00:00.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '30.0000',
    unitSize: '1',
    principal: '3000.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3000.00',
    method: 'Cash',
    customer: 'Customer',
    customerId: 'CUS-1',
    staff: 'owner',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'S-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
  });
}

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

let sent: Sent[];
let handlers: Record<string, (params: URLSearchParams) => { status: number; body: unknown }>;
let dispose: (() => void) | undefined;
let root: HTMLElement;

function ok(data: unknown) {
  return { status: 200, body: { version: 1, changed: true, data } };
}

function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  sent.push({ path: url.pathname, params: url.searchParams, method });
  const handler = handlers[url.pathname];
  const reply = handler
    ? handler(url.searchParams)
    : { status: 200, body: { version: 1, changed: false } };
  return Promise.resolve(
    new Response(JSON.stringify(reply.body), {
      status: reply.status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount(
  init: { branch?: string; period?: { from: string; to: string }; shown?: boolean } = {},
) {
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal(init.period ?? { from: '2026-10-05', to: '2026-10-06' });
  const [shown, setShown] = createSignal(init.shown ?? true);
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
  return { el: root, setBranch, setPeriod, setShown };
}

describe('V2-overview: extra tests for controller decisions (#461)', () => {
  let blobs: Blob[];
  let names: string[];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  beforeEach(() => {
    sent = [];
    blobs = [];
    names = [];
    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download);
    });

    handlers = {
      [VIEW]: () =>
        ok({
          period: { from: '2026-10-05', to: '2026-10-06' },
          totals: {
            volume: '3000.00',
            buyPrincipal: '3000.00',
            sellPrincipal: '0.00',
            buyCount: 1,
            sellCount: 0,
            buyShare: '100',
            sellShare: '0',
            received: '0.00',
            paid: '3000.00',
            net: '-3000.00',
            fees: '0.00',
            completedCount: 1,
            cancelledCount: 0,
            pendingCount: 0,
          },
          latest: [row('XC-001')],
          status: { shiftOpen: true, pendingCount: 0, openShifts: [{ shiftId: 'SH-1', tellerId: 'U-1', tellerName: 'Somchai', openedAt: '2026-10-06T01:00:00Z' }] },
        }),
      '/api/v3/views/admin_branches': () =>
        ok({ branches: [{ id: 'MAIN', name: 'Main Branch' }] }),
      '/api/v3/views/admin_transactions': () => {
        return ok([row('XC-001')]);
      },
      '/api/v3/views/me': () =>
        ok(EXPORTER_ME),
      [QUEUE_CARD_PATH]: () => ({ status: 200, body: queueCardUnchanged() }),
    };
    vi.stubGlobal('fetch', vi.fn(fakeFetch));
    setLang('th');
  });

  afterEach(async () => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
    setLang('th');
    await settle();
  });

  it('Decision 2: shows the open shift teller name from openShifts when the shift is open, and switches language', async () => {
    const m = mount();
    await settle();
    expect(m.el.textContent).toContain('ผู้เปิดกะ Somchai');

    setLang('en');
    await settle();
    expect(m.el.textContent).toContain('Opened by Somchai');
    expect(m.el.textContent).not.toContain('ผู้เปิดกะ');
  });

  it('Decision 2: does not show ผู้เปิดกะ when shift is closed', async () => {
    handlers[VIEW] = () =>
      ok({
        period: { from: '2026-10-05', to: '2026-10-06' },
        totals: {
          volume: '0.00',
          buyPrincipal: '0.00',
          sellPrincipal: '0.00',
          buyCount: 0,
          sellCount: 0,
          buyShare: '0',
          sellShare: '0',
          received: '0.00',
          paid: '0.00',
          net: '0.00',
          fees: '0.00',
          completedCount: 0,
          cancelledCount: 0,
          pendingCount: 0,
        },
        latest: [],
        status: { shiftOpen: false, pendingCount: 0 },
      });
    const m = mount();
    await settle();
    expect(m.el.textContent).not.toContain('ผู้เปิดกะ');
  });

  it('Decision 5: export CSV walks admin_transactions with from/to and downloads CSV file', async () => {
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();

    const exportBtn = [...m.el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === 'ส่งออกรายการ CSV',
    );
    expect(exportBtn).toBeDefined();

    exportBtn!.click();
    await settle();

    const txReads = sent.filter((s) => s.path === '/api/v3/views/admin_transactions');
    expect(txReads.length).toBeGreaterThanOrEqual(1);
    expect(txReads[0]!.params.get('branch')).toBe('MAIN');
    expect(txReads[0]!.params.get('from')).toBe('2026-10-05');
    expect(txReads[0]!.params.get('to')).toBe('2026-10-06');

    expect(blobs.length).toBe(1);
    expect(names.length).toBe(1);
    expect(names[0]).toContain('XCHANGE-LIVE-');
  });

  it('Decision 5: export CSV shows toast when no transactions to export', async () => {
    handlers['/api/v3/views/admin_transactions'] = () => ok([]);
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();

    const exportBtn = [...m.el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === 'ส่งออกรายการ CSV',
    );
    exportBtn!.click();
    await settle();

    expect(blobs.length).toBe(0);
    const toastEl = m.el.querySelector('.toast');
    expect(toastEl?.textContent).toContain('ไม่มีรายการให้ส่งออก');
  });

  it('activity bars switch to Buy and Sell under setLang(en)', async () => {
    const m = mount();
    await settle();
    const activityPanel = m.el.querySelector('.activity-panel');
    expect(activityPanel).toBeDefined();
    expect(activityPanel?.textContent).toContain('ร้านรับซื้อ');
    expect(activityPanel?.textContent).toContain('ร้านขาย');

    setLang('en');
    await settle();
    expect(activityPanel?.textContent).toContain('Buy');
    expect(activityPanel?.textContent).toContain('Sell');
    expect(activityPanel?.textContent).not.toContain('ร้านรับซื้อ');
    expect(activityPanel?.textContent).not.toContain('ร้านขาย');
  });
});
