import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { meOf } from '../../../test-support/me';

const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as { permissions: string[] };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

const VIEW = '/api/v3/views/admin_overview';

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

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

describe('#2631 Overview export CSV in-flight suppression', () => {
  let sent: Sent[];
  let blobs: Blob[];
  let names: string[];
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  let pendingTxResolvers: Array<() => void>;
  let holdTransactions: boolean;
  let failTransactions: boolean;

  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  function ok(data: unknown) {
    return { status: 200, body: { version: 1, changed: true, data } };
  }

  function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
    const url = new URL(input, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    sent.push({ path: url.pathname, params: url.searchParams, method });

    if (url.pathname === '/api/v3/views/admin_transactions') {
      if (holdTransactions) {
        return new Promise<Response>((resolve) => {
          pendingTxResolvers.push(() => {
            if (failTransactions) {
              resolve(
                new Response(JSON.stringify({ error: { code: 'server_error', message: 'DB down' } }), {
                  status: 500,
                  headers: { 'content-type': 'application/json' },
                }),
              );
            } else {
              resolve(
                new Response(JSON.stringify(ok([row('XC-001')]).body), {
                  status: 200,
                  headers: { 'content-type': 'application/json' },
                }),
              );
            }
          });
        });
      }
      if (failTransactions) {
        return Promise.resolve(
          new Response(JSON.stringify({ error: { code: 'server_error', message: 'DB down' } }), {
            status: 500,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }
      return Promise.resolve(
        new Response(JSON.stringify(ok([row('XC-001')]).body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      );
    }

    if (url.pathname === VIEW) {
      return Promise.resolve(
        new Response(
          JSON.stringify(
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
            }).body,
          ),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    }

    if (url.pathname === '/api/v3/views/admin_branches') {
      return Promise.resolve(
        new Response(JSON.stringify(ok({ branches: [{ id: 'MAIN', name: 'Main Branch' }] }).body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      );
    }

    if (url.pathname === '/api/v3/views/me') {
      return Promise.resolve(
        new Response(JSON.stringify(ok(EXPORTER_ME).body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      );
    }

    if (url.pathname === QUEUE_CARD_PATH) {
      return Promise.resolve(
        new Response(JSON.stringify(queueCardUnchanged()), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      );
    }

    return Promise.resolve(
      new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
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

  function getExportButton(el: HTMLElement): HTMLButtonElement {
    const btn = [...el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === 'ส่งออกรายการ CSV',
    );
    expect(btn, 'export button should exist').toBeDefined();
    return btn!;
  }

  beforeEach(() => {
    sent = [];
    blobs = [];
    names = [];
    pendingTxResolvers = [];
    holdTransactions = false;
    failTransactions = false;

    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download);
    });

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
    await settle();
  });

  it('two clicks before the walk finishes trigger only one walk and one download', async () => {
    holdTransactions = true;
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();

    const exportBtn = getExportButton(m.el);
    expect(exportBtn.disabled).toBe(false);

    // First click starts export
    exportBtn.click();
    expect(exportBtn.disabled, 'button is disabled while export is in flight').toBe(true);

    // Second click while walk is in flight
    exportBtn.click();

    // Release the held walk
    expect(pendingTxResolvers.length).toBe(1);
    pendingTxResolvers[0]!();
    await settle();

    // After completion
    expect(exportBtn.disabled, 'button is enabled after export completes').toBe(false);
    const txReads = sent.filter((s) => s.path === '/api/v3/views/admin_transactions');
    expect(txReads.length, 'only one walk triggered').toBe(1);
    expect(blobs.length, 'only one download triggered').toBe(1);
  });

  it('when export fails, the button becomes usable again and error is shown', async () => {
    holdTransactions = true;
    failTransactions = true;
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();

    const exportBtn = getExportButton(m.el);
    expect(exportBtn.disabled).toBe(false);

    // First click starts export
    exportBtn.click();
    expect(exportBtn.disabled, 'button is disabled while export is in flight').toBe(true);

    // Second click while in flight
    exportBtn.click();

    expect(pendingTxResolvers.length).toBe(1);
    pendingTxResolvers[0]!();
    await settle();

    // Button is re-enabled and error banner is shown
    expect(exportBtn.disabled, 'button is enabled after failure').toBe(false);
    expect(blobs.length).toBe(0);
    const errorBanner = m.el.querySelector('.error-banner');
    expect(errorBanner).not.toBeNull();
  });

  it('when export finishes, clicking again runs a second export', async () => {
    holdTransactions = true;
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();

    const exportBtn = getExportButton(m.el);
    expect(exportBtn.disabled).toBe(false);

    // First export
    exportBtn.click();
    expect(exportBtn.disabled, 'button is disabled during first export').toBe(true);
    expect(pendingTxResolvers.length).toBe(1);
    pendingTxResolvers[0]!();
    await settle();

    expect(exportBtn.disabled, 'button is enabled after first export').toBe(false);
    expect(blobs.length).toBe(1);

    // Reset held promise for second export
    pendingTxResolvers = [];

    // Second export
    exportBtn.click();
    expect(exportBtn.disabled, 'button is disabled during second export').toBe(true);
    expect(pendingTxResolvers.length).toBe(1);
    pendingTxResolvers[0]!();
    await settle();

    expect(exportBtn.disabled, 'button is enabled after second export').toBe(false);
    expect(blobs.length).toBe(2);
  });
});
