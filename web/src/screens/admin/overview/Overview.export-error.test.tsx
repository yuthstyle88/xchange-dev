import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Overview } from './Overview';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

const EXPORTER_ME = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as {
    permissions: string[];
    [key: string]: unknown;
  };
  return { ...m, permissions: [...(m.permissions ?? []), 'ReportsExport'].sort() };
})();

const VIEW_OVERVIEW = '/api/v3/views/admin_overview';
const VIEW_TRANSACTIONS = '/api/v3/views/admin_transactions';
const VIEW_BRANCHES = '/api/v3/views/admin_branches';

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

const DEFAULT_OVERVIEW_DATA = {
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
};

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

let sent: Sent[];
let handlers: Record<string, (params: URLSearchParams) => { status: number; body: unknown } | Promise<Response>>;
let dispose: (() => void) | undefined;
let root: HTMLElement;
let blobs: Blob[];
let names: string[];

const originalCreate = URL.createObjectURL;
const originalRevoke = URL.revokeObjectURL;

function ok(data: unknown) {
  return { status: 200, body: { version: 1, changed: true, data } };
}

function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  sent.push({ path: url.pathname, params: url.searchParams, method });
  const handler = handlers[url.pathname];
  if (!handler) {
    return Promise.resolve(
      new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
  }
  const res = handler(url.searchParams);
  if (res instanceof Promise) {
    return res;
  }
  return Promise.resolve(
    new Response(JSON.stringify(res.body), {
      status: res.status,
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

  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Overview branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return { el: root, setBranch, setPeriod, setShown };
}

describe('#2722: Admin Overview export failure error isolation', () => {
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
      [VIEW_OVERVIEW]: () => ok(DEFAULT_OVERVIEW_DATA),
      [VIEW_BRANCHES]: () => ok({ branches: [{ id: 'MAIN', name: 'Main Branch' }, { id: 'BRANCH_2', name: 'Second Branch' }] }),
      [VIEW_TRANSACTIONS]: () => ok([row('XC-001')]),
      '/api/v3/views/me': () => ok(EXPORTER_ME),
      [QUEUE_CARD_PATH]: () => ({ status: 200, body: queueCardUnchanged() }),
    };

    sharedSession().updateMe?.(EXPORTER_ME as any);
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

  it('O1: Overview loaded (shift open, pending 3), then Export CSV fails (500) -> error-banner shows worded error, strip shows shift status, pending reads 3, Check pending enabled', async () => {
    handlers[VIEW_OVERVIEW] = () =>
      ok({
        ...DEFAULT_OVERVIEW_DATA,
        status: { shiftOpen: true, pendingCount: 3, openShifts: [{ shiftId: 'SH-1', tellerId: 'U-1', tellerName: 'Somchai', openedAt: '2026-10-06T01:00:00Z' }] },
      });
    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'Internal server error' } },
    });

    const m = mount();
    await settle();

    // Verify initial load succeeded
    const strip = m.el.querySelector('.status-strip')!;
    expect(strip.textContent).toContain('กะเปิดอยู่');
    expect(strip.textContent).toContain('3 รายการ');
    const checkPendingBtn = strip.querySelector<HTMLButtonElement>('button.secondary')!;
    expect(checkPendingBtn.disabled).toBe(false);
    expect(m.el.querySelector('.error-banner')).toBeNull();

    // Click Export CSV
    const exportBtn = m.el.querySelector<HTMLButtonElement>('button.btn-export')!;
    exportBtn.click();
    await settle();

    // Required result:
    // .error-banner shows the worded error
    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toBeTruthy();

    // the strip shows the shift status (not statusError)
    expect(strip.textContent).toContain('กะเปิดอยู่');
    expect(strip.textContent).not.toContain('ยังโหลดสถานะไม่ได้');

    // the pending count reads 3
    expect(strip.textContent).toContain('3 รายการ');

    // "Check pending" is enabled
    expect(checkPendingBtn.disabled).toBe(false);
  });

  it('O2: Export fails with outOfScope -> .error-banner shows out-of-scope sentence', async () => {
    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 403,
      body: { error: { code: 'outOfScope', message: 'Out of scope' } },
    });

    const m = mount();
    await settle();

    const exportBtn = m.el.querySelector<HTMLButtonElement>('button.btn-export')!;
    exportBtn.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toBe('สาขานี้อยู่นอกขอบเขตของบัญชีนี้');
  });

  it('O3: The overview read fails -> Banner shows the error, strip says statusError, count shows "—", Check pending disabled', async () => {
    handlers[VIEW_OVERVIEW] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'Internal server error' } },
    });

    const m = mount();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();

    const strip = m.el.querySelector('.status-strip')!;
    expect(strip.textContent).toContain('ยังโหลดสถานะไม่ได้');
    expect(strip.textContent).toContain('—');

    const checkPendingBtn = strip.querySelector<HTMLButtonElement>('button.secondary')!;
    expect(checkPendingBtn.disabled).toBe(true);
  });

  it('O4: Export fails, then Export again and it succeeds -> No banner', async () => {
    let callCount = 0;
    handlers[VIEW_TRANSACTIONS] = () => {
      callCount++;
      if (callCount === 1) {
        return {
          status: 500,
          body: { error: { code: 'serverError', message: 'Internal server error' } },
        };
      }
      return ok([row('XC-001')]);
    };

    const m = mount();
    await settle();

    const exportBtn = m.el.querySelector<HTMLButtonElement>('button.btn-export')!;
    // 1st export: fails
    exportBtn.click();
    await settle();
    expect(m.el.querySelector('.error-banner')).not.toBeNull();

    // 2nd export: succeeds
    exportBtn.click();
    await settle();
    expect(m.el.querySelector('.error-banner')).toBeNull();
  });

  it('O5: Export fails, then the branch changes -> No banner from the export', async () => {
    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'Internal server error' } },
    });

    const m = mount({ branch: 'MAIN' });
    await settle();

    const exportBtn = m.el.querySelector<HTMLButtonElement>('button.btn-export')!;
    exportBtn.click();
    await settle();

    expect(m.el.querySelector('.error-banner')).not.toBeNull();

    // Change branch
    m.setBranch('BRANCH_2');
    await settle();

    expect(m.el.querySelector('.error-banner')).toBeNull();
  });

  it("O6: The overview read fails, then Export fails -> Banner shows error, strip still says statusError (the read's failure wins)", async () => {
    handlers[VIEW_OVERVIEW] = () => ({
      status: 500,
      body: { error: { code: 'overviewError', message: 'Overview error' } },
    });
    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 500,
      body: { error: { code: 'exportError', message: 'Export error' } },
    });

    const m = mount();
    await settle();

    // Overview read failed
    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    const strip = m.el.querySelector('.status-strip')!;
    expect(strip.textContent).toContain('ยังโหลดสถานะไม่ได้');

    // Trigger export failure
    const exportBtn = m.el.querySelector<HTMLButtonElement>('button.btn-export')!;
    exportBtn.click();
    await settle();

    // Banner still shows an error
    expect(m.el.querySelector('.error-banner')).not.toBeNull();
    // Strip still says statusError
    expect(strip.textContent).toContain('ยังโหลดสถานะไม่ได้');
    const checkPendingBtn = strip.querySelector<HTMLButtonElement>('button.secondary')!;
    expect(checkPendingBtn.disabled).toBe(true);
  });
});
