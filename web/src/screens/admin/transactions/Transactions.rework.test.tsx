import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import { Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Transactions } from './Transactions';

const EXPORTER_ME: unknown = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'B1', name: 'Owner' }) as unknown as {
    permissions: string[];
  };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

function dummyTx() {
  return makeTransactionRow({
    id: 'XC-20261008-000001',
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    unitSize: '1',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3209.99',
    method: 'Cash',
    customer: 'Customer 1',
    customerId: 'CUS-1',
    staff: 'front',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
  });
}

describe('Transactions rework tests (#1701): worded error on export failure', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let exportFailure: 'network' | { status: number; body: unknown } | null = null;
  let branchesFailure: 'network' | { status: number; body: unknown } | null = null;
  let downloadedFiles: string[] = [];

  beforeEach(() => {
    setLang('th');
    exportFailure = null;
    branchesFailure = null;
    downloadedFiles = [];

    sharedSession().updateMe(EXPORTER_ME as any);

    // Track CSV downloads
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      if (this.download) {
        downloadedFiles.push(this.download);
      }
    });

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);

        if (url.includes('/api/v3/views/admin_transactions')) {
          const u = new URL(url, 'http://localhost');
          const isExport = u.searchParams.get('purpose') === 'export';

          if (isExport && exportFailure) {
            if (exportFailure === 'network') {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(JSON.stringify(exportFailure.body), {
                status: exportFailure.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }

          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: [dummyTx()],
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.includes('/api/v3/views/admin_branches')) {
          if (branchesFailure) {
            if (branchesFailure === 'network') {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(JSON.stringify(branchesFailure.body), {
                status: branchesFailure.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { branches: [{ id: 'B1', name: 'Silom #2' }] },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.includes('/api/v3/views/me')) {
          return Promise.resolve(
            new Response(JSON.stringify({ version: 1, changed: true, data: EXPORTER_ME }), {
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
      }),
    );

    root = document.createElement('div');
    document.body.appendChild(root);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('B1');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <Transactions branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    return root;
  }

  function getExportButton(): HTMLButtonElement {
    const buttons = [...root.querySelectorAll('button')];
    const btn = buttons.find(
      (b) => b.textContent?.includes('ส่งออก') || b.textContent?.includes('Export CSV'),
    );
    expect(btn, 'export button should exist').toBeDefined();
    return btn!;
  }

  it('1. network failure on export shows worded error, never raw err.message', async () => {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('tbody tr')).not.toBeNull();
    });

    exportFailure = 'network';
    getExportButton().click();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(sharedTh['error.network']);
    expect(alertText).not.toContain('Failed to fetch');
    expect(downloadedFiles).toHaveLength(0);
  });

  it('2. network failure on export in English shows English worded error', async () => {
    setLang('en');
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('tbody tr')).not.toBeNull();
    });

    exportFailure = 'network';
    getExportButton().click();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(sharedEn['error.network']);
    expect(alertText).not.toContain('Failed to fetch');
    expect(downloadedFiles).toHaveLength(0);
  });

  it('3. refusal with known code on export shows worded error for that code', async () => {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('tbody tr')).not.toBeNull();
    });

    exportFailure = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowed',
          message: 'RAW-ROLE-NOT-ALLOWED-FORBIDDEN',
        },
      },
    };
    getExportButton().click();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(sharedTh['error.roleNotAllowed']);
    expect(alertText).not.toContain('RAW-ROLE-NOT-ALLOWED-FORBIDDEN');
    expect(downloadedFiles).toHaveLength(0);
  });

  it('4. refusal with unknown code on export shows generic worded message, never raw code or message', async () => {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('tbody tr')).not.toBeNull();
    });

    exportFailure = {
      status: 500,
      body: {
        error: {
          code: 'unknownCode999',
          message: 'RAW-UNKNOWN-INTERNAL-CRASH',
        },
      },
    };
    getExportButton().click();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(sharedTh['error.serverError']);
    expect(alertText).not.toContain('RAW-UNKNOWN-INTERNAL-CRASH');
    expect(alertText).not.toContain('unknownCode999');
    expect(downloadedFiles).toHaveLength(0);
  });

  it('5. admin_branches API failure during export does not fall back silently; shows worded error and downloads no file', async () => {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('tbody tr')).not.toBeNull();
    });

    branchesFailure = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'admin branches failed',
        },
      },
    };
    getExportButton().click();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(sharedTh['error.serverError']);
    expect(downloadedFiles).toHaveLength(0);
  });
});
