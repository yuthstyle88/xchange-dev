// Issue #1700: Admin overview export fails silently on a network error.
// Evidence table §2:
// web/src/screens/admin/overview/Overview.tsx: export catch handles ApiError only -> worded error for any failure.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, setLang } from '../../../shared/i18n';
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

function row(id: string): TransactionRow {
  return {
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
  };
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

describe('Overview rework tests (#1700): worded errors on export failure', () => {
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
      [VIEW_BRANCHES]: () => ok({ branches: [{ id: 'MAIN', name: 'Main Branch' }] }),
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

  it('export failure: network error (TypeError: Failed to fetch) displays worded error and preserves data', async () => {
    const m = mount();
    await settle();

    // Verify initial load succeeded and data is visible
    expect(m.el.querySelector('.error-banner')).toBeNull();
    expect(m.el.textContent).toContain('3,000.00');

    // Simulate network error on export call
    handlers[VIEW_TRANSACTIONS] = () => Promise.reject(new TypeError('Failed to fetch'));

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    expect(exportBtn).toBeDefined();
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    // Must show worded message in current language (th), not empty and not raw message
    expect(banner?.textContent).toBe(errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่'));
    expect(banner?.textContent).not.toContain('Failed to fetch');
    // Stale data remains visible
    expect(m.el.textContent).toContain('3,000.00');
    // No file was downloaded
    expect(blobs.length).toBe(0);
  });

  it('export failure: 500 server error displays worded error', async () => {
    const m = mount();
    await settle();

    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'Internal Server Error' } },
    });

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(banner?.textContent).not.toContain('Internal Server Error');
  });

  it('export failure: refusal with known code displays translated worded error', async () => {
    const m = mount();
    await settle();

    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 403,
      body: { error: { code: 'roleNotAllowedHere', message: 'Not permitted' } },
    });

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe(errorText('roleNotAllowedHere', ''));
    expect(banner?.textContent).toBe('บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้');
    expect(banner?.textContent).not.toContain('roleNotAllowedHere');
    expect(banner?.textContent).not.toContain('Not permitted');
  });

  it('export failure: refusal with unknown code displays server sentence, never raw code', async () => {
    const m = mount();
    await settle();

    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 400,
      body: { error: { code: 'unknownRefusal999', message: 'Private stack details here' } },
    });

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe('Private stack details here');
    expect(banner?.textContent).not.toContain('unknownRefusal999');
  });

  it('export failure under setLang("en") displays English worded error', async () => {
    setLang('en');
    const m = mount();
    await settle();

    handlers[VIEW_TRANSACTIONS] = () => Promise.reject(new TypeError('Failed to fetch'));

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe('Cannot reach the server. Please try again.');
  });

  it('export failure: admin_branches failure displays worded error and does not download CSV', async () => {
    const m = mount();
    await settle();

    // Transactions succeed, but branches read fails
    handlers[VIEW_BRANCHES] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'Branches unavailable' } },
    });

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(blobs.length).toBe(0);
  });

  it('export failure: outOfScope refusal shows worded message in TH and EN, never wire text', async () => {
    // 1. In Thai
    const m = mount();
    await settle();

    handlers[VIEW_TRANSACTIONS] = () => ({
      status: 403,
      body: { error: { code: 'outOfScope', message: 'raw scope text' } },
    });

    const exportBtn = m.el.querySelector<HTMLButtonElement>('.btn-export');
    exportBtn!.click();
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe('สาขานี้อยู่นอกขอบเขตของบัญชีนี้');
    expect(banner?.textContent).not.toContain('raw scope text');

    // 2. In English
    setLang('en');
    exportBtn!.click();
    await settle();

    expect(banner?.textContent).toBe("This branch is outside this account's scope");
    expect(banner?.textContent).not.toContain('raw scope text');
  });
});
